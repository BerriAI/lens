import os
from collections.abc import AsyncIterator, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from datetime import UTC, datetime, timedelta
from typing import Final
from uuid import uuid4

import httpx
import uvicorn
from fastapi import FastAPI, Request, Response
from litellm.router import Router
from litellm.types.utils import ModelResponse
from pydantic import SecretStr
from starlette.middleware.base import RequestResponseEndpoint

from litellm_lens import auth
from litellm_lens.access_repository import AccessRepository
from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.context import Runtime, runtime_scope
from litellm_lens.dataset_endpoints import router as dataset_router
from litellm_lens.identity import Identity
from litellm_lens.trace.storage import ClickHouseStorage, TraceStorageConfig
from litellm_lens.tracing import TraceReceiver


class AnalysisStub:
    async def router(self) -> Router | None:
        return None

    async def validate_key(self, key_id: str | None) -> Identity | None:
        return None

    async def complete(
        self,
        key_id: str,
        data: dict[str, object],
        reserve: Callable[[], AbstractAsyncContextManager[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]:
        raise RuntimeError("Analysis is unavailable in the parity harness")


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    port: Final = int(os.environ.get("PARITY_PORT", "4100"))
    clickhouse_url: Final = os.environ.get("CH_URL", "http://127.0.0.1:18124")
    fresh_db: Final = f"lens_parity_{uuid4().hex}"
    async with httpx.AsyncClient(base_url=clickhouse_url, timeout=30) as administration:
        created: Final = await administration.post(
            "/", params={"query": "CREATE DATABASE {name:Identifier}", "param_name": fresh_db}
        )
        created.raise_for_status()
        try:
            async with httpx.AsyncClient(
                base_url=clickhouse_url, params={"database": fresh_db}, timeout=30
            ) as clickhouse_client:
                state: Final = ClickHouseState(clickhouse_client)
                failure: Final = await state.initialize(f"/parity/{fresh_db}")
                if failure is not None:
                    raise RuntimeError(f"ClickHouse state initialization failed: {failure.kind}")
                await AccessRepository(state).create_session(
                    auth.token_hash("parity-expired-session"), datetime.now(UTC) - timedelta(hours=1)
                )
                app.state.runtime = Runtime(
                    state=state,
                    analysis=AnalysisStub(),
                    tracing=TraceReceiver(ClickHouseStorage(TraceStorageConfig(url=clickhouse_url, database=fresh_db))),
                    admin_token=SecretStr("parity-admin-token"),
                    sql_signing_key=SecretStr("parity-sql-signing-key"),
                    public_url=f"http://127.0.0.1:{port}",
                    gateway_secret=SecretStr("parity-gateway-secret"),
                )
                yield
        finally:
            removed: Final = await administration.post(
                "/", params={"query": "DROP DATABASE {name:Identifier} SYNC", "param_name": fresh_db}
            )
            removed.raise_for_status()


async def bind_runtime(request: Request, call_next: RequestResponseEndpoint) -> Response:
    runtime: Final[Runtime] = request.app.state.runtime
    with runtime_scope(runtime):
        return await call_next(request)


def create_app() -> FastAPI:
    app = FastAPI(lifespan=lifespan)
    app.middleware("http")(bind_runtime)
    app.include_router(auth.router)
    app.include_router(dataset_router)
    return app


if __name__ == "__main__":
    port: Final = int(os.environ.get("PARITY_PORT", "4100"))
    uvicorn.run(create_app(), host="127.0.0.1", port=port, log_level="info")
