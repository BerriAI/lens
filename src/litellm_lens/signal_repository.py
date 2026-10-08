import asyncio
import random
from datetime import datetime
from typing import Final

from pydantic import JsonValue

from litellm_lens.clickhouse_state import Change, ClickHouseState
from litellm_lens.models import Execution, TraceIdentity
from litellm_lens.persistence import document, record_key, require_storage, utc_time
from litellm_lens.signals import (
    SIGNAL_RECLASSIFY_AFTER,
    SIGNAL_RETRY_FAILED_AFTER,
    SignalAttempt,
    SignalConfig,
    StoredTraceSignal,
)

_CONFIG_KEY: Final = "signals/config"


def claimable(execution: Execution, existing: StoredTraceSignal | None, config_key: str, now: datetime) -> bool:
    if existing is None:
        return True
    if existing.claimed_until is not None and existing.claimed_until >= now:
        return False
    if existing.config_key != config_key:
        return True
    status: Final = existing.data.get("status") if isinstance(existing.data, dict) else ""
    expired_pending: Final = status == "pending" and existing.claimed_until is not None and existing.claimed_until < now
    grew: Final = (
        execution.span_count > existing.span_count
        and existing.classified_at is not None
        and existing.classified_at < now - SIGNAL_RECLASSIFY_AFTER
    )
    retry: Final = (
        status == "failed"
        and existing.classified_at is not None
        and existing.classified_at < now - SIGNAL_RETRY_FAILED_AFTER
    )
    return expired_pending or grew or retry


class SignalRepository:
    def __init__(self, state: ClickHouseState) -> None:
        self.state: Final = state

    async def get_config(self) -> SignalConfig:
        record: Final = require_storage(await self.state.read(_CONFIG_KEY))
        return SignalConfig() if record.value is None else SignalConfig.model_validate(record.value)

    async def save_config(self, config: SignalConfig) -> None:
        require_storage(await self.state.update(_CONFIG_KEY, lambda _: document(config)))

    async def traces(self, identities: tuple[TraceIdentity, ...]) -> tuple[StoredTraceSignal, ...]:
        if not identities:
            return ()
        keys: Final = tuple(
            dict.fromkeys(record_key("trace-signal", trace.trace_id, trace.trace_ref) for trace in identities)
        )
        records: Final = require_storage(await self.state.read_many(keys))
        return tuple(StoredTraceSignal.model_validate(record.value) for record in records if record.value is not None)

    async def claim(
        self,
        execution: Execution,
        config: SignalConfig,
        claimed_until: datetime,
        now: datetime,
    ) -> bool:
        key: Final = record_key("trace-signal", execution.trace_id, execution.trace_ref)
        pending: Final = StoredTraceSignal(
            trace_id=execution.trace_id,
            trace_ref=execution.trace_ref,
            config_key=config.key(),
            span_count=execution.span_count,
            claimed_until=claimed_until,
            data={"status": "pending", "scores": {}, "model": config.model, "error": ""},
        )
        for attempt in range(40):
            record: Final = require_storage(await self.state.read(key))
            existing: Final = None if record.value is None else StoredTraceSignal.model_validate(record.value)
            if not claimable(execution, existing, config.key(), utc_time(now)):
                return False
            result: Final = await self.state.commit((Change(record, document(pending)),))
            if result is None:
                return True
            if result.kind != "conflict":
                require_storage(result)
            await asyncio.sleep(random.uniform(0, 0.02 * min(attempt + 1, 8)))
        return False

    async def store(
        self,
        execution: Execution,
        config: SignalConfig,
        claimed_until: datetime,
        classified_at: datetime,
        attempt: SignalAttempt,
    ) -> None:
        def complete(value: JsonValue) -> JsonValue:
            if value is None:
                return value
            existing: Final = StoredTraceSignal.model_validate(value)
            if existing.config_key != config.key() or existing.claimed_until != utc_time(claimed_until):
                return value
            return document(
                existing.model_copy(
                    update={
                        "classified_at": utc_time(classified_at),
                        "claimed_until": None,
                        "data": {
                            "status": attempt.status,
                            "scores": dict(attempt.scores),
                            "model": attempt.model,
                            "error": attempt.error,
                        },
                    }
                )
            )

        require_storage(
            await self.state.update(record_key("trace-signal", execution.trace_id, execution.trace_ref), complete)
        )
