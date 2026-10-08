import json
from collections.abc import AsyncIterator, Awaitable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from dataclasses import dataclass
from typing import Final, LiteralString, Protocol

import asyncpg


class Database(Protocol):
    def query_raw(self, query: LiteralString, *args: object) -> Awaitable[object]: ...
    def execute_raw(self, query: LiteralString, *args: object) -> Awaitable[int]: ...
    def transaction(self) -> AbstractAsyncContextManager["Database"]: ...


def encode_json(value: object) -> str:
    return value if isinstance(value, str) else json.dumps(value)


async def initialize_connection(connection: asyncpg.Connection) -> None:
    await connection.set_type_codec("json", schema="pg_catalog", encoder=encode_json, decoder=json.loads)
    await connection.set_type_codec("jsonb", schema="pg_catalog", encoder=encode_json, decoder=json.loads)


@dataclass(frozen=True, slots=True)
class PostgresDatabase:
    source: asyncpg.Pool | asyncpg.Connection

    @asynccontextmanager
    async def connection(self) -> AsyncIterator[asyncpg.Connection]:
        if isinstance(self.source, asyncpg.Connection):
            yield self.source
            return
        async with self.source.acquire(timeout=30) as connection:
            yield connection

    @asynccontextmanager
    async def transaction(self) -> AsyncIterator[Database]:
        async with self.connection() as connection:
            async with connection.transaction():
                yield PostgresDatabase(connection)

    async def query_raw(self, query: LiteralString, *args: object) -> object:
        async with self.connection() as connection:
            rows: Final = await connection.fetch(query, *args, timeout=30)
            return tuple(dict(row) for row in rows)

    async def execute_raw(self, query: LiteralString, *args: object) -> int:
        async with self.connection() as connection:
            status: Final = await connection.execute(query, *args, timeout=30)
            affected: Final = status.rsplit(" ", 1)[-1]
            return int(affected) if affected.isdigit() else 0


@asynccontextmanager
async def connect_database(url: str) -> AsyncIterator[PostgresDatabase]:
    async with asyncpg.create_pool(
        url, min_size=1, max_size=10, init=initialize_connection, command_timeout=30
    ) as pool:
        yield PostgresDatabase(pool)
