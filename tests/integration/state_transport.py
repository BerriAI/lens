from collections.abc import Awaitable, Callable
from typing import Final

import httpx

from litellm_lens.clickhouse_state import Blob


class ReadMeter:
    def __init__(self) -> None:
        self.batches: tuple[tuple[int, ...], ...] = ()

    def record(self, response: bytes) -> None:
        records: Final = tuple(Blob.model_validate_json(row) for row in response.splitlines())
        self.batches = (*self.batches, tuple(len(record.data.encode()) for record in records))

    @property
    def document_count(self) -> int:
        return sum(len(batch) for batch in self.batches)

    @property
    def total_bytes(self) -> int:
        return sum(sum(batch) for batch in self.batches)


class ObservedTransport(httpx.AsyncBaseTransport):
    def __init__(
        self, meter: ReadMeter, before: Callable[[httpx.Request], Awaitable[httpx.Response | None]] | None = None
    ) -> None:
        self.meter: Final = meter
        self.before: Final = before
        self.transport: Final = httpx.AsyncHTTPTransport()

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        if self.before is not None:
            intercepted: Final = await self.before(request)
            if intercepted is not None:
                return intercepted
        response: Final = await self.transport.handle_async_request(request)
        if request.url.params.get("query", "").startswith("SELECT key, revision, digest, data") and response.is_success:
            self.meter.record(await response.aread())
        return response

    async def aclose(self) -> None:
        await self.transport.aclose()


class PublicationGate:
    def __init__(self) -> None:
        import asyncio

        self.entered: Final = asyncio.Event()
        self.release: Final = asyncio.Event()

    async def before(self, request: httpx.Request) -> None:
        if request.url.params.get("query", "").startswith("ALTER TABLE lens_state_heads") and not self.entered.is_set():
            self.entered.set()
            await self.release.wait()
