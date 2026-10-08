from collections.abc import Callable
from contextlib import AbstractAsyncContextManager

from fastapi import Request
from litellm.types.utils import ModelResponse

from litellm_lens.context import current_runtime
from litellm_lens.identity import Identity


async def validate_key(key_id: str | None) -> Identity | None:
    return await current_runtime().analysis.validate_key(key_id)


async def complete(
    key_id: str, data: dict[str, object], reserve: Callable[[], AbstractAsyncContextManager[None]], incoming: Request
) -> tuple[ModelResponse, float | None]:
    return await current_runtime().analysis.complete(key_id, data, reserve, incoming)
