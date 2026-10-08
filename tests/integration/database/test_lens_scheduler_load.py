import asyncio
import sys
from datetime import UTC, datetime, timedelta
from time import perf_counter
from typing import Final
from uuid import uuid4

import httpx
import pytest

from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.endpoints import claim_due
from litellm_lens.models import Claim, Evidence, Finding, Lens, LensSettings, Scope, Worker
from litellm_lens.repository import LensRepository
from litellm_lens.state import current_job
from tests.integration.state_transport import ObservedTransport, ReadMeter


def _large_lens(lens_id: str, scope: Scope, now: datetime, next_run_at: datetime) -> Lens:
    findings: Final = tuple(
        Finding(
            id=f"f{index}",
            title=f"Issue {index}",
            description="Repeated operation returns an unexpected result.",
            check_id="behavior",
            evidence=(Evidence(execution_id=f"t{index}", span_id=f"s{index}", quote="Unexpected result"),),
            first_seen=now,
            last_seen=now,
            revision=1,
        )
        for index in range(100)
    )
    return Lens(
        id=lens_id,
        scope=scope,
        settings=LensSettings(
            name="Claim scheduler load", model="analysis", context="Find unexpected behavior", enabled=True
        ),
        created_at=now,
        next_run_at=next_run_at,
        findings=findings,
        budget_month=now.strftime("%Y-%m"),
    )


def _due_lens(lens_id: str, scope: Scope, now: datetime, model: str, next_run_at: datetime) -> Lens:
    return Lens(
        id=lens_id,
        scope=scope,
        settings=LensSettings(name="Claim paging test", model=model, context="Find unexpected behavior", enabled=True),
        created_at=now,
        next_run_at=next_run_at,
        budget_month=now.strftime("%Y-%m"),
    )


async def _supports_model(_worker: Worker, _settings: LensSettings) -> bool:
    return True


async def _supports_supported_model(_worker: Worker, settings: LensSettings) -> bool:
    return settings.model == "supported"


@pytest.mark.asyncio
async def test_claim_due_reaches_a_supported_lens_behind_a_full_page_of_unsupported_ones(
    store: ClickHouseState,
) -> None:
    now: Final = datetime.now(UTC).replace(microsecond=0)
    scope: Final = Scope(team_id=uuid4().hex)
    worker: Final = Worker(id=uuid4().hex, name="paging-test-worker", scope=scope, last_seen=now)
    unsupported_at: Final = now - timedelta(minutes=5)
    supported_at: Final = now - timedelta(minutes=1)
    unsupported: Final = tuple(_due_lens(uuid4().hex, scope, now, "unsupported", unsupported_at) for _ in range(25))
    supported: Final = _due_lens(uuid4().hex, scope, now, "supported", supported_at)
    candidates: Final = (*unsupported, supported)
    repository: Final = LensRepository(store)
    await asyncio.gather(*(repository.create(candidate) for candidate in candidates))
    claim: Final = await claim_due(worker, now, repository, _supports_supported_model)
    assert claim is not None
    assert claim.lens_id == supported.id
    assert claim.job.status == "running"


@pytest.mark.asyncio
async def test_lens_claim_reads_scale_with_due_lenses_not_total_lenses(store: ClickHouseState) -> None:
    now: Final = datetime.now(UTC).replace(microsecond=0)
    scope: Final = Scope(team_id=uuid4().hex)
    worker: Final = Worker(id=uuid4().hex, name="load-test-worker", scope=scope, last_seen=now)
    due_lens: Final = _large_lens(uuid4().hex, scope, now, now - timedelta(seconds=1))
    initial_future: Final = tuple(_large_lens(uuid4().hex, scope, now, now + timedelta(days=1)) for _ in range(20))
    additional_future: Final = tuple(_large_lens(uuid4().hex, scope, now, now + timedelta(days=1)) for _ in range(200))
    seed_repository: Final = LensRepository(store)
    await asyncio.gather(*(seed_repository.create(lens) for lens in (due_lens, *initial_future)))
    before_meter: Final = ReadMeter()
    before_claim, before_seconds = await _measured_claim(store, before_meter, worker, now)
    assert before_claim is not None
    assert before_claim.lens_id == due_lens.id
    assert before_claim.job.status == "running"
    claimed_lens: Final = await seed_repository.get(due_lens.id)
    assert claimed_lens is not None
    assert current_job(claimed_lens) == before_claim.job
    await seed_repository.update(
        due_lens.id,
        lambda lens: lens.model_copy(update={"jobs": (), "next_run_at": now - timedelta(seconds=1)}),
        attempts=1,
    )
    await asyncio.gather(*(seed_repository.create(lens) for lens in additional_future))
    after_meter: Final = ReadMeter()
    after_claim, after_seconds = await _measured_claim(store, after_meter, worker, now)
    assert after_claim is not None
    assert after_claim.lens_id == due_lens.id
    assert after_claim.job.status == "running"
    sys.stdout.write(
        f"claim read: before={before_meter.total_bytes} bytes, {before_seconds:.4f}s; "
        f"after={after_meter.total_bytes} bytes, {after_seconds:.4f}s\n"
    )
    assert before_meter.document_count == after_meter.document_count == 2
    assert before_meter.total_bytes == after_meter.total_bytes


async def _measured_claim(
    store: ClickHouseState, meter: ReadMeter, worker: Worker, now: datetime
) -> tuple[Claim | None, float]:
    async with httpx.AsyncClient(
        base_url=store.client.base_url, params=store.client.params, transport=ObservedTransport(meter), timeout=30
    ) as client:
        started: Final = perf_counter()
        claimed: Final = await claim_due(worker, now, LensRepository(ClickHouseState(client)), _supports_model)
        return claimed, perf_counter() - started
