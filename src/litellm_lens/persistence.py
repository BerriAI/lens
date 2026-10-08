from collections.abc import AsyncIterator, Mapping
from datetime import UTC, datetime
from typing import Final, LiteralString, Never, TypeVar, assert_never
from urllib.parse import quote

from fastapi import HTTPException
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

from litellm_lens.clickhouse_state import ClickHouseState, Snapshot, StorageFailure, json_text

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_T = TypeVar("_T")
_Model = TypeVar("_Model", bound=BaseModel)


def document(model: BaseModel) -> JsonValue:
    return _JSON.validate_json(model.model_dump_json())


def record_key(namespace: str, *identity: str | int) -> str:
    return f"{namespace}/{quote(json_text(identity), safe='')}"


def utc_time(value: datetime) -> datetime:
    return value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)


def require_storage(result: _T | StorageFailure) -> _T:
    if not isinstance(result, StorageFailure):
        return result
    raise_storage(result)


async def select_rows(
    state: ClickHouseState, query: LiteralString, model: type[_Model], parameters: Mapping[str, str]
) -> tuple[_Model, ...]:
    result: Final = require_storage(await state.command(query, parameters))
    try:
        return tuple(model.model_validate_json(row) for row in result.splitlines())
    except ValidationError:
        raise_storage(StorageFailure("invalid"))


async def record_pages(state: ClickHouseState, prefix: str) -> AsyncIterator[tuple[Snapshot, ...]]:
    cursor = ""  # rebind-ok: advance the keyset cursor through bounded pages
    while keys := require_storage(await state.keys(prefix, after=cursor)):
        yield require_storage(await state.read_many(keys))
        cursor = keys[-1]


def raise_storage(failure: StorageFailure) -> Never:
    match failure.kind:
        case "conflict" | "exists":
            raise HTTPException(409, "Lens state changed; retry the operation")
        case "unavailable" | "invalid":
            raise HTTPException(503, "Lens storage is unavailable")
        case other:
            assert_never(other)
