import asyncio
from datetime import timedelta
from typing import Final

import pytest

from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.models import TraceIdentity
from litellm_lens.signal_repository import SignalRepository
from litellm_lens.signals import SIGNAL_RECLASSIFY_AFTER, SIGNAL_RETRY_FAILED_AFTER, SignalAttempt, SignalConfig
from tests.unit.litellm_lens.test_signals import NOW, execution


@pytest.mark.asyncio
async def test_signal_config_round_trips_through_a_new_repository(store: ClickHouseState) -> None:
    repository: Final = SignalRepository(store)
    assert await repository.get_config() == SignalConfig()
    configured: Final = SignalConfig(model="decision", threshold=0.7)
    await repository.save_config(configured)
    assert await SignalRepository(store).get_config() == configured


@pytest.mark.asyncio
async def test_concurrent_signal_claims_have_one_winner(store: ClickHouseState) -> None:
    repository: Final = SignalRepository(store)
    config: Final = SignalConfig(model="decision")
    claims: Final = await asyncio.gather(
        *(repository.claim(execution("trace"), config, NOW + timedelta(minutes=5), NOW) for _ in range(16))
    )
    assert claims.count(True) == 1
    saved: Final = await repository.traces((TraceIdentity(trace_id="trace"),))
    assert len(saved) == 1
    assert saved[0].data == {"status": "pending", "scores": {}, "model": config.model, "error": ""}


@pytest.mark.asyncio
async def test_signal_lease_must_be_strictly_expired_before_reclaiming(store: ClickHouseState) -> None:
    repository: Final = SignalRepository(store)
    config: Final = SignalConfig(model="decision")
    lease: Final = NOW + timedelta(minutes=5)
    assert await repository.claim(execution("trace"), config, lease, NOW)
    changed: Final = config.model_copy(update={"model": "new-model"})
    assert not await repository.claim(execution("trace"), changed, lease + timedelta(minutes=5), lease)
    assert await repository.claim(
        execution("trace"), changed, lease + timedelta(minutes=5), lease + timedelta(microseconds=1)
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("legacy_timestamps", (False, True))
async def test_expired_worker_cannot_overwrite_a_new_claim_or_completed_result(
    store: ClickHouseState, legacy_timestamps: bool
) -> None:
    repository: Final = SignalRepository(store)
    config: Final = SignalConfig(model="decision")
    run: Final = execution("trace")
    old_lease: Final = NOW + timedelta(minutes=5)
    new_start: Final = old_lease + timedelta(seconds=1)
    new_lease: Final = new_start + timedelta(minutes=5)
    assert await repository.claim(run, config, old_lease, NOW)
    assert await repository.claim(run, config, new_lease, new_start)
    current: Final = await repository.traces((TraceIdentity(trace_id="trace"),))
    stale: Final = SignalAttempt(status="failed", model=config.model, error="obsolete")
    await repository.store(run, config, old_lease, new_start, stale)
    assert await repository.traces((TraceIdentity(trace_id="trace"),)) == current
    wrong_config: Final = config.model_copy(update={"model": "other-model"})
    await repository.store(run, wrong_config, new_lease, new_start, stale)
    assert await repository.traces((TraceIdentity(trace_id="trace"),)) == current
    accepted: Final = SignalAttempt(status="classified", model=config.model, scores={"user_frustration": 0.8})
    stored_lease: Final = new_lease.replace(tzinfo=None) if legacy_timestamps else new_lease
    stored_time: Final = new_start.replace(tzinfo=None) if legacy_timestamps else new_start
    await repository.store(run, config, stored_lease, stored_time, accepted)
    completed: Final = await repository.traces((TraceIdentity(trace_id="trace"),))
    assert completed[0].data == accepted.model_dump(mode="json")
    assert completed[0].claimed_until is None
    assert completed[0].classified_at == new_start
    await repository.store(run, config, new_lease, new_start, stale)
    assert await repository.traces((TraceIdentity(trace_id="trace"),)) == completed


@pytest.mark.asyncio
@pytest.mark.parametrize("failed", (False, True))
async def test_signal_reclassification_and_failure_retry_keep_strict_interval_boundaries(
    store: ClickHouseState, failed: bool
) -> None:
    repository: Final = SignalRepository(store)
    config: Final = SignalConfig(model="decision")
    run: Final = execution("trace", span_count=2)
    period: Final = SIGNAL_RETRY_FAILED_AFTER if failed else SIGNAL_RECLASSIFY_AFTER
    classified_at: Final = NOW - period
    lease: Final = classified_at + timedelta(minutes=1)
    assert await repository.claim(run, config, lease, classified_at)
    await repository.store(
        run,
        config,
        lease,
        classified_at,
        SignalAttempt(status="failed" if failed else "classified", model=config.model),
    )
    next_run: Final = execution("trace", span_count=1 if failed else 3)
    assert not await repository.claim(next_run, config, NOW + timedelta(minutes=5), NOW)
    assert await repository.claim(next_run, config, NOW + timedelta(minutes=5), NOW + timedelta(microseconds=1))


@pytest.mark.asyncio
async def test_signal_results_keep_trace_references_distinct_and_deduplicate_requested_identities(
    store: ClickHouseState,
) -> None:
    repository: Final = SignalRepository(store)
    config: Final = SignalConfig(model="decision")
    first: Final = execution("trace").model_copy(update={"trace_ref": "one/quoted ' \\ λ"})
    second: Final = execution("trace").model_copy(update={"trace_ref": "two"})
    assert await repository.claim(first, config, NOW + timedelta(minutes=5), NOW)
    assert await repository.claim(second, config, NOW + timedelta(minutes=5), NOW)
    first_identity: Final = TraceIdentity(trace_id=first.trace_id, trace_ref=first.trace_ref)
    assert await repository.traces(()) == ()
    rows: Final = await repository.traces((first_identity, first_identity, TraceIdentity(trace_id="missing")))
    assert tuple(row.trace_ref for row in rows) == (first.trace_ref,)
    assert tuple(
        row.trace_ref for row in await repository.traces((TraceIdentity(trace_id="trace", trace_ref="two"),))
    ) == (second.trace_ref,)
