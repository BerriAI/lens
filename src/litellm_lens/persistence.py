from datetime import UTC, datetime
from typing import Final, Never, TypeVar, assert_never
from urllib.parse import quote

from fastapi import HTTPException
from pydantic import BaseModel, JsonValue, TypeAdapter

from litellm_lens.clickhouse_state import StorageFailure, json_text

_JSON: Final = TypeAdapter(JsonValue)
_T = TypeVar("_T")


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


def raise_storage(failure: StorageFailure) -> Never:
    match failure.kind:
        case "conflict" | "exists":
            raise HTTPException(409, "Lens state changed; retry the operation")
        case "unavailable" | "invalid":
            raise HTTPException(503, "Lens storage is unavailable")
        case other:
            assert_never(other)
