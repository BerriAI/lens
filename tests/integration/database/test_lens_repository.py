import asyncio
from datetime import UTC, datetime, timedelta, timezone
from typing import Final
from uuid import uuid4

import pytest
from fastapi import HTTPException

from litellm_lens.clickhouse_state import Change, ClickHouseState
from litellm_lens.models import (
    Check,
    Evidence,
    Execution,
    Finding,
    Job,
    Lens,
    LensSettings,
    Progress,
    RunAssessment,
    Sample,
    Scope,
    TraceFindingCount,
    TraceIdentity,
    Worker,
)
from litellm_lens.persistence import document, record_key, require_storage
from litellm_lens.repository import LensRepository, StoredLens, review_key
from litellm_lens.state import cancel_job, claim_job, current_job, due_at, end_job, queue_job, replace_job


async def _stored_due_at(repo: LensRepository, lens_id: str) -> datetime | None:
    record: Final = require_storage(await repo.state.read(record_key("lens", lens_id)))
    return StoredLens.model_validate(record.value).due_at


async def _assert_due_column(repo: LensRepository, lens_id: str) -> None:
    stored: Final = await repo.get(lens_id)
    assert stored is not None
    assert await _stored_due_at(repo, lens_id) == due_at(stored)


async def _rewrite_due(repo: LensRepository, lens_id: str, at: datetime, bump: bool = False) -> None:
    previous: Final = require_storage(await repo.state.read(record_key("lens", lens_id)))
    value: Final = StoredLens.model_validate(previous.value)
    updated: Final = value.lens.model_copy(update={"version": value.lens.version + 1}) if bump else value.lens
    require_storage(await repo.state.commit((Change(previous, document(StoredLens(lens=updated, due_at=at))),)))


async def _rewrite_scope(repo: LensRepository, lens_id: str, scope: Scope) -> None:
    previous: Final = require_storage(await repo.state.read(record_key("lens", lens_id)))
    value: Final = StoredLens.model_validate(previous.value)
    raw: Final = value.model_dump(mode="json")
    payload: Final = {
        **raw,
        "lens": {**value.lens.model_dump(mode="json"), "scope": scope.model_dump(mode="json", exclude_defaults=True)},
    }
    require_storage(await repo.state.commit((Change(previous, payload),)))


def _scheduled_lens(
    lens_id: str,
    scope: Scope,
    now: datetime,
    next_run_at: datetime,
    *,
    enabled: bool = True,
    jobs: tuple[Job, ...] = (),
) -> Lens:
    return Lens(
        id=lens_id,
        scope=scope,
        settings=LensSettings(
            name="Scheduling test", model="analysis", context="Find unexpected behavior", enabled=enabled
        ),
        created_at=now,
        next_run_at=next_run_at,
        jobs=jobs,
        budget_month=now.strftime("%Y-%m"),
    )


