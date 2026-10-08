import asyncio
import random
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from itertools import chain
from types import MappingProxyType
from typing import Final

from fastapi import HTTPException

from litellm_lens.access_repository import AccessRepository
from litellm_lens.clickhouse_state import Change, ClickHouseState, Snapshot, StorageFailure
from litellm_lens.ingestion import IngestionKey
from litellm_lens.models import (
    Job,
    Lens,
    Progress,
    Record,
    Review,
    ReviewVersion,
    Scope,
    TraceFindingCount,
    TraceIdentity,
    Worker,
)
from litellm_lens.persistence import document, record_key, record_pages, require_storage, utc_time
from litellm_lens.repository_queries import FindingRun, InvestigationQueries
from litellm_lens.reviews import criteria_key
from litellm_lens.state import apply_progress, current_job, due_at, replace_job

UPDATE_ATTEMPTS: Final = 40
UPDATE_BACKOFF_SECONDS: Final = 0.02


class StoredLens(Record):
    lens: Lens
    due_at: datetime | None


class ArchivedJob(Record):
    lens_id: str
    parent_key: str
    archived_version: int
    job: Job


@dataclass(frozen=True, slots=True)
class DueLens:
    lens: Lens
    due_at: datetime


def stored(lens: Lens) -> StoredLens:
    scheduled: Final = due_at(lens)
    return StoredLens(lens=lens, due_at=utc_time(scheduled) if scheduled is not None else None)


def review_key(lens_id: str, job: Job, execution_id: str) -> str:
    return record_key("review", lens_id, criteria_key(job.settings), execution_id)


