import asyncio
import os
from collections.abc import AsyncIterator
from types import MappingProxyType
from typing import Final
from uuid import uuid4

import httpx
import pytest
import pytest_asyncio

from litellm_lens.clickhouse_state import Change, ClickHouseState, PreparedCommit, Snapshot, StorageFailure


@pytest_asyncio.fixture
async def store() -> AsyncIterator[ClickHouseState]:
    database: Final = f"lens_state_test_{uuid4().hex}"
    async with httpx.AsyncClient(
        base_url=os.environ["CLICKHOUSE_STATE_TEST_URL"], timeout=30
    ) as administration:
        result: Final = await administration.post(
            "/", params={"query": "CREATE DATABASE {name:Identifier}", "param_name": database}
        )
        result.raise_for_status()
        async with httpx.AsyncClient(
            base_url=os.environ["CLICKHOUSE_STATE_TEST_URL"], params={"database": database}, timeout=30
        ) as client:
            state: Final = ClickHouseState(client)
            assert await state.initialize(f"/state-tests/{database}") is None
            yield state
        removed: Final = await administration.post(
            "/", params={"query": "DROP DATABASE {name:Identifier} SYNC", "param_name": database}
        )
        removed.raise_for_status()


@pytest.mark.asyncio
async def test_concurrent_claims_publish_one_winner(store: ClickHouseState) -> None:
    previous: Final = Snapshot(key="investigation")
    assert await store.commit((Change(previous, {"worker": None}),)) is None
    queued: Final = await store.read(previous.key)
    assert isinstance(queued, Snapshot)
    outcomes: Final = await asyncio.gather(
        *(store.commit((Change(queued, {"worker": str(worker)}),)) for worker in range(16))
    )
    assert sum(outcome is None for outcome in outcomes) == 1
    assert all(outcome is None or outcome.kind == "conflict" for outcome in outcomes)
    winner: Final = next(str(worker) for worker, outcome in enumerate(outcomes) if outcome is None)
    claimed: Final = await store.read(previous.key)
    assert isinstance(claimed, Snapshot)
    assert claimed.value == {"worker": winner}
    assert claimed.revision == queued.revision + 1


@pytest.mark.asyncio
async def test_stale_progress_cannot_publish_its_checkpoint(store: ClickHouseState) -> None:
    assert await store.commit(
        (Change(Snapshot(key="lens"), {"worker": "old"}), Change(Snapshot(key="review"), {"content": "original"}))
    ) is None
    previous: Final = await store.read_many(("lens", "review"))
    assert isinstance(previous, tuple)
    obsolete: Final = await store.prepare(
        (Change(previous[0], {"worker": "old", "done": True}), Change(previous[1], {"content": "stale"}))
    )
    assert isinstance(obsolete, PreparedCommit)
    assert await store.commit((Change(previous[0], {"worker": "new"}),)) is None
    rejected: Final = await store.publish(obsolete)
    assert isinstance(rejected, StorageFailure)
    assert rejected.kind == "conflict"
    current: Final = await store.read_many(("lens", "review"))
    assert isinstance(current, tuple)
    assert tuple(record.value for record in current) == ({"worker": "new"}, {"content": "original"})


@pytest.mark.asyncio
async def test_prepared_payloads_are_invisible_until_published_together(store: ClickHouseState) -> None:
    prepared: Final = await store.prepare(
        (Change(Snapshot(key="lens"), {"completed": 1}), Change(Snapshot(key="review"), {"content": "saved"}))
    )
    assert isinstance(prepared, PreparedCommit)
    before: Final = await store.read_many(("lens", "review"))
    assert isinstance(before, tuple)
    assert tuple(record.value for record in before) == (None, None)
    assert await store.publish(prepared) is None
    after: Final = await store.read_many(("lens", "review"))
    assert isinstance(after, tuple)
    assert tuple(record.value for record in after) == ({"completed": 1}, {"content": "saved"})


@pytest.mark.asyncio
async def test_duplicate_publication_cannot_advance_or_replace_state(store: ClickHouseState) -> None:
    prepared: Final = await store.prepare((Change(Snapshot(key="budget"), {"reserved": 10}),))
    assert isinstance(prepared, PreparedCommit)
    assert await store.publish(prepared) is None
    first: Final = await store.read("budget")
    repeated: Final = await store.publish(prepared)
    assert isinstance(repeated, StorageFailure)
    assert repeated.kind == "conflict"
    assert await store.read("budget") == first


@pytest.mark.asyncio
async def test_large_record_stays_in_clickhouse_and_only_its_pointer_enters_keeper(store: ClickHouseState) -> None:
    value: Final = {"content": "trace evidence \u03bb\n" * 150000}
    assert await store.commit((Change(Snapshot(key="large"), value),)) is None
    observed: Final = await store.read("large")
    assert isinstance(observed, Snapshot)
    assert observed.value == value
    metadata: Final = await store.command(
        "SELECT length(digest) FROM lens_state_heads WHERE key={key:String}",
        MappingProxyType({"key": "large"}),
    )
    assert metadata == f"{len(observed.digest)}\n"


@pytest.mark.asyncio
async def test_tombstone_keeps_revision_so_old_writes_cannot_recreate_a_deleted_record(store: ClickHouseState) -> None:
    key: Final = "key 'quoted' \\ \u03bb"
    assert await store.commit((Change(Snapshot(key=key), {"active": True}),)) is None
    active: Final = await store.read(key)
    assert isinstance(active, Snapshot)
    assert active.value == {"active": True}
    assert await store.commit((Change(active, None),)) is None
    deleted: Final = await store.read(key)
    assert isinstance(deleted, Snapshot)
    assert deleted.value is None
    assert deleted.revision > active.revision
    stale: Final = await store.commit((Change(active, {"active": True}),))
    assert isinstance(stale, StorageFailure)
    assert stale.kind == "conflict"
    assert await store.read(key) == deleted
