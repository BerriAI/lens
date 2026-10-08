import asyncio
import hashlib
import json
import random
import re
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Final, Literal, LiteralString

import httpx
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

from litellm_lens.state_schema import INDEX_SCHEMA


class Head(BaseModel):
    model_config = ConfigDict(frozen=True)
    key: str
    revision: int = 0
    digest: str = ""


class Blob(Head):
    data: str


class Snapshot(Head):
    value: JsonValue = None


@dataclass(frozen=True, slots=True)
class Change:
    previous: Snapshot
    value: JsonValue


@dataclass(frozen=True, slots=True)
class PreparedCommit:
    changes: tuple[Change, ...]
    blobs: tuple[Blob, ...]


@dataclass(frozen=True, slots=True)
class StorageFailure:
    kind: Literal["conflict", "exists", "unavailable", "invalid"]
    code: int | None = None


_JSON: Final = TypeAdapter(JsonValue)
_HEADS: Final = TypeAdapter(tuple[Head, ...])
_BLOBS: Final = TypeAdapter(tuple[Blob, ...])
_KEYS: Final = TypeAdapter(tuple[str, ...])
_EMPTY: Final[Mapping[str, str]] = MappingProxyType({})


def json_text(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def next_blob(change: Change) -> Blob:
    data: Final = json_text(change.value)
    return Blob(
        key=change.previous.key,
        revision=change.previous.revision + 1,
        digest=hashlib.sha256(data.encode()).hexdigest(),
        data=data,
    )


def response_failure(response: httpx.Response) -> StorageFailure:
    code_match: Final = re.match(r"Code: (\d+)\.", response.text)
    code: Final = int(code_match[1]) if code_match else None
    if code == 395 and "LENS_STATE_CONFLICT" in response.text:
        return StorageFailure("conflict", code)
    if code == 999 and "(node exists)" in response.text.lower():
        return StorageFailure("exists", code)
    if code == 999 and "bad version" in response.text.lower():
        return StorageFailure("conflict", code)
    return StorageFailure("unavailable", code)


@dataclass(frozen=True, slots=True)
class ClickHouseState:
    client: httpx.AsyncClient

    async def command(
        self, query: LiteralString, parameters: Mapping[str, str] = _EMPTY, body: str = ""
    ) -> str | StorageFailure:
        try:
            response: Final = await self.client.post(
                "/",
                params={
                    "query": query,
                    "keeper_map_strict_mode": "1",
                    "insert_keeper_max_retries": "0",
                    "async_insert": "0",
                    "wait_end_of_query": "1",
                    # ClickHouse decodes escaped text before interpreting String parameters as JSON.
                    **{f"param_{key}": value.replace("\\", "\\\\") for key, value in parameters.items()},
                },
                content=body.encode(),
            )
        except httpx.HTTPError:
            return StorageFailure("unavailable")
        return response.text if response.is_success else response_failure(response)

    async def initialize(self, keeper_path: str) -> StorageFailure | None:
        heads: Final = await self.command(
            "CREATE TABLE IF NOT EXISTS lens_state_heads "
            "(key String, revision UInt64, digest String) "
            "ENGINE=KeeperMap({keeper_path:String}) PRIMARY KEY key",
            MappingProxyType({"keeper_path": keeper_path}),
        )
        if isinstance(heads, StorageFailure):
            return heads
        blobs: Final = await self.command(
            "CREATE TABLE IF NOT EXISTS lens_state_blobs "
            "(key String, revision UInt64, digest FixedString(64), data String CODEC(ZSTD(3)), "
            "created_at DateTime64(3) DEFAULT now64(3)) "
            "ENGINE=ReplacingMergeTree ORDER BY (key, revision, digest) "
            "SETTINGS fsync_after_insert=1, fsync_part_directory=1"
        )
        if isinstance(blobs, StorageFailure):
            return blobs
        for statement in INDEX_SCHEMA:
            indexed: Final = await self.command(statement)
            if isinstance(indexed, StorageFailure):
                return indexed
        return None

    async def ensure_head(self, key: str) -> StorageFailure | None:
        result: Final = await self.command(
            "INSERT INTO lens_state_heads FORMAT JSONEachRow", body=Head(key=key).model_dump_json()
        )
        if isinstance(result, StorageFailure) and result.kind != "exists":
            return result
        return None

    async def heads(self, keys: tuple[str, ...]) -> tuple[Head, ...] | StorageFailure:
        result: Final = await self.command(
            "SELECT key, revision, digest FROM lens_state_heads "
            "WHERE key IN JSONExtract({keys:String}, 'Array(String)') ORDER BY key FORMAT JSONEachRow",
            MappingProxyType({"keys": json_text(keys)}),
        )
        if isinstance(result, StorageFailure):
            return result
        try:
            rows: Final = _HEADS.validate_python(tuple(json.loads(row) for row in result.splitlines()))
        except (ValidationError, ValueError):
            return StorageFailure("invalid")
        by_key: Final = MappingProxyType({row.key: row for row in rows})
        return tuple(by_key.get(key, Head(key=key)) for key in keys)

    async def read(self, key: str) -> Snapshot | StorageFailure:
        result: Final = await self.read_many((key,))
        return result if isinstance(result, StorageFailure) else result[0]

    async def keys(self, prefix: str, after: str = "", limit: int = 128) -> tuple[str, ...] | StorageFailure:
        result: Final = await self.command(
            "SELECT DISTINCT key FROM lens_state_blobs "
            "WHERE key >= {prefix:String} AND key < concat({prefix:String}, char(127)) AND key > {after:String} "
            "ORDER BY key LIMIT {limit:UInt32} FORMAT JSONEachRow",
            MappingProxyType({"prefix": prefix, "after": after, "limit": str(limit)}),
        )
        if isinstance(result, StorageFailure):
            return result
        try:
            return _KEYS.validate_python(tuple(json.loads(row)["key"] for row in result.splitlines()))
        except (ValidationError, ValueError, KeyError, TypeError):
            return StorageFailure("invalid")

    async def read_many(self, keys: tuple[str, ...]) -> tuple[Snapshot, ...] | StorageFailure:
        if not keys:
            return ()
        for attempt in range(8):
            before: Final = await self.heads(keys)
            if isinstance(before, StorageFailure):
                return before
            values: Final = await self._values(before)
            if isinstance(values, StorageFailure):
                return values
            if len(keys) == 1 and values is not None:
                return values
            after: Final = await self.heads(keys)
            if isinstance(after, StorageFailure):
                return after
            if before == after and values is not None:
                return values
            await asyncio.sleep(0.002 * (attempt + 1))
        return StorageFailure("unavailable")

    async def _values(self, heads: tuple[Head, ...]) -> tuple[Snapshot, ...] | StorageFailure | None:
        if not heads:
            return ()
        references: Final = tuple((head.key, head.revision, head.digest) for head in heads if head.digest)
        result: Final = await self.command(
            "SELECT key, revision, digest, data FROM lens_state_blobs FINAL "
            "WHERE (key, revision, digest) IN "
            "JSONExtract({references:String}, 'Array(Tuple(String, UInt64, String))') FORMAT JSONEachRow",
            MappingProxyType({"references": json_text(references)}),
        )
        if isinstance(result, StorageFailure):
            return result
        try:
            blobs: Final = _BLOBS.validate_python(tuple(json.loads(row) for row in result.splitlines()))
            values: Final = MappingProxyType(
                {
                    blob.key: Snapshot(**blob.model_dump(exclude={"data"}), value=_JSON.validate_json(blob.data))
                    for blob in blobs
                }
            )
        except (ValidationError, ValueError):
            return StorageFailure("invalid")
        if any(head.digest and head.key not in values for head in heads):
            return None
        return tuple(values.get(head.key, Snapshot(**head.model_dump())) for head in heads)

    async def resolve(self, heads: tuple[Head, ...]) -> tuple[Snapshot, ...] | StorageFailure:
        values: Final = await self._values(heads)
        return StorageFailure("unavailable") if values is None else values

    async def prepare(self, changes: tuple[Change, ...]) -> PreparedCommit | StorageFailure:
        keys: Final = tuple(change.previous.key for change in changes)
        if not changes or len(frozenset(keys)) != len(keys):
            return StorageFailure("invalid")
        ensured: Final = await asyncio.gather(*(self.ensure_head(key) for key in keys))
        if failure := next((result for result in ensured if isinstance(result, StorageFailure)), None):
            return failure
        blobs: Final = tuple(next_blob(change) for change in changes)
        result: Final = await self.command(
            "INSERT INTO lens_state_blobs FORMAT JSONEachRow",
            body="\n".join(blob.model_dump_json() for blob in blobs),
        )
        if isinstance(result, StorageFailure):
            return result
        return PreparedCommit(changes=changes, blobs=blobs)

    async def publish(self, prepared: PreparedCommit) -> StorageFailure | None:
        result: Final = await self.command(
            "ALTER TABLE lens_state_heads UPDATE "
            "revision=revision+1+throwIf(revision != "
            "JSONExtract({versions:String}, 'Map(String, UInt64)')[key], 'LENS_STATE_CONFLICT'), "
            "digest=JSONExtract({digests:String}, 'Map(String, String)')[key] "
            "WHERE key IN JSONExtract({keys:String}, 'Array(String)')",
            MappingProxyType(
                {
                    "keys": json_text(tuple(blob.key for blob in prepared.blobs)),
                    "versions": json_text(
                        {change.previous.key: change.previous.revision for change in prepared.changes}
                    ),
                    "digests": json_text({blob.key: blob.digest for blob in prepared.blobs}),
                }
            ),
        )
        return result if isinstance(result, StorageFailure) else None

    async def commit(self, changes: tuple[Change, ...]) -> StorageFailure | None:
        prepared: Final = await self.prepare(changes)
        return prepared if isinstance(prepared, StorageFailure) else await self.publish(prepared)

    async def update(
        self, key: str, transform: Callable[[JsonValue], JsonValue], attempts: int = 40
    ) -> Snapshot | StorageFailure:
        for attempt in range(attempts):
            previous: Final = await self.read(key)
            if isinstance(previous, StorageFailure):
                return previous
            value: Final = transform(previous.value)
            if value == previous.value:
                return previous
            change: Final = Change(previous, value)
            result: Final = await self.commit((change,))
            if result is None:
                blob: Final = next_blob(change)
                return Snapshot(key=blob.key, revision=blob.revision, digest=blob.digest, value=value)
            if result.kind != "conflict":
                return result
            await asyncio.sleep(random.uniform(0, 0.02 * min(attempt + 1, 8)))
        return StorageFailure("conflict")