@pytest.mark.asyncio
async def test_due_filters_by_schedule_and_scope(store: ClickHouseState) -> None:
    utc_now: Final = datetime.now(UTC).replace(microsecond=0)
    worker_now: Final = utc_now.astimezone(timezone(timedelta(hours=3)))
    team_id: Final = uuid4().hex
    worker_scope: Final = Scope(team_id=team_id)
    worker: Final = Worker(id=uuid4().hex, name="worker", scope=worker_scope, last_seen=worker_now)
    repo: Final = LensRepository(store)
    due_lens: Final = _scheduled_lens(uuid4().hex, worker_scope, utc_now, utc_now - timedelta(minutes=20))
    future_lens: Final = _scheduled_lens(uuid4().hex, worker_scope, utc_now, utc_now + timedelta(minutes=20))
    disabled_lens: Final = _scheduled_lens(
        uuid4().hex, worker_scope, utc_now, utc_now - timedelta(minutes=10), enabled=False
    )
    live_queued: Final = queue_job(
        _scheduled_lens(uuid4().hex, worker_scope, utc_now - timedelta(minutes=5), utc_now - timedelta(minutes=5)),
        utc_now - timedelta(minutes=5),
        uuid4().hex,
    )
    live_lens: Final = claim_job(live_queued, worker, worker_now)
    expired_queued: Final = queue_job(
        _scheduled_lens(uuid4().hex, worker_scope, utc_now - timedelta(minutes=10), utc_now - timedelta(minutes=10)),
        utc_now - timedelta(minutes=10),
        uuid4().hex,
    )
    expired_claimed: Final = claim_job(expired_queued, worker, utc_now - timedelta(minutes=10))
    expired_job: Final = expired_claimed.jobs[0].model_copy(update={"lease_until": utc_now - timedelta(minutes=5)})
    expired_lens: Final = expired_claimed.model_copy(update={"jobs": (expired_job,)})
    other_lens: Final = _scheduled_lens(
        uuid4().hex, Scope(team_id=uuid4().hex), utc_now, utc_now - timedelta(minutes=3)
    )
    worker_key: Final = uuid4().hex
    key_lens: Final = _scheduled_lens(
        uuid4().hex, Scope(api_key_hash=worker_key), utc_now, utc_now - timedelta(minutes=2)
    )
    candidates: Final = (due_lens, future_lens, disabled_lens, live_lens, expired_lens, other_lens, key_lens)
    await asyncio.gather(*(repo.create(candidate) for candidate in candidates))
    await _rewrite_scope(repo, due_lens.id, Scope(team_id=team_id))
    await _rewrite_scope(repo, key_lens.id, Scope(api_key_hash=worker_key))
    team_due: Final = await repo.due(worker_scope, worker_now, 20)
    assert tuple(candidate.lens.id for candidate in team_due) == tuple(
        lens.id for lens in sorted((due_lens, expired_lens), key=lambda lens: (due_at(lens), lens.id))
    )
    assert team_due[0].lens.scope == worker_scope
    key_due: Final = await repo.due(Scope(api_key_hash=worker_key), worker_now, 20)
    assert tuple(candidate.lens.id for candidate in key_due) == (key_lens.id,)
    all_due: Final = await repo.due(Scope(all_teams=True), worker_now, 20)
    assert {candidate.lens.id for candidate in all_due} == {due_lens.id, expired_lens.id, other_lens.id, key_lens.id}


@pytest.mark.asyncio
async def test_due_pages_lenses_with_equal_due_at_without_skipping_or_repeating(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC).replace(microsecond=0)
    scope: Final = Scope(team_id=uuid4().hex)
    repo: Final = LensRepository(store)
    lenses: Final = tuple(_scheduled_lens(uuid4().hex, scope, now, now - timedelta(minutes=1)) for _ in range(45))
    await asyncio.gather(*(repo.create(lens) for lens in lenses))
    await asyncio.gather(*(_rewrite_due(repo, item.id, datetime(1970, 1, 1, tzinfo=UTC)) for item in lenses))
    first: Final = await repo.due(scope, now, 20)
    second: Final = await repo.due(scope, now, 20, first[-1])
    third: Final = await repo.due(scope, now, 20, second[-1])
    assert tuple(len(page) for page in (first, second, third)) == (20, 20, 5)
    ids: Final = tuple(candidate.lens.id for candidate in (*first, *second, *third))
    assert ids == tuple(sorted(lens.id for lens in lenses))


@pytest.mark.asyncio
async def test_due_at_stays_consistent_through_job_lifecycle(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC).replace(microsecond=0)
    scope: Final = Scope(team_id=uuid4().hex)
    lens: Final = _scheduled_lens(uuid4().hex, scope, now, now)
    repo: Final = LensRepository(store)
    worker: Final = Worker(id=uuid4().hex, name="worker", scope=scope, last_seen=now)
    await repo.create(lens)
    await _assert_due_column(repo, lens.id)
    job_id: Final = uuid4().hex
    claimed: Final = await repo.update(
        lens.id, lambda candidate: claim_job(queue_job(candidate, now, job_id), worker, now), attempts=1
    )
    assert claimed is not None
    await _assert_due_column(repo, lens.id)
    active: Final = current_job(claimed)
    assert active is not None
    progressed: Final = await repo.progress(lens.id, active, Progress())
    assert progressed is not None
    await _assert_due_column(repo, lens.id)
    result_at: Final = datetime.now(UTC)

    def finish(candidate: Lens) -> Lens:
        active_job: Final = current_job(candidate)
        if active_job is None:
            return candidate
        return replace_job(candidate, end_job(active_job, "completed", result_at)).model_copy(
            update={"next_run_at": result_at + timedelta(minutes=candidate.settings.interval_minutes)}
        )

    completed: Final = await repo.update(lens.id, finish, attempts=1)
    assert completed is not None
    await _assert_due_column(repo, lens.id)
    cancelled_at: Final = datetime.now(UTC)
    cancelled: Final = await repo.update(
        lens.id,
        lambda candidate: cancel_job(queue_job(candidate, cancelled_at, uuid4().hex, trigger="manual"), cancelled_at),
        attempts=1,
    )
    assert cancelled is not None
    await _assert_due_column(repo, lens.id)


