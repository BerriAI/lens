import asyncio
from datetime import UTC, datetime, timedelta
from typing import Final
from uuid import uuid4

import pytest

from litellm_lens.clickhouse_state import Change, ClickHouseState, PreparedCommit, Snapshot
from litellm_lens.dataset_repository import DatasetRepository, StoredSummary
from litellm_lens.models import CaseSource, Dataset, DatasetCase, DatasetMessage, DatasetSummary
from litellm_lens.persistence import document, record_key

SAVED_AT: Final = datetime(2026, 3, 1, 12, 0, 0, 123000, tzinfo=UTC)


@pytest.fixture
def dataset_ids() -> tuple[str, ...]:
    return tuple(uuid4().hex for _ in range(3))


def _dataset(dataset_id: str, revision: int, case_count: int, team_id: str = "team-a") -> Dataset:
    return Dataset(
        id=dataset_id,
        name=f"Dataset r{revision}",
        agent_name="support-agent",
        team_id=team_id,
        created_at=SAVED_AT,
        revision=revision,
        created_by="user-1",
        cases=tuple(
            DatasetCase(
                id=f"case-{revision}-{i}",
                messages=(DatasetMessage(role="user", content=f"question {i}"),),
                reply=f"answer {i}",
                source=CaseSource(trace_id=f"trace-{i}"),
            )
            for i in range(case_count)
        ),
    )


@pytest.mark.asyncio
async def test_get_returns_requested_revision_and_defaults_to_latest(
    store: ClickHouseState, dataset_ids: tuple[str, ...]
) -> None:
    repo: Final = DatasetRepository(store)
    dataset_id: Final = dataset_ids[0]
    revisions: Final = tuple(_dataset(dataset_id, revision, revision) for revision in (1, 3, 2))
    assert [await repo.insert(dataset, SAVED_AT) for dataset in revisions] == [True, True, True]

    assert await repo.get(dataset_id, revision=1) == revisions[0]
    assert await repo.get(dataset_id, revision=2) == revisions[2]
    assert await repo.get(dataset_id) == revisions[1]


@pytest.mark.asyncio
async def test_get_unknown_dataset_or_revision_returns_none(
    store: ClickHouseState, dataset_ids: tuple[str, ...]
) -> None:
    repo: Final = DatasetRepository(store)
    assert await repo.insert(_dataset(dataset_ids[0], 1, 1), SAVED_AT)

    assert await repo.get(dataset_ids[1]) is None
    assert await repo.get(dataset_ids[1], revision=1) is None
    assert await repo.get(dataset_ids[0], revision=2) is None


@pytest.mark.asyncio
async def test_inserting_an_existing_revision_is_rejected_and_keeps_the_first(
    store: ClickHouseState, dataset_ids: tuple[str, ...]
) -> None:
    repo: Final = DatasetRepository(store)
    original: Final = _dataset(dataset_ids[0], 1, 1)
    overwrite: Final = _dataset(dataset_ids[0], 1, 4).model_copy(update={"name": "Overwritten"})

    assert await repo.insert(original, SAVED_AT) is True
    assert await repo.insert(overwrite, SAVED_AT + timedelta(days=1)) is False

    assert await repo.get(dataset_ids[0], revision=1) == original
    summaries: Final = tuple(s for s in await repo.summaries() if s.summary.id == dataset_ids[0])
    assert tuple(s.summary.updated_at for s in summaries) == (SAVED_AT,)


