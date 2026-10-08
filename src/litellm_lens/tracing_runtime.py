from typing import Final

from fastapi import HTTPException, Request
from pydantic import ConfigDict, TypeAdapter

from litellm_lens.trace.storage import ClickHouseStorage
from litellm_lens.tracing import TraceReceiver

_RECEIVER_ADAPTER: Final[TypeAdapter[TraceReceiver | None]] = TypeAdapter(
    TraceReceiver | None, config=ConfigDict(arbitrary_types_allowed=True)
)
_UNAVAILABLE_DETAIL: Final = "Agent tracing is not enabled. Configure the Lens service and LITELLM_LENS_URL."


def require_receiver(tracing: TraceReceiver | None) -> TraceReceiver:
    if tracing is None:
        raise HTTPException(status_code=501, detail=_UNAVAILABLE_DETAIL)
    return tracing


async def provide_receiver(request: Request) -> TraceReceiver | None:
    return _RECEIVER_ADAPTER.validate_python(getattr(request.state, "tracing_receiver", None))


async def provide_storage(request: Request) -> ClickHouseStorage | None:
    tracing: Final = await provide_receiver(request)
    return tracing.storage if tracing is not None else None
