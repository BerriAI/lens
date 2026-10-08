import asyncio
import random
from collections.abc import AsyncIterator, Callable
from datetime import datetime
from itertools import chain
from types import MappingProxyType
from typing import Final

from fastapi import HTTPException
from pydantic import AwareDatetime, JsonValue, TypeAdapter

from litellm_lens.clickhouse_state import Change, ClickHouseState, Head, StorageFailure
from litellm_lens.ingestion import IngestionKey
from litellm_lens.models import Record, Scope, Worker
from litellm_lens.persistence import (
    document,
    raise_storage,
    record_key,
    record_pages,
    require_storage,
    select_rows,
    utc_time,
)

INGESTION_KEY_LIMIT: Final = 10000
_KEYS: Final = TypeAdapter(tuple[IngestionKey, ...])
_TOKEN_OWNER: Final = TypeAdapter(str)
_CATALOG: Final = "ingestion-keys/catalog"


class Session(Record):
    expires_at: AwareDatetime


class AccessRepository:
    def __init__(self, state: ClickHouseState) -> None:
        self.state: Final = state

    async def ingestion_keys(self) -> tuple[IngestionKey, ...]:
        stored: Final = require_storage(await self.state.read(_CATALOG))
        keys: Final = _KEYS.validate_python(stored.value or ())
        if len(keys) > INGESTION_KEY_LIMIT:
            raise HTTPException(503, "Lens ingestion key limit exceeded")
        return tuple(sorted(keys, key=lambda key: key.id))

    async def save_ingestion_key(self, key: IngestionKey) -> None:
        def insert(value: JsonValue) -> JsonValue:
            keys: Final = _KEYS.validate_python(value or ())
            if any(existing.id == key.id for existing in keys):
                raise HTTPException(409, "Lens ingestion key already exists")
            if len(keys) >= INGESTION_KEY_LIMIT:
                raise HTTPException(409, "Revoke an unused ingestion key before creating another")
            return [document(entry) for entry in (*keys, key)]

        require_storage(await self.state.update(_CATALOG, insert))

    async def revoke_ingestion_key(self, key_id: str) -> None:
        def revoke(value: JsonValue) -> JsonValue:
            return [document(key) for key in _KEYS.validate_python(value or ()) if key.id != key_id]

        require_storage(await self.state.update(_CATALOG, revoke))

    async def workers(self) -> tuple[Worker, ...]:
        pages: Final = tuple([page async for page in record_pages(self.state, "worker/")])
        records: Final = chain.from_iterable(pages)
        workers: Final = tuple(Worker.model_validate(record.value) for record in records if record.value is not None)
        return tuple(sorted(workers, key=lambda worker: worker.id))

    async def eligible_workers(self, scope: Scope) -> AsyncIterator[Worker]:
        cursor = ""  # rebind-ok: advance through eligible workers in bounded metadata pages
        while workers := await self._eligible_workers_page(scope, cursor):
            for worker in workers:
                yield worker
            if len(workers) < 50:
                return
            cursor = workers[-1].id

    async def _eligible_workers_page(self, scope: Scope, after: str) -> tuple[Worker, ...]:
        heads: Final = await select_rows(
            self.state,
            "SELECT w.key AS key, w.revision AS revision, w.digest AS digest "
            "FROM (SELECT * FROM lens_workers FINAL) AS w "
            "INNER JOIN lens_state_heads AS h USING (key, revision, digest) "
            "WHERE w.revoked=0 AND w.id > {after:String} "
            "AND (w.all_teams=1 OR ({all_teams:Bool}=0 AND w.team_id={team_id:String} "
            "AND ({team_id:String} != '' OR w.api_key_hash={api_key_hash:String}))) "
            "ORDER BY w.id LIMIT 50 FORMAT JSONEachRow",
            Head,
            MappingProxyType(
                {
                    "after": after,
                    "all_teams": str(int(scope.all_teams)),
                    "team_id": scope.team_id,
                    "api_key_hash": scope.api_key_hash,
                }
            ),
        )
        records: Final = require_storage(await self.state.resolve(heads))
        return tuple(Worker.model_validate(record.value) for record in records)

    async def worker(self, token_hash: str) -> Worker | None:
        token: Final = require_storage(await self.state.read(record_key("worker-token", token_hash)))
        if token.value is None:
            return None
        worker: Final = require_storage(
            await self.state.read(record_key("worker", _TOKEN_OWNER.validate_python(token.value)))
        )
        return Worker.model_validate(worker.value) if worker.value is not None else None

    async def save_worker(self, worker: Worker, token_hash: str | None = None) -> None:
        if token_hash is None:
            await self._update_worker(worker.id, lambda _: worker)
            return
        token, previous = require_storage(
            await self.state.read_many((record_key("worker-token", token_hash), record_key("worker", worker.id)))
        )
        if token.value is not None or previous.value is not None:
            raise_storage(StorageFailure("exists"))
        require_storage(await self.state.commit((Change(token, worker.id), Change(previous, document(worker)))))

    async def configure_service_worker(self, worker: Worker, token_hash: str) -> Worker:
        for attempt in range(40):
            match await self._try_configure_service_worker(worker, token_hash):
                case Worker() as updated:
                    return updated
                case StorageFailure(kind="conflict"):
                    await asyncio.sleep(random.uniform(0, 0.02 * min(attempt + 1, 8)))
                case StorageFailure() as failure:
                    raise_storage(failure)
        raise_storage(StorageFailure("conflict"))

    async def _try_configure_service_worker(self, worker: Worker, token_hash: str) -> Worker | StorageFailure:
        token: Final = require_storage(await self.state.read(record_key("worker-token", token_hash)))
        worker_id: Final = _TOKEN_OWNER.validate_python(token.value) if token.value is not None else worker.id
        previous: Final = require_storage(await self.state.read(record_key("worker", worker_id)))
        if token.value is None and previous.value is not None:
            raise_storage(StorageFailure("exists"))
        updated: Final = worker.model_copy(update={"id": worker_id})
        changes: Final = (
            (Change(previous, document(updated)), Change(token, worker_id))
            if token.value is None
            else (Change(previous, document(updated)),)
        )
        result: Final = await self.state.commit(changes)
        return updated if result is None else result

    async def _update_worker(self, worker_id: str, transform: Callable[[Worker], Worker]) -> Worker | None:
        def update(value: JsonValue) -> JsonValue:
            return document(transform(Worker.model_validate(value))) if value is not None else None

        result: Final = require_storage(await self.state.update(record_key("worker", worker_id), update))
        return Worker.model_validate(result.value) if result.value is not None else None

    async def set_worker_billing(self, worker_id: str, key_id: str) -> Worker | None:
        updated: Final = await self._update_worker(
            worker_id,
            lambda worker: worker if worker.revoked else worker.model_copy(update={"analysis_key_id": key_id}),
        )
        return updated if updated is not None and not updated.revoked else None

    async def revoke_worker(self, worker_id: str) -> None:
        await self._update_worker(worker_id, lambda worker: worker.model_copy(update={"revoked": True}))

    async def heartbeat(self, worker_id: str, now: str) -> None:
        at: Final = datetime.fromisoformat(now)
        await self._update_worker(worker_id, lambda worker: worker.model_copy(update={"last_seen": at}))

    async def create_session(self, session_hash: str, expires_at: datetime) -> None:
        previous: Final = require_storage(await self.state.read(record_key("session", session_hash)))
        if previous.value is not None:
            raise_storage(StorageFailure("exists"))
        require_storage(
            await self.state.commit((Change(previous, document(Session(expires_at=utc_time(expires_at)))),))
        )

    async def session_active(self, session_hash: str, now: datetime) -> bool:
        stored: Final = require_storage(await self.state.read(record_key("session", session_hash)))
        return stored.value is not None and Session.model_validate(stored.value).expires_at > utc_time(now)

    async def revoke_session(self, session_hash: str) -> None:
        require_storage(await self.state.update(record_key("session", session_hash), lambda _: None))