class LensRepository:
    def __init__(self, state: ClickHouseState, sleep: Callable[[float], Awaitable[None]] = asyncio.sleep) -> None:
        self.state: Final = state
        self.sleep: Final = sleep
        self.access: Final = AccessRepository(state)
        self.queries: Final = InvestigationQueries(state)

    async def ingestion_keys(self) -> tuple[IngestionKey, ...]:
        return await self.access.ingestion_keys()

    async def save_ingestion_key(self, key: IngestionKey) -> None:
        await self.access.save_ingestion_key(key)

    async def revoke_ingestion_key(self, key_id: str) -> None:
        await self.access.revoke_ingestion_key(key_id)

    async def finding_runs(self, lens_id: str, finding_ids: tuple[str, ...]) -> tuple[FindingRun, ...]:
        if not finding_ids:
            return ()
        snapshot: Final = require_storage(await self.state.read(record_key("lens", lens_id)))
        if snapshot.value is None:
            return ()
        return await self.queries.finding_runs(snapshot, StoredLens.model_validate(snapshot.value).lens, finding_ids)

    async def reviews(self, lens_id: str, job: Job) -> tuple[Review, ...]:
        keys: Final = tuple(review_key(lens_id, job, e.id) for e in job.sample.executions) if job.sample else ()
        records: Final = require_storage(await self.state.read_many(keys))
        return tuple(Review.model_validate(record.value) for record in records if record.value is not None)

    async def update_locked(self, lens_id: str, transform: Callable[[Lens], Lens]) -> Lens | None:
        return await self.update(lens_id, transform)

    async def progress(self, lens_id: str, assigned: Job, body: Progress) -> Lens | None:
        def renew(lens: Lens) -> Lens:
            job: Final = current_job(lens)
            now: Final = datetime.now(UTC)
            if (
                job is None
                or job.id != assigned.id
                or job.worker_id != assigned.worker_id
                or job.attempts != assigned.attempts
                or job.status != "running"
                or job.lease_until is None
                or job.lease_until <= now
            ):
                raise HTTPException(409, "This worker no longer owns the job")
            return replace_job(lens, apply_progress(job, body, now))

        review: Final = body.review
        checkpoint: Final = (
            (review_key(lens_id, assigned, review.execution_id), review)
            if review is not None and not review.reused and review.extraction is not None and review.content_version
            else None
        )
        return await self._update(lens_id, renew, UPDATE_ATTEMPTS, False, checkpoint)

    async def complete_reviews(self, lens_id: str, job: Job, versions: tuple[ReviewVersion, ...]) -> None:
        expected: Final = MappingProxyType(
            {review_key(lens_id, job, v.execution_id): v.content_version for v in versions}
        )
        for attempt in range(UPDATE_ATTEMPTS):
            match await self._complete_reviews(expected):
                case None:
                    return
                case StorageFailure(kind="conflict"):
                    await self._backoff(attempt)
                case StorageFailure() as failure:
                    require_storage(failure)
        require_storage(StorageFailure("conflict"))

    async def _complete_reviews(self, expected: Mapping[str, str]) -> StorageFailure | None:
        previous: Final = require_storage(await self.state.read_many(tuple(expected)))
        matching: Final = tuple(
            (record, Review.model_validate(record.value)) for record in previous if record.value is not None
        )
        changes: Final = tuple(
            Change(record, document(review.model_copy(update={"consolidated": True})))
            for record, review in matching
            if review.content_version == expected[record.key] and not review.consolidated
        )
        return await self.state.commit(changes) if changes else None

    async def lenses(self) -> tuple[Lens, ...]:
        pages: Final = tuple([page async for page in record_pages(self.state, "lens/")])
        records: Final = chain.from_iterable(pages)
        lenses: Final = tuple(
            StoredLens.model_validate(record.value).lens for record in records if record.value is not None
        )
        return tuple(sorted(lenses, key=lambda lens: lens.id))

    async def due(self, scope: Scope, now: datetime, limit: int, after: DueLens | None = None) -> tuple[DueLens, ...]:
        heads: Final = await self.queries.due(scope, now, limit, (after.due_at, after.lens.id) if after else None)
        records: Final = require_storage(await self.state.resolve(heads))
        return tuple(
            DueLens(lens=StoredLens.model_validate(record.value).lens, due_at=utc_time(head.due_at))
            for head, record in zip(heads, records, strict=True)
        )

    async def get(self, lens_id: str) -> Lens | None:
        record: Final = require_storage(await self.state.read(record_key("lens", lens_id)))
        return StoredLens.model_validate(record.value).lens if record.value is not None else None

    async def create(self, lens: Lens) -> Lens:
        previous: Final = require_storage(await self.state.read(record_key("lens", lens.id)))
        if previous.value is not None:
            require_storage(StorageFailure("exists"))
        require_storage(await self.state.commit((Change(previous, document(stored(lens))),)))
        return lens

    async def sync_due(self, lens: Lens) -> None:
        previous: Final = require_storage(await self.state.read(record_key("lens", lens.id)))
        if previous.value is None:
            return
        value: Final = StoredLens.model_validate(previous.value)
        corrected: Final = stored(value.lens)
        if value.lens.version != lens.version or value.due_at == corrected.due_at:
            return
        result: Final = await self.state.commit((Change(previous, document(corrected)),))
        if result is not None and result.kind != "conflict":
            require_storage(result)

    async def update(
        self,
        lens_id: str,
        transform: Callable[[Lens], Lens],
        attempts: int = UPDATE_ATTEMPTS,
        *,
        changed_only: bool = False,
    ) -> Lens | None:
        return await self._update(lens_id, transform, attempts, changed_only, None)

    async def _backoff(self, attempt: int) -> None:
        await self.sleep(random.uniform(0, UPDATE_BACKOFF_SECONDS * min(attempt + 1, 8)))

    async def _update(
        self,
        lens_id: str,
        transform: Callable[[Lens], Lens],
        attempts: int,
        changed_only: bool,
        checkpoint: tuple[str, Review] | None,
    ) -> Lens | None:
        for attempt in range(attempts):
            completed, updated = await self._try_update(lens_id, transform, changed_only, checkpoint)
            if completed:
                return updated
            await self._backoff(attempt)
        return None

    async def _try_update(
        self,
        lens_id: str,
        transform: Callable[[Lens], Lens],
        changed_only: bool,
        checkpoint: tuple[str, Review] | None,
    ) -> tuple[bool, Lens | None]:
        previous: Final = require_storage(await self.state.read(record_key("lens", lens_id)))
        if previous.value is None:
            return True, None
        lens: Final = StoredLens.model_validate(previous.value).lens
        candidate: Final = transform(lens)
        if candidate == lens and checkpoint is None:
            return True, None if changed_only else lens
        updated: Final = candidate.model_copy(update={"version": lens.version + 1}) if candidate != lens else lens
        archives: Final = await self._archive_changes(previous, lens, updated)
        review: Final = (
            (Change(require_storage(await self.state.read(checkpoint[0])), document(checkpoint[1])),)
            if checkpoint is not None
            else ()
        )
        result: Final = await self.state.commit((Change(previous, document(stored(updated))), *archives, *review))
        if result is not None and result.kind != "conflict":
            require_storage(result)
        return result is None, updated

    async def _archive_changes(self, parent: Snapshot, previous: Lens, updated: Lens) -> tuple[Change, ...]:
        retained: Final = frozenset(job.id for job in updated.jobs)
        removed: Final = tuple(job for job in previous.jobs if job.id not in retained)
        records: Final = require_storage(
            await self.state.read_many(tuple(record_key("run", previous.id, job.id) for job in removed))
        )
        return tuple(
            Change(
                record,
                document(
                    ArchivedJob(
                        lens_id=previous.id,
                        parent_key=parent.key,
                        archived_version=updated.version,
                        job=job,
                    )
                ),
            )
            for job, record in zip(removed, records, strict=True)
            if record.value is None
        )

    async def jobs(self, lens_id: str, offset: int = 0) -> tuple[Job, ...]:
        snapshot: Final = require_storage(await self.state.read(record_key("lens", lens_id)))
        if snapshot.value is None:
            return ()
        return await self.queries.jobs(snapshot, StoredLens.model_validate(snapshot.value).lens, offset)

    async def job(self, lens_id: str, job_id: str) -> Job | None:
        snapshot: Final = require_storage(await self.state.read(record_key("lens", lens_id)))
        if snapshot.value is None:
            return None
        jobs: Final = await self.queries.jobs(snapshot, StoredLens.model_validate(snapshot.value).lens, 0, job_id)
        return jobs[0] if jobs else None

    async def trace_findings(self, traces: tuple[TraceIdentity, ...]) -> tuple[TraceFindingCount, ...]:
        return await self.queries.trace_findings(traces)

    async def workers(self) -> tuple[Worker, ...]:
        return await self.access.workers()

    def eligible_workers(self, scope: Scope) -> AsyncIterator[Worker]:
        return self.access.eligible_workers(scope)

    async def worker(self, token_hash: str) -> Worker | None:
        return await self.access.worker(token_hash)

    async def save_worker(self, worker: Worker, token_hash: str | None = None) -> None:
        await self.access.save_worker(worker, token_hash)

    async def configure_service_worker(self, worker: Worker, token_hash: str) -> Worker:
        return await self.access.configure_service_worker(worker, token_hash)

    async def set_worker_billing(self, worker_id: str, key_id: str) -> Worker | None:
        return await self.access.set_worker_billing(worker_id, key_id)

    async def revoke_worker(self, worker_id: str) -> None:
        await self.access.revoke_worker(worker_id)

    async def heartbeat(self, worker_id: str, now: str) -> None:
        await self.access.heartbeat(worker_id, now)
