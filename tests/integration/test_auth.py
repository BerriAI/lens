from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from typing import Final

import httpx
import pytest
from fastapi import FastAPI, Request
from litellm.router import Router
from litellm.types.utils import ModelResponse
from pydantic import SecretStr

from litellm_lens.auth import COOKIE, router, token_hash
from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.context import Runtime, runtime_scope
from litellm_lens.identity import Identity
from litellm_lens.persistence import record_key, require_storage
from litellm_lens.trace.storage import ClickHouseStorage, TraceStorageConfig
from litellm_lens.tracing import TraceReceiver


class NoModelService:
    async def router(self) -> Router | None:
        pytest.fail("Session authentication must not call a model provider")

    async def validate_key(self, key_id: str | None) -> Identity | None:
        pytest.fail("Session authentication must not require analysis credentials")

    async def complete(
        self,
        key_id: str,
        data: dict[str, object],
        reserve: Callable[[], AbstractAsyncContextManager[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]:
        pytest.fail("Session authentication must not run inference")


@pytest.mark.asyncio
async def test_browser_sign_in_reload_csrf_protection_and_logout_use_clickhouse(store: ClickHouseState) -> None:
    app: Final = FastAPI()
    app.include_router(router)
    runtime: Final = Runtime(
        state=store,
        analysis=NoModelService(),
        tracing=TraceReceiver(ClickHouseStorage(TraceStorageConfig(url=str(store.client.base_url)))),
        admin_token=SecretStr("local-setup-token"),
        sql_signing_key=SecretStr("test-sql-key"),
        public_url="https://lens.test",
    )
    with runtime_scope(runtime):
        async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="https://lens.test") as browser:
            assert (await browser.get("/auth/session")).status_code == 401
            invalid: Final = await browser.post("/auth/session", json={"token": "wrong"})
            assert invalid.status_code == 401
            signed_in: Final = await browser.post("/auth/session", json={"token": "local-setup-token"})
            assert signed_in.status_code == 200, signed_in.text
            assert signed_in.json() == {"user_id": "lens-admin", "user_role": "proxy_admin"}
            cookie: Final = browser.cookies[COOKIE]
            assert all(flag in signed_in.headers["set-cookie"] for flag in ("HttpOnly", "Secure", "SameSite=strict"))
            assert (await browser.get("/auth/session")).json() == signed_in.json()
            stored: Final = require_storage(await store.read(record_key("session", token_hash(cookie))))
            assert stored.value is not None and cookie not in str(stored.value)
            cross_origin: Final = await browser.delete("/auth/session", headers={"origin": "https://other.test"})
            assert cross_origin.status_code == 403
            assert (await browser.get("/auth/session")).status_code == 200
            signed_out: Final = await browser.delete("/auth/session", headers={"origin": "https://lens.test"})
            assert signed_out.status_code == 204, signed_out.text
            assert (await browser.get("/auth/session")).status_code == 401
            replayed: Final = await browser.get("/auth/session", headers={"cookie": f"{COOKIE}={cookie}"})
            assert replayed.status_code == 401