@pytest.mark.asyncio
async def test_sync_due_repairs_legacy_rows_and_ignores_stale_versions(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC).replace(microsecond=0)
    team_id: Final = uuid4().hex
    scope: Final = Scope(team_id=team_id)
    worker: Final = Worker(id=uuid4().hex, name="worker", scope=scope, last_seen=now)
    repo: Final = LensRepository(store)
    due_idle: Final = _scheduled_lens(uuid4().hex, scope, now, now - timedelta(minutes=20))
    future_idle: Final = _scheduled_lens(uuid4().hex, scope, now, now + timedelta(minutes=20))
    disabled_idle: Final = _scheduled_lens(uuid4().hex, scope, now, now - timedelta(minutes=10), enabled=False)
    queued_lens: Final = queue_job(
        _scheduled_lens(uuid4().hex, scope, now, now + timedelta(minutes=20), enabled=False),
        now - timedelta(minutes=3),
        uuid4().hex,
        trigger="manual",
    )
    live_lens: Final = claim_job(
        queue_job(
            _scheduled_lens(uuid4().hex, scope, now, now + timedelta(minutes=20)),
            now - timedelta(minutes=10),
            uuid4().hex,
        ),
        worker,
        now,
    )
    expired_claimed: Final = claim_job(
        queue_job(
            _scheduled_lens(uuid4().hex, scope, now, now + timedelta(minutes=20)),
            now - timedelta(minutes=10),
            uuid4().hex,
        ),
        worker,
        now - timedelta(minutes=10),
    )
    expired_lens: Final = expired_claimed.model_copy(
        update={"jobs": (expired_claimed.jobs[0].model_copy(update={"lease_until": now - timedelta(minutes=5)}),)}
    )
    candidates: Final = (due_idle, future_idle, disabled_idle, queued_lens, live_lens, expired_lens)
    await asyncio.gather(*(repo.create(candidate) for candidate in candidates))
    past: Final = now - timedelta(hours=1)
    await asyncio.gather(*(_rewrite_due(repo, item.id, past) for item in candidates))
    legacy_due: Final = await repo.due(scope, now, 20)
    assert {candidate.lens.id for candidate in legacy_due} == {lens.id for lens in candidates}
    for candidate in legacy_due:
        await repo.sync_due(candidate.lens)
    repaired_due: Final = await repo.due(scope, now, 20)
    assert {candidate.lens.id for candidate in repaired_due} == {due_idle.id, queued_lens.id, expired_lens.id}
    await asyncio.gather(*(_assert_due_column(repo, lens.id) for lens in candidates))
    stale: Final = await repo.get(future_idle.id)
    assert stale is not None
    await _rewrite_due(repo, stale.id, past, bump=True)
    await repo.sync_due(stale)
    assert await _stored_due_at(repo, stale.id) == past


@pytest.mark.asyncio
async def test_concurrent_workers_cannot_both_acquire_the_same_job(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC)
    scope: Final = Scope(team_id=uuid4().hex)
    repo: Final = LensRepository(store)
    lens: Final = Lens(
        id=uuid4().hex,
        scope=scope,
        settings=LensSettings(name="Lease test", model="test", checks=(Check(id="c", instruction="Find retries"),)),
        created_at=now,
        next_run_at=now,
        budget_month=now.strftime("%Y-%m"),
    )
    await repo.create(queue_job(lens, now, uuid4().hex))
    workers: Final = tuple(Worker(id=uuid4().hex, name="worker", scope=scope, last_seen=now) for _ in range(2))
    results: Final = await asyncio.gather(*(repo.update(lens.id, lambda e, w=w: claim_job(e, w, now)) for w in workers))
    stored: Final = await repo.get(lens.id)
    assert stored is not None
    assert stored.jobs[0].attempts == 1
    assert stored.jobs[0].worker_id in tuple(w.id for w in workers)
    assert tuple(r.jobs[0].worker_id for r in results if r) == (stored.jobs[0].worker_id, stored.jobs[0].worker_id)


