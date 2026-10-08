from collections.abc import Awaitable
from contextlib import AbstractAsyncContextManager
from datetime import datetime
from typing import LiteralString, Protocol

from pydantic import JsonValue

from litellm_lens.models import Record


class Database(Protocol):
    def query_raw(self, query: LiteralString, *args: object) -> Awaitable[object]: ...
    def execute_raw(self, query: LiteralString, *args: object) -> Awaitable[int]: ...
    def transaction(self) -> AbstractAsyncContextManager["Database"]: ...


class Row(Record):
    data: JsonValue
    due_at: datetime | None = None
