from dataclasses import replace
from pathlib import Path
from typing import Final

import httpx
import pytest
from pydantic import SecretStr

from litellm_lens.application import create_app
from litellm_lens.auth import COOKIE
from litellm_lens.clickhouse_state import ClickHouseState
from litellm_lens.configuration import Settings
from litellm_lens.context import Runtime
from litellm_lens.release import PROTOCOL_VERSION
from litellm_lens.tracing.remote import LensConnection


@pytest.fixture
def settings(store: ClickHouseState, tmp_path: Path) -> Settings:
    page: Final = tmp_path / "index.html"
    page.write_text("<!doctype html><title>Lens integration shell</title>")
    return Settings(
        clickhouse_url=SecretStr(str(store.client.base_url)),
        database=store.client.params["database"],
        admin_token=SecretStr("integration-admin" * 3),
        service_token=SecretStr("integration-service" * 3),
        runtime_url="http://127.0.0.1:1",
        public_url="https://lens.test",
        ingestion_url="https://traces.lens.test",
        ui_directory=tmp_path,
    )


@pytest.mark.asyncio
async def test_standalone_sign_in_key_creation_and_dataset_routing_survive_api_restart(settings: Settings) -> None:
    app: Final = create_app(settings)
    async with app.router.lifespan_context(app):
        async with httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url=settings.public_url) as browser:
            assert (await browser.get("/")).headers["location"] == "/ui/"
            assert "Lens integration shell" in (await browser.get("/ui/")).text
            assert (await browser.get("/lens/datasets")).status_code == 401
            session: Final = await browser.post(
                "/auth/session", json={"token": settings.admin_token.get_secret_value()}
            )
            assert session.status_code == 200, session.text
            cookie: Final = browser.cookies[COOKIE]
            browser.headers["origin"] = settings.public_url
            key: Final = await browser.post("/lens/tracing/keys", json={"name": "Before restart"})
            assert key.status_code == 200, key.text
            assert key.json()["active"] is False
            dataset: Final = await browser.post("/lens/datasets", json={"name": "Before restart", "agent_name": "test"})
            assert dataset.status_code == 200, dataset.text
            listed: Final = await browser.get("/lens/datasets")
            assert listed.status_code == 200, listed.text
            assert [item["id"] for item in listed.json()] == [dataset.json()["id"]]
            connection: Final = await browser.get("/lens/service")
            assert connection.json()["url"] == settings.ingestion_url
            assert connection.json()["configured"] is True
            assert connection.json()["connected"] is False
    restarted: Final = create_app(settings)
    async with restarted.router.lifespan_context(restarted):
        async with httpx.AsyncClient(
            transport=httpx.ASGITransport(restarted),
            base_url=settings.public_url,
            headers={"cookie": f"{COOKIE}={cookie}", "origin": settings.public_url},
        ) as browser:
            assert (await browser.get("/auth/session")).json() == session.json()
            keys: Final = await browser.get("/lens/tracing/keys")
            assert keys.json() == [key.json()["record"]]
            saved: Final = await browser.get(f"/lens/datasets/{dataset.json()['id']}")
            assert saved.json() == dataset.json()
            assert (await browser.delete("/auth/session")).status_code == 204
            assert (await browser.get("/auth/session")).status_code == 401


@pytest.mark.asyncio
async def test_runtime_credentials_and_readiness_use_the_configured_connection(settings: Settings) -> None:
    app: Final = create_app(settings)
    async with app.router.lifespan_context(app):
        async with httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url=settings.public_url) as runtime:
            assert (await runtime.get("/health/live")).json() == {"status": "live"}
            unavailable: Final = await runtime.get("/health/ready")
            assert unavailable.status_code == 503, unavailable.text
            assert "runtime is not ready" in unavailable.json()["detail"]
            denied: Final = await runtime.get(
                "/lens/internal/ingestion-credentials", headers={"authorization": "Bearer wrong"}
            )
            assert denied.status_code == 401
            snapshot: Final = await runtime.get(
                "/lens/internal/ingestion-credentials",
                headers={"authorization": f"Bearer {settings.service_token.get_secret_value()}"},
            )
            assert snapshot.status_code == 200, snapshot.text
            assert snapshot.json()["keys"] == []
            assert snapshot.headers["cache-control"] == "no-store"


@pytest.mark.parametrize(
    ("storage_ready", "credentials_ready", "protocol", "release", "status"),
    (
        (False, True, PROTOCOL_VERSION, "startup-test", 503),
        (True, False, PROTOCOL_VERSION, "startup-test", 503),
        (True, True, PROTOCOL_VERSION + 1, "startup-test", 503),
        (True, True, PROTOCOL_VERSION, "different-release", 503),
        (True, True, PROTOCOL_VERSION, "startup-test", 200),
    ),
)
@pytest.mark.asyncio
async def test_ready_requires_storage_credentials_and_a_matching_runtime(
    settings: Settings,
    monkeypatch: pytest.MonkeyPatch,
    storage_ready: bool,
    credentials_ready: bool,
    protocol: int,
    release: str,
    status: int,
) -> None:
    monkeypatch.setenv("LITELLM_RELEASE_TAG", "startup-test")

    def status_response(request: httpx.Request) -> httpx.Response:
        assert request.url.path == "/internal/status"
        assert request.headers["authorization"] == f"Bearer {settings.service_token.get_secret_value()}"
        return httpx.Response(
            200,
            json={
                "storage_ready": storage_ready,
                "credentials_ready": credentials_ready,
                "protocol_version": protocol,
                "release": release,
            },
        )

    app: Final = create_app(settings)
    async with app.router.lifespan_context(app):
        async with httpx.AsyncClient(transport=httpx.MockTransport(status_response)) as runtime_client:
            runtime: Final[Runtime] = app.state.runtime
            app.state.runtime = replace(
                runtime,
                connection=LensConnection(
                    settings.runtime_url, settings.service_token.get_secret_value(), runtime_client
                ),
            )
            async with httpx.AsyncClient(transport=httpx.ASGITransport(app), base_url=settings.public_url) as client:
                response: Final = await client.get("/health/ready")
                assert response.status_code == status, response.text
