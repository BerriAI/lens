from collections.abc import Callable, Iterator
from contextlib import AbstractAsyncContextManager, contextmanager
from contextvars import ContextVar
from dataclasses import dataclass
from typing import Final, Protocol

from fastapi import Request
from litellm.router import Router
from litellm.types.utils import ModelResponse
from pydantic import SecretStr

from litellm_lens.identity import Identity
from litellm_lens.repository import Database
from litellm_lens.tracing import TraceReceiver


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
    database: Database
    analysis: AnalysisAccess
    tracing: TraceReceiver
    admin_token: SecretStr
    sql_signing_key: SecretStr
    public_url: str
    gateway_secret: SecretStr | None = None


_CURRENT: Final[ContextVar[Runtime]] = ContextVar("lens_runtime")


def current_runtime() -> Runtime:
    return _CURRENT.get()


@contextmanager
def runtime_scope(runtime: Runtime) -> Iterator[None]:
    token: Final = _CURRENT.set(runtime)
    try:
        yield
    finally:
        _CURRENT.reset(token)