@pytest.mark.asyncio
async def test_summaries_list_each_dataset_once_at_its_latest_revision_newest_first(
    store: ClickHouseState, dataset_ids: tuple[str, ...]
) -> None:
    repo: Final = DatasetRepository(store)
    older, newer, single = dataset_ids
    writes: Final = (
        (_dataset(older, 1, 1, "team-a"), SAVED_AT),
        (_dataset(older, 2, 3, "team-a"), SAVED_AT + timedelta(minutes=1)),
        (_dataset(newer, 1, 5, "team-b"), SAVED_AT + timedelta(minutes=2)),
        (_dataset(newer, 2, 2, "team-b"), SAVED_AT + timedelta(minutes=4)),
        (_dataset(single, 1, 0, ""), SAVED_AT + timedelta(minutes=3)),
    )
    assert [await repo.insert(dataset, saved_at) for dataset, saved_at in writes] == [True] * len(writes)

    summaries: Final = tuple(s for s in await repo.summaries() if s.summary.id in dataset_ids)

    assert summaries == (
        StoredSummary(
            team_id="team-b",
            summary=DatasetSummary(
                id=newer,
                name="Dataset r2",
                agent_name="support-agent",
                revision=2,
                case_count=2,
                updated_at=SAVED_AT + timedelta(minutes=4),
            ),
        ),
        StoredSummary(
            team_id="",
            summary=DatasetSummary(
                id=single,
                name="Dataset r1",
                agent_name="support-agent",
                revision=1,
                case_count=0,
                updated_at=SAVED_AT + timedelta(minutes=3),
            ),
        ),
        StoredSummary(
            team_id="team-a",
            summary=DatasetSummary(
                id=older,
                name="Dataset r2",
                agent_name="support-agent",
                revision=2,
                case_count=3,
                updated_at=SAVED_AT + timedelta(minutes=1),
            ),
        ),
    )


@pytest.mark.asyncio
async def test_concurrent_revision_writes_keep_exactly_one_payload_and_its_summary(store: ClickHouseState) -> None:
    repo: Final = DatasetRepository(store)
    attempts: Final = tuple(_dataset("same-revision", 1, index) for index in range(16))
    results: Final = await asyncio.gather(*(repo.insert(dataset, SAVED_AT) for dataset in attempts))
    assert results.count(True) == 1
    winner: Final = next(dataset for dataset, accepted in zip(attempts, results, strict=True) if accepted)
    assert await repo.get(winner.id) == winner
    assert await repo.get(winner.id, revision=1) == winner
    assert tuple(entry.summary.case_count for entry in await repo.summaries()) == (len(winner.cases),)


@pytest.mark.asyncio
async def test_concurrent_distinct_revisions_keep_every_revision_and_the_greatest_as_latest(
    store: ClickHouseState,
) -> None:
    repo: Final = DatasetRepository(store)
    revisions: Final = tuple(_dataset("out-of-order", revision, revision) for revision in range(1, 17))
    assert all(await asyncio.gather(*(repo.insert(dataset, SAVED_AT) for dataset in revisions)))
    assert await repo.get("out-of-order") == revisions[-1]
    for expected in revisions:
        assert await repo.get(expected.id, expected.revision) == expected
    assert tuple(entry.summary.revision for entry in await repo.summaries()) == (revisions[-1].revision,)


@pytest.mark.asyncio
async def test_summary_listing_pages_only_published_dataset_metadata(store: ClickHouseState) -> None:
    summaries: Final = tuple(
        StoredSummary(
            team_id="team-a",
            summary=DatasetSummary(
                id=f"dataset-{index:03d}",
                name="Saved",
                agent_name="agent",
                revision=1,
                case_count=index,
                updated_at=SAVED_AT + timedelta(seconds=index),
            ),
        )
        for index in range(130)
    )
    changes: Final = tuple(
        Change(Snapshot(key=record_key("dataset-latest", summary.summary.id)), document(summary))
        for summary in summaries
    )
    assert await store.commit(changes) is None
    abandoned: Final = await store.prepare(
        (Change(Snapshot(key=record_key("dataset-latest", "abandoned")), document(summaries[0])),)
    )
    assert isinstance(abandoned, PreparedCommit)
    assert (
        await store.commit((Change(Snapshot(key=record_key("trace-signal", "unrelated")), {"status": "pending"}),))
        is None
    )
    assert await DatasetRepository(store).summaries() == tuple(reversed(summaries))


@pytest.mark.asyncio
async def test_saved_at_from_legacy_utc_timestamps_sorts_with_timezone_aware_writes(store: ClickHouseState) -> None:
    repository: Final = DatasetRepository(store)
    assert await repository.insert(_dataset("legacy", 1, 1), SAVED_AT.replace(tzinfo=None))
    assert await repository.insert(_dataset("new", 1, 1), SAVED_AT + timedelta(seconds=1))
    summaries: Final = await repository.summaries()
    assert tuple(summary.summary.id for summary in summaries) == ("new", "legacy")
    assert summaries[1].summary.updated_at == SAVED_AT
    assert summaries[1].summary.updated_at.tzinfo is not None
