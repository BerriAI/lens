import asyncio
from datetime import UTC, datetime, timedelta
from typing import Final

import httpx
import pytest
from fastapi import HTTPException

from litellm_lens.access_repository import INGESTION_KEY_LIMIT, AccessRepository
from litellm_lens.auth import token_hash
from litellm_lens.clickhouse_state import Change, ClickHouseState
from litellm_lens.ingestion import IngestionKey, IngestionTenant
from litellm_lens.models import Scope, Worker
from litellm_lens.persistence import document, record_key, require_storage
from tests.integration.state_transport import ObservedTransport, ReadMeter

NOW: Final = datetime(2026, 1, 15, tzinfo=UTC)


def _key(index: int) -> IngestionKey:
    return IngestionKey(
        id=f"{index:05d}",
        name="capture",
        tenant=IngestionTenant(user_id="owner", api_key_hash=token_hash(str(index))),
        created_at=NOW,
        expires_at=None,
    )


@pytest.mark.asyncio
async def test_concurrent_creation_cannot_exceed_the_ingestion_key_limit(store: ClickHouseState) -> None:
    repo: Final = AccessRepository(store)
    previous: Final = require_storage(await store.read("ingestion-keys/catalog"))
    require_storage(
        await store.commit((Change(previous, [document(_key(i)) for i in range(INGESTION_KEY_LIMIT - 1)]),))
    )
    attempts: Final = (_key(INGESTION_KEY_LIMIT - 1), _key(INGESTION_KEY_LIMIT))
    results: Final = await asyncio.gather(*(repo.save_ingestion_key(key) for key in attempts), return_exceptions=True)
    assert sum(result is None for result in results) == 1
    failures: Final = tuple(result for result in results if result is not None)
    assert len(failures) == 1 and isinstance(failures[0], HTTPException)
    assert failures[0].status_code == 409
    keys: Final = await AccessRepository(store).ingestion_keys()
    assert len(keys) == INGESTION_KEY_LIMIT
    accepted: Final = next(key for key, result in zip(attempts, results, strict=True) if result is None)
    assert accepted in keys
    await repo.revoke_ingestion_key(accepted.id)
    assert len(await repo.ingestion_keys()) == INGESTION_KEY_LIMIT - 1
    await repo.save_ingestion_key(accepted)
    assert await repo.ingestion_keys() == keys


@pytest.mark.asyncio
async def test_duplicate_worker_token_cannot_replace_its_owner(store: ClickHouseState) -> None:
    repo: Final = AccessRepository(store)
    owner: Final = Worker(id="owner", name="first", scope=Scope(team_id="team"), last_seen=NOW)
    intruder: Final = owner.model_copy(update={"id": "intruder", "scope": Scope(all_teams=True)})
    await repo.save_worker(owner, "token")
    with pytest.raises(HTTPException) as duplicate:
        await repo.save_worker(intruder, "token")
    assert duplicate.value.status_code == 409
    assert await repo.worker("token") == owner
    assert await repo.workers() == (owner,)
    assert await repo.worker("unknown") is None


@pytest.mark.asyncio
async def test_concurrent_heartbeats_and_billing_cannot_restore_revoked_access(store: ClickHouseState) -> None:
    repo: Final = AccessRepository(store)
    worker: Final = Worker(id="worker", name="worker", scope=Scope(team_id="team"), last_seen=NOW)
    await repo.save_worker(worker, "token")
    billing: Final = token_hash("model-key")
    await asyncio.gather(
        repo.revoke_worker(worker.id),
        *(repo.heartbeat(worker.id, (NOW + timedelta(seconds=i)).isoformat()) for i in range(4)),
        repo.set_worker_billing(worker.id, billing),
    )
    revoked: Final = await repo.worker("token")
    assert revoked is not None and revoked.revoked
    assert await repo.set_worker_billing(worker.id, token_hash("replacement")) is None
    assert await repo.worker("token") == revoked
    assert tuple([entry async for entry in repo.eligible_workers(Scope(team_id="team"))]) == ()


@pytest.mark.asyncio
@pytest.mark.parametrize("scope", (Scope(team_id="team"), Scope(api_key_hash="personal"), Scope(all_teams=True)))
async def test_worker_queries_filter_scope_before_reading_documents(store: ClickHouseState, scope: Scope) -> None:
    from litellm_lens.state import can_access

    candidates: Final = tuple(
        Worker(id=f"worker-{i:03d}", name="worker", scope=Scope(team_id="team"), last_seen=NOW) for i in range(52)
    )
    others: Final = (
        Worker(id="all", name="all", scope=Scope(all_teams=True), last_seen=NOW),
        Worker(id="personal", name="key", scope=Scope(api_key_hash="personal"), last_seen=NOW),
        Worker(id="other", name="other", scope=Scope(team_id="other"), last_seen=NOW),
        Worker(id="revoked", name="revoked", scope=scope, last_seen=NOW, revoked=True),
    )
    workers: Final = (*candidates, *others)
    snapshots: Final = require_storage(await store.read_many(tuple(record_key("worker", w.id) for w in workers)))
    require_storage(await store.commit(tuple(Change(s, document(w)) for s, w in zip(snapshots, workers, strict=True))))
    meter: Final = ReadMeter()
    async with httpx.AsyncClient(
        base_url=store.client.base_url,
        params=store.client.params,
        transport=ObservedTransport(meter),
        timeout=30,
    ) as client:
        actual: Final = tuple(
            [entry async for entry in AccessRepository(ClickHouseState(client)).eligible_workers(scope)]
        )
    expected: Final = tuple(
        sorted((w for w in workers if not w.revoked and can_access(w.scope, scope)), key=lambda w: w.id)
    )
    assert actual == expected
    assert meter.document_count == len(expected)
    assert all(len(batch) <= 50 for batch in meter.batches)


@pytest.mark.asyncio
async def test_sessions_expire_at_the_boundary_and_stay_revoked_across_clients(store: ClickHouseState) -> None:
    repo: Final = AccessRepository(store)
    identity: Final = token_hash("browser-cookie")
    expires: Final = NOW + timedelta(hours=1)
    await repo.create_session(identity, expires)
    assert await AccessRepository(store).session_active(identity, expires - timedelta(microseconds=1))
    assert not await repo.session_active(identity, expires)
    assert not await repo.session_active("unknown", NOW)
    await repo.revoke_session(identity)
    assert not await AccessRepository(store).session_active(identity, NOW)
    await repo.revoke_session(identity)
    assert not await repo.session_active(identity, NOW)