@pytest.mark.asyncio
async def test_heartbeat_never_restores_revoked_access(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    worker: Final = Worker(id=uuid4().hex, name="worker", scope=Scope(team_id=uuid4().hex), last_seen=now)
    token_hash: Final = uuid4().hex
    await repo.save_worker(worker, token_hash)
    await repo.save_worker(worker.model_copy(update={"revoked": True}))
    await repo.heartbeat(worker.id, now.isoformat())
    stored: Final = await repo.worker(token_hash)
    assert stored is not None and stored.revoked is True


@pytest.mark.asyncio
async def test_managed_registration_is_atomic_and_keeps_the_original_worker_id(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    token_hash: Final = uuid4().hex
    workers: Final = tuple(
        Worker(id=uuid4().hex, name="Managed Lens", scope=Scope(all_teams=True), last_seen=now) for _ in range(8)
    )
    registered: Final = await asyncio.gather(*(repo.configure_service_worker(w, token_hash) for w in workers))
    assert len(frozenset(w.id for w in registered)) == 1
    assert await repo.worker(token_hash) == registered[0]
    await repo.revoke_worker(registered[0].id)
    restored: Final = await repo.configure_service_worker(workers[-1], token_hash)
    assert restored.id == registered[0].id
    assert restored.revoked is False


@pytest.mark.asyncio
async def test_claim_pages_only_yield_work_the_worker_can_claim(store: ClickHouseState) -> None:
    clock: Final = datetime.now(UTC)
    now: Final = clock.replace(microsecond=clock.microsecond // 1000 * 1000)
    scope: Final = Scope(team_id=uuid4().hex)
    repo: Final = LensRepository(store)
    prefix: Final = uuid4().hex
    base: Final = Lens(
        id=prefix,
        scope=scope,
        settings=LensSettings(
            name="Candidate pagination", model="test", enabled=False, context="Find repeated failures"
        ),
        created_at=now,
        next_run_at=now + timedelta(days=1),
        budget_month=now.strftime("%Y-%m"),
    )
    queued: Final = tuple(
        queue_job(base.model_copy(update={"id": f"{prefix}-{i:03d}"}), now, uuid4().hex) for i in range(52)
    )
    other_scope: Final = queued[0].model_copy(update={"id": f"{prefix}-other", "scope": Scope(team_id=uuid4().hex)})
    due: Final = base.model_copy(
        update={
            "id": f"{prefix}-due",
            "settings": base.settings.model_copy(update={"enabled": True}),
            "next_run_at": now,
        }
    )
    live: Final = claim_job(queued[0], Worker(id=prefix, name="worker", scope=scope, last_seen=now), now)
    expired: Final = live.model_copy(
        update={
            "id": f"{prefix}-expired",
            "jobs": (live.jobs[0].model_copy(update={"lease_until": now - timedelta(seconds=1)}),),
        }
    )
    rows: Final = (*queued[1:], live, base, due, expired, other_scope)
    for row in rows:
        await repo.create(row)
    first: Final = await repo.due(scope, now, 50)
    second: Final = await repo.due(scope, now, 50, first[-1])
    assert len(first) == 50
    found: Final = tuple(candidate.lens for candidate in (*first, *second))
    assert frozenset(candidate.id for candidate in found) == frozenset(
        candidate.id for candidate in (*queued[1:], due, expired)
    )
    assert len(found) == 53


@pytest.mark.asyncio
async def test_trace_findings_include_archived_assessments_without_counting_retries_or_counterexamples(
    store: ClickHouseState,
) -> None:
    now: Final = datetime.now(UTC)
    prefix: Final = uuid4().hex
    repo: Final = LensRepository(store)
    settings: Final = LensSettings(name="Finding counts", model="test", context="Answer the question")
    identities: Final = tuple(TraceIdentity(trace_id=prefix, trace_ref=f"{prefix}-{i}") for i in range(7))
    executions: Final = tuple(
        (
            Execution(
                id=f"{prefix}-{i}",
                source="traces",
                trace_id=identity.trace_id,
                trace_ref=identity.trace_ref,
                team_id="",
                name="Run",
                start_time=now.isoformat(),
                span_count=1,
            )
            for i, identity in enumerate(identities)
        )
    )
    finding: Final = Finding(
        id=prefix,
        title="Repeated lookup",
        description="The agent never answered the question",
        check_id="expected_behavior",
        first_seen=now,
        last_seen=now,
        revision=1,
        occurrences=(executions[0].id,),
        evidence=(
            Evidence(execution_id=executions[0].id, span_id="step", quote="no answer"),
            Evidence(execution_id=executions[1].id, span_id="step", quote="answered", role="counterexample"),
        ),
    )
    completed: Final = Job(
        id=f"{prefix}-old",
        status="completed",
        created_at=now,
        start=now,
        end=now,
        settings=settings,
        revision=1,
        sample=Sample(executions=executions[:4], eligible=4),
        assessments=(
            RunAssessment(execution_id=executions[0].id),
            RunAssessment(execution_id=executions[1].id),
            RunAssessment(execution_id=executions[2].id, cannot_assess=True),
        ),
        findings=(finding,),
    )
    lens: Final = Lens(
        id=prefix,
        scope=Scope(all_teams=True),
        settings=settings,
        created_at=now,
        next_run_at=now,
        budget_month=now.strftime("%Y-%m"),
        jobs=(completed,),
    )
    await repo.create(lens)
    current: Final = completed.model_copy(update={"id": f"{prefix}-current"})
    unfinished: Final = tuple(
        (
            completed.model_copy(
                update={
                    "id": f"{prefix}-{status}",
                    "status": status,
                    "sample": Sample(executions=(executions[index],), eligible=1),
                    "assessments": (RunAssessment(execution_id=executions[index].id),),
                    "findings": (),
                }
            )
            for index, status in enumerate(("running", "failed", "cancelled"), start=4)
        )
    )
    await repo.update(prefix, lambda item: item.model_copy(update={"jobs": (current, *unfinished)}))
    archived: Final = await repo.job(prefix, completed.id)
    assert archived is not None and archived.status == "completed"
    expected: Final = tuple(
        (
            TraceFindingCount(**identity.model_dump(), finding_count=1 if i == 0 else 0 if i == 1 else None)
            for i, identity in enumerate(identities)
        )
    )
    counts: Final = await repo.trace_findings(identities)
    assert sorted(counts, key=lambda item: item.trace_ref) == list(expected)
    await repo.update(prefix, lambda item: item.model_copy(update={"jobs": unfinished}))
    archived_counts: Final = await repo.trace_findings(identities)
    assert sorted(archived_counts, key=lambda item: item.trace_ref) == list(expected)
    assert await repo.trace_findings((TraceIdentity(trace_id=prefix),)) == (
        TraceFindingCount(trace_id=prefix, finding_count=None),
    )


@pytest.mark.asyncio
async def test_review_checkpoints_survive_new_jobs_and_only_relevant_settings_invalidate_them(
    store: ClickHouseState,
) -> None:
    from litellm_lens.models import Extraction, Review, ReviewVersion

    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    settings: Final = LensSettings(name="Checkpoint test", model="analysis", context="Find blocked user requests")
    execution: Final = Execution(
        id=uuid4().hex,
        source="traces",
        trace_id=uuid4().hex,
        team_id="",
        name="task",
        start_time=now.isoformat(),
        span_count=1,
    )
    lens: Final = Lens(
        id=uuid4().hex,
        scope=Scope(all_teams=True),
        settings=settings,
        created_at=now,
        next_run_at=now,
        budget_month=now.strftime("%Y-%m"),
    )
    job: Final = (
        queue_job(lens, now, uuid4().hex)
        .jobs[0]
        .model_copy(
            update={
                "sample": Sample(executions=(execution,), eligible=1),
                "status": "running",
                "worker_id": uuid4().hex,
                "attempts": 1,
                "lease_until": now + timedelta(minutes=5),
            }
        )
    )
    checkpoint: Final = Review(
        execution_id=execution.id,
        trace_id=execution.trace_id,
        agent="agent",
        name="task",
        model=settings.model,
        duration_ms=10,
        at=now,
        content_version="version-1",
        extraction=Extraction(),
    )
    await repo.create(lens.model_copy(update={"jobs": (job,)}))
    assert await repo.progress(lens.id, job, Progress(review=checkpoint)) is not None
    resumed: Final = job.model_copy(
        update={"id": uuid4().hex, "settings": settings.model_copy(update={"monthly_budget": 200})}
    )
    assert await repo.reviews(lens.id, resumed) == (checkpoint,)
    await repo.complete_reviews(
        lens.id, resumed, (ReviewVersion(execution_id=execution.id, content_version="version-1"),)
    )
    assert (await repo.reviews(lens.id, resumed))[0].consolidated
    changed: Final = resumed.model_copy(
        update={"settings": settings.model_copy(update={"context": "Find fabricated answers"})}
    )
    assert await repo.reviews(lens.id, changed) == ()
    updated: Final = checkpoint.model_copy(update={"content_version": "version-2"})
    await repo.update(lens.id, lambda value: value.model_copy(update={"jobs": (resumed,)}))
    assert await repo.progress(lens.id, resumed, Progress(review=updated)) is not None
    await repo.complete_reviews(
        lens.id, resumed, (ReviewVersion(execution_id=execution.id, content_version="version-1"),)
    )
    assert await repo.reviews(lens.id, resumed) == (updated,)


@pytest.mark.asyncio
@pytest.mark.parametrize("lost_ownership", ("expired", "reassigned", "same_worker", "cancelled", "next_job"))
async def test_delayed_progress_cannot_replace_a_newer_checkpoint(store: ClickHouseState, lost_ownership: str) -> None:
    from litellm_lens.models import Extraction, Review
    from tests.unit.litellm_lens.test_state import lens, worker

    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    claimed: Final = claim_job(queue_job(lens(), now, uuid4().hex), worker(), now).model_copy(
        update={"id": uuid4().hex}
    )
    old: Final = claimed.jobs[0]
    newer: Final = Review(
        execution_id="trace",
        trace_id="trace",
        agent="agent",
        name="task",
        model="analysis",
        duration_ms=1,
        at=now,
        content_version="new",
        extraction=Extraction(),
    )
    await repo.create(claimed)
    assert await repo.progress(claimed.id, old, Progress(review=newer)) is not None
    next_owner: Final = old.model_copy(
        update={
            "status": "cancelled" if lost_ownership == "cancelled" else "running",
            "lease_until": now - timedelta(seconds=1) if lost_ownership == "expired" else old.lease_until,
            "attempts": old.attempts + 1 if lost_ownership in ("reassigned", "same_worker") else old.attempts,
            "worker_id": "replacement" if lost_ownership == "reassigned" else old.worker_id,
            "id": uuid4().hex if lost_ownership == "next_job" else old.id,
        }
    )
    await repo.update(claimed.id, lambda value: value.model_copy(update={"jobs": (next_owner,)}))
    stale: Final = newer.model_copy(update={"content_version": "old"})
    with pytest.raises(HTTPException) as error:
        await repo.progress(claimed.id, old, Progress(review=stale))
    assert error.value.status_code == 409
    record: Final = require_storage(await store.read(review_key(claimed.id, old, newer.execution_id)))
    assert Review.model_validate(record.value) == newer
    stored: Final = await repo.get(claimed.id)
    assert stored is not None and stored.jobs == (next_owner,)


@pytest.mark.asyncio
async def test_legacy_finding_run_provenance_is_recovered_from_archived_and_current_jobs(
    store: ClickHouseState,
) -> None:
    from litellm_lens.state import merge_finding
    from tests.unit.litellm_lens.test_state import NOW, finding, lens

    repo: Final = LensRepository(store)
    saved: Final = merge_finding(lens(), finding("trace"), 1, NOW)
    old: Final = (
        queue_job(lens(), NOW, uuid4().hex).jobs[0].model_copy(update={"status": "completed", "findings": (saved,)})
    )
    stored: Final = lens().model_copy(update={"id": uuid4().hex, "findings": (saved,), "jobs": (old,)})
    await repo.create(stored)
    await repo.update(stored.id, lambda value: queue_job(value, NOW, uuid4().hex))
    matches: Final = await repo.finding_runs(stored.id, (saved.id,))
    assert tuple((match.finding_id, match.job_id) for match in matches) == ((saved.id, old.id),)
    assert await repo.finding_runs(stored.id, ("unrelated",)) == ()


@pytest.mark.asyncio
async def test_locked_settlement_charges_every_concurrent_call_exactly_once(store: ClickHouseState) -> None:
    from litellm_lens.inference import settle_amount
    from litellm_lens.models import BudgetReservation, Step
    from tests.unit.litellm_lens.test_state import lens

    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    queued: Final = queue_job(lens(), now, uuid4().hex)
    holds: Final = tuple(
        BudgetReservation(id=uuid4().hex, job_id=queued.jobs[0].id, amount=1, month=queued.budget_month)
        for _ in range(50)
    )
    step: Final = Step(at=now, kind="model", label="Reviewed a run", cost=0.25)
    await repo.create(queued.model_copy(update={"reservations": holds}))

    async def settle(hold: BudgetReservation) -> None:
        updated: Final = await repo.update_locked(queued.id, lambda value: settle_amount(value, hold.id, 0.25, step))
        assert updated is not None

    await asyncio.gather(*(settle(hold) for hold in (*holds, *holds)))
    stored: Final = await repo.get(queued.id)
    assert stored is not None
    assert stored.spent == queued.spent + 50 * 0.25
    assert stored.jobs[0].cost == 50 * 0.25
    assert len(stored.jobs[0].steps) == 50
    assert stored.reservations == ()


@pytest.mark.asyncio
async def test_progress_rechecks_ownership_after_waiting_for_a_concurrent_update(store: ClickHouseState) -> None:
    import httpx

    from litellm_lens.models import Extraction, Review
    from tests.integration.state_transport import ObservedTransport, PublicationGate, ReadMeter
    from tests.unit.litellm_lens.test_state import lens, worker

    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    claimed: Final = claim_job(queue_job(lens(), now, uuid4().hex), worker(), now)
    old: Final = claimed.jobs[0]
    review: Final = Review(
        execution_id="trace",
        trace_id="trace",
        agent="agent",
        name="task",
        model="analysis",
        duration_ms=1,
        at=now,
        content_version="stale",
        extraction=Extraction(),
    )
    await repo.create(claimed)
    gate: Final = PublicationGate()
    async with httpx.AsyncClient(
        base_url=store.client.base_url,
        params=store.client.params,
        transport=ObservedTransport(ReadMeter(), gate.before),
        timeout=30,
    ) as client:
        delayed: Final = asyncio.create_task(
            LensRepository(ClickHouseState(client)).progress(claimed.id, old, Progress(review=review))
        )
        await asyncio.wait_for(gate.entered.wait(), 5)
        await repo.update(
            claimed.id, lambda value: replace_job(value, old.model_copy(update={"worker_id": "replacement"}))
        )
        gate.release.set()
        with pytest.raises(HTTPException) as error:
            await delayed
    assert error.value.status_code == 409
    checkpoint: Final = require_storage(await store.read(review_key(claimed.id, old, review.execution_id)))
    assert checkpoint.value is None
    stored: Final = await repo.get(claimed.id)
    assert stored is not None and stored.jobs[0].worker_id == "replacement"
    assert stored.jobs[0].reviewed == 0


@pytest.mark.asyncio
async def test_checkpoint_and_progress_commit_together_or_remain_unchanged(store: ClickHouseState) -> None:
    import httpx

    from litellm_lens.models import Extraction, Review
    from tests.integration.state_transport import ObservedTransport, ReadMeter
    from tests.unit.litellm_lens.test_state import lens, worker

    now: Final = datetime.now(UTC)
    repo: Final = LensRepository(store)
    claimed: Final = claim_job(queue_job(lens(), now, uuid4().hex), worker(), now)
    assigned: Final = claimed.jobs[0]
    review: Final = Review(
        execution_id="trace",
        trace_id="trace",
        agent="agent",
        name="task",
        model="analysis",
        duration_ms=1,
        at=now,
        content_version="content",
        extraction=Extraction(),
    )
    await repo.create(claimed)

    async def unavailable(request: httpx.Request) -> None:
        if request.url.params.get("query", "").startswith("ALTER TABLE lens_state_heads"):
            raise httpx.ConnectError("Storage unavailable before publication", request=request)

    async with httpx.AsyncClient(
        base_url=store.client.base_url,
        params=store.client.params,
        transport=ObservedTransport(ReadMeter(), unavailable),
        timeout=30,
    ) as client:
        with pytest.raises(HTTPException) as error:
            await LensRepository(ClickHouseState(client)).progress(claimed.id, assigned, Progress(review=review))
    assert error.value.status_code == 503
    assert await repo.get(claimed.id) == claimed
    assert require_storage(await store.read(review_key(claimed.id, assigned, review.execution_id))).value is None
    updated: Final = await repo.progress(claimed.id, assigned, Progress(review=review))
    assert updated is not None and updated == await repo.get(claimed.id)
    checkpoint: Final = require_storage(await store.read(review_key(claimed.id, assigned, review.execution_id)))
    assert Review.model_validate(checkpoint.value) == review
    assert updated.jobs[0].reviewed == 1
    assert updated.jobs[0].reviews == (review.model_copy(update={"extraction": None, "content_version": ""}),)


@pytest.mark.asyncio
async def test_failed_archive_publication_never_exposes_an_obsolete_run(store: ClickHouseState) -> None:
    import httpx

    from tests.integration.state_transport import ObservedTransport, PublicationGate, ReadMeter
    from tests.unit.litellm_lens.test_state import NOW, lens

    repo: Final = LensRepository(store)
    queued: Final = queue_job(lens(), NOW, "history")
    old: Final = queued.jobs[0].model_copy(update={"status": "completed", "cost": 1})
    await repo.create(queued.model_copy(update={"jobs": (old,)}))
    gate: Final = PublicationGate()
    async with httpx.AsyncClient(
        base_url=store.client.base_url,
        params=store.client.params,
        transport=ObservedTransport(ReadMeter(), gate.before),
        timeout=30,
    ) as client:
        delayed: Final = asyncio.create_task(
            LensRepository(ClickHouseState(client)).update(
                queued.id,
                lambda value: value.model_copy(update={"jobs": ()}),
                attempts=1,
            )
        )
        await asyncio.wait_for(gate.entered.wait(), 5)
        assert await repo.jobs(queued.id) == (old,)
        corrected: Final = old.model_copy(update={"cost": 2})
        await repo.update(queued.id, lambda value: replace_job(value, corrected))
        gate.release.set()
        assert await delayed is None
    assert await repo.jobs(queued.id) == (corrected,)
    await repo.update(queued.id, lambda value: value.model_copy(update={"jobs": ()}))
    assert await repo.jobs(queued.id) == (corrected,)
    assert await repo.job(queued.id, old.id) == corrected


@pytest.mark.asyncio
async def test_history_pages_keep_time_and_id_order_across_archived_and_current_runs(store: ClickHouseState) -> None:
    from tests.unit.litellm_lens.test_state import NOW, lens

    repo: Final = LensRepository(store)
    jobs: Final = tuple(queue_job(lens(), NOW, f"run-{i:03d}").jobs[0] for i in range(56))
    saved: Final = lens().model_copy(update={"jobs": jobs})
    await repo.create(saved)
    await repo.update(saved.id, lambda value: value.model_copy(update={"jobs": jobs[-2:]}))
    first: Final = await repo.jobs(saved.id)
    second: Final = await repo.jobs(saved.id, 50)
    assert len(first) == 50 and len(second) == 6
    assert (*first, *second) == tuple(reversed(jobs))
    assert await repo.jobs(saved.id, 100) == ()
    assert await repo.job(saved.id, "absent") is None


@pytest.mark.asyncio
@pytest.mark.parametrize("losses", (12, 40))
async def test_update_backs_off_and_respects_its_contention_limit(store: ClickHouseState, losses: int) -> None:
    import httpx

    from litellm_lens.repository import UPDATE_ATTEMPTS
    from tests.integration.state_transport import ObservedTransport, ReadMeter
    from tests.unit.litellm_lens.test_state import lens

    class Conflicts:
        def __init__(self) -> None:
            self.writes = 0
            self.waits: tuple[float, ...] = ()

        async def respond(self, request: httpx.Request) -> httpx.Response | None:
            if request.url.params.get("query", "").startswith("ALTER TABLE lens_state_heads"):
                self.writes += 1
                if self.writes <= losses:
                    return httpx.Response(500, text="Code: 395. LENS_STATE_CONFLICT", request=request)
            return None

        async def backoff(self, seconds: float) -> None:
            self.waits = (*self.waits, seconds)

    original: Final = lens()
    await LensRepository(store).create(original)
    conflicts: Final = Conflicts()
    async with httpx.AsyncClient(
        base_url=store.client.base_url,
        params=store.client.params,
        transport=ObservedTransport(ReadMeter(), conflicts.respond),
        timeout=30,
    ) as client:
        updated: Final = await LensRepository(ClickHouseState(client), sleep=conflicts.backoff).update(
            original.id,
            lambda value: value.model_copy(update={"settings": value.settings.model_copy(update={"name": "Renamed"})}),
        )
    assert len(conflicts.waits) == losses
    assert all(wait >= 0 for wait in conflicts.waits)
    if losses == UPDATE_ATTEMPTS:
        assert updated is None
        assert conflicts.writes == UPDATE_ATTEMPTS
        assert await LensRepository(store).get(original.id) == original
    else:
        assert updated is not None and updated.settings.name == "Renamed"
        assert conflicts.writes == losses + 1
        assert await LensRepository(store).get(original.id) == updated
