import asyncio
from collections.abc import AsyncIterator, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from dataclasses import replace
from typing import Final

import httpx
from fastapi import FastAPI, HTTPException, Request, Response
from fastapi.responses import RedirectResponse
from fastapi.staticfiles import StaticFiles
from litellm.router import Router
from litellm.types.utils import ModelResponse
from starlette.middleware.base import RequestResponseEndpoint

from litellm_lens import auth, dataset_endpoints, endpoints, feedback_endpoints, tracing_endpoints
from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.configuration import Settings
from litellm_lens.context import AnalysisAccess, Runtime, current_runtime, runtime_scope
from litellm_lens.identity import Identity
from litellm_lens.persistence import require_storage
from litellm_lens.release import PROTOCOL_VERSION
from litellm_lens.signal_repository import SignalRepository
from litellm_lens.signals import (
    SIGNAL_BACKLOG_SWEEP,
    SIGNAL_LIVE_SWEEP,
    DecisionsCall,
    SignalCompletion,
    run_signal_loop,
)
from litellm_lens.trace.storage import ClickHouseStorage
from litellm_lens.tracing import TraceReceiver
from litellm_lens.tracing.remote import LensConnection, RemoteTraceStore


class UnconfiguredAnalysis:
    async def router(self) -> Router | None:
        return None

    async def validate_key(self, key_id: str | None) -> Identity | None:
        raise HTTPException(400, "Configure an analysis provider before enabling investigations")

    async def complete(
        self,
        key_id: str,
        data: dict[str, object],
        reserve: Callable[[], AbstractAsyncContextManager[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]:
        raise HTTPException(400, "Configure an analysis provider before running investigations")


async def bind_runtime(request: Request, call_next: RequestResponseEndpoint) -> Response:
    runtime: Final[Runtime] = request.app.state.runtime
    request.state.tracing_receiver = runtime.tracing
    with runtime_scope(runtime):
        return await call_next(request)


async def resolve_decisions() -> DecisionsCall | None:
    router: Final = await current_runtime().analysis.router()
    return router.adecisions if router is not None else None


async def readiness() -> dict[str, str]:
    _ = require_storage(await current_runtime().state.read("health"))
    connection: Final = await endpoints.service_connection(auth.local_admin())
    if not connection.connected or not connection.status.storage_ready or not connection.status.credentials_ready:
        raise HTTPException(503, "Lens runtime is not ready; check its storage and API connection")
    if (
        connection.status.protocol_version != PROTOCOL_VERSION
        or not connection.release
        or connection.status.release != connection.release
    ):
        raise HTTPException(503, "Lens API and runtime must use the same release and protocol")
    return {"status": "ready"}


def create_app(settings: Settings | None = None, analysis: AnalysisAccess | None = None) -> FastAPI:
    config: Final = settings or Settings.from_env()
    connection: Final = LensConnection(config.runtime_url, config.service_token.get_secret_value())

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        async with httpx.AsyncClient(base_url=config.clickhouse_url.get_secret_value(), timeout=30) as administration:
            require_storage(
                await ClickHouseState(administration).command(
                    "CREATE DATABASE IF NOT EXISTS {database:Identifier}", {"database": config.database}
                )
            )
        async with (
            httpx.AsyncClient(
                base_url=config.clickhouse_url.get_secret_value(), params={"database": config.database}, timeout=30
            ) as state_client,
            connection.lifespan_client() as runtime_client,
        ):
            state: Final = ClickHouseState(state_client)
            require_storage(await state.initialize(f"/lens/{config.database}"))
            runtime: Final = Runtime(
                state=state,
                analysis=analysis or UnconfiguredAnalysis(),
                tracing=TraceReceiver(ClickHouseStorage(RemoteTraceStore(runtime_client))),
                admin_token=config.admin_token,
                sql_signing_key=config.service_token,
                public_url=config.public_url,
                connection=replace(connection, client=runtime_client),
                ingestion_url=config.ingestion_url,
            )
            app.state.runtime = runtime
            with runtime_scope(runtime):
                tasks: Final = tuple(
                    asyncio.create_task(
                        run_signal_loop(
                            runtime.tracing.storage,
                            SignalRepository(state),
                            SignalCompletion(resolve_decisions),
                            router_ready=lambda: analysis is not None,
                            sweep=sweep,
                        ),
                        name=f"lens-signals-{name}",
                    )
                    for name, sweep in (("live", SIGNAL_LIVE_SWEEP), ("backlog", SIGNAL_BACKLOG_SWEEP))
                )
                try:
                    yield
                finally:
                    for task in tasks:
                        task.cancel()
                    await asyncio.gather(*tasks, return_exceptions=True)

    app: Final = FastAPI(title="Lens", lifespan=lifespan)
    app.middleware("http")(bind_runtime)
    app.include_router(auth.router)
    app.include_router(tracing_endpoints.router)
    app.include_router(dataset_endpoints.router)
    app.include_router(feedback_endpoints.router)
    app.include_router(endpoints.router)
    app.add_api_route("/health/live", lambda: {"status": "live"}, methods=["GET"])
    app.add_api_route("/health/ready", readiness, methods=["GET"])
    if config.ui_directory is not None:
        app.mount("/ui", StaticFiles(directory=config.ui_directory, html=True), name="ui")
        app.add_api_route("/", lambda: RedirectResponse("/ui/"), include_in_schema=False)
    return app
