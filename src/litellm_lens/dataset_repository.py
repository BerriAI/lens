import asyncio
import random
from collections.abc import AsyncIterator
from datetime import datetime
from itertools import chain
from typing import Final, Protocol

from litellm_lens.clickhouse_state import Change, ClickHouseState, StorageFailure
from litellm_lens.models import Dataset, DatasetSummary, Record
from litellm_lens.persistence import document, raise_storage, record_key, require_storage, utc_time


class StoredSummary(Record):
    team_id: str
    summary: DatasetSummary


class DatasetStore(Protocol):
    async def summaries(self) -> tuple[StoredSummary, ...]: ...
    async def get(self, dataset_id: str, revision: int | None = None) -> Dataset | None: ...
    async def insert(self, dataset: Dataset, saved_at: datetime) -> bool: ...


class DatasetRepository:
    def __init__(self, state: ClickHouseState) -> None:
        self.state: Final = state

    async def _summary_pages(self) -> AsyncIterator[tuple[StoredSummary, ...]]:
        cursor = ""  # rebind-ok: advance the keyset cursor through bounded metadata pages
        while True:
            keys: Final = require_storage(await self.state.keys("dataset-latest/", after=cursor))
            if not keys:
                return
            records: Final = require_storage(await self.state.read_many(keys))
            yield tuple(StoredSummary.model_validate(record.value) for record in records if record.value is not None)
            cursor = keys[-1]

    async def summaries(self) -> tuple[StoredSummary, ...]:
        pages: Final = tuple([page async for page in self._summary_pages()])
        entries: Final = chain.from_iterable(pages)
        return tuple(sorted(entries, key=lambda entry: entry.summary.updated_at, reverse=True))

    async def get(self, dataset_id: str, revision: int | None = None) -> Dataset | None:
        if revision is None:
            latest: Final = require_storage(await self.state.read(record_key("dataset-latest", dataset_id)))
            if latest.value is None:
                return None
            return await self.get(dataset_id, StoredSummary.model_validate(latest.value).summary.revision)
        record: Final = require_storage(await self.state.read(record_key("dataset", dataset_id, revision)))
        return None if record.value is None else Dataset.model_validate(record.value)

    async def insert(self, dataset: Dataset, saved_at: datetime) -> bool:
        summary: Final = StoredSummary(
            team_id=dataset.team_id,
            summary=DatasetSummary(
                id=dataset.id,
                name=dataset.name,
                agent_name=dataset.agent_name,
                revision=dataset.revision,
                case_count=len(dataset.cases),
                updated_at=utc_time(saved_at),
            ),
        )
        for attempt in range(40):
            revision, latest = require_storage(
                await self.state.read_many(
                    (record_key("dataset", dataset.id, dataset.revision), record_key("dataset-latest", dataset.id))
                )
            )
            if revision.value is not None:
                return False
            advance: Final = latest.value is None or (
                dataset.revision > StoredSummary.model_validate(latest.value).summary.revision
            )
            changes: Final = (
                (Change(revision, document(dataset)), Change(latest, document(summary)))
                if advance
                else (Change(revision, document(dataset)),)
            )
            result: Final = await self.state.commit(changes)
            if result is None:
                return True
            if result.kind != "conflict":
                require_storage(result)
            await asyncio.sleep(random.uniform(0, 0.02 * min(attempt + 1, 8)))
        raise_storage(StorageFailure("conflict"))
