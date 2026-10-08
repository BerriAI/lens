import os
from collections.abc import AsyncIterator
from typing import Final
from uuid import uuid4

import httpx
import pytest_asyncio

from litellm_lens.clickhouse_state import ClickHouseState


@pytest_asyncio.fixture
async def store() -> AsyncIterator[ClickHouseState]:
    database: Final = f"lens_state_test_{uuid4().hex}"
    async with httpx.AsyncClient(base_url=os.environ["CLICKHOUSE_STATE_TEST_URL"], timeout=30) as administration:
        result: Final = await administration.post(
            "/", params={"query": "CREATE DATABASE {name:Identifier}", "param_name": database}
        )
        result.raise_for_status()
        async with httpx.AsyncClient(
            base_url=os.environ["CLICKHOUSE_STATE_TEST_URL"], params={"database": database}, timeout=30
        ) as client:
            state: Final = ClickHouseState(client)
            assert await state.initialize(f"/state-tests/{database}") is None
            yield state
        removed: Final = await administration.post(
            "/", params={"query": "DROP DATABASE {name:Identifier} SYNC", "param_name": database}
        )
        removed.raise_for_status()
