from collections.abc import Callable, Iterator
from contextlib import AbstractAsyncContextManager, contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
from typing import Final, Protocol

from fastapi import Request
from litellm.router import Router
from litellm.types.utils import ModelResponse
from pydantic import SecretStr

from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.identity import Identity
from litellm_lens.tracing import TraceReceiver
from litellm_lens.tracing.remote import LensConnection


class AnalysisAccess(Protocol):
    async def router(self) -> Router | None: ...

    async def validate_key(self, key_id: str | None) -> Identity | None: ...

    async def complete(
        self,
        key_id: str,
        data: dict[str, object],
        reserve: Callable[[], AbstractAsyncContextManager[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]: ...


@dataclass(frozen=True, slots=True)
class Runtime:
    state: ClickHouseState
    analysis: AnalysisAccess
    tracing: TraceReceiver
    admin_token: SecretStr
    sql_signing_key: SecretStr
    public_url: str
    gateway_secret: SecretStr | None = None
    connection: LensConnection | None = None
    ingestion_url: str | None = None


_CURRENT: Final[ContextVar[Runtime]] = ContextVar("lens_runtime")


def current_runtime() -> Runtime:
    return _CURRENT.get()


def current_connection() -> LensConnection:
    return current_runtime().connection or LensConnection.from_env()


@contextmanager
def runtime_scope(runtime: Runtime) -> Iterator[None]:
    token: Final = _CURRENT.set(runtime)
    try:
        yield
    finally:
        _CURRENT.reset(token)
