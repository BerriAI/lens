import asyncio
import http.client
import time
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import suppress
from dataclasses import dataclass, field
from typing import Final
from urllib.parse import urlsplit
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field

from .errors import ConfigurationError, InfrastructureError
from .models import Case, Run

Task = Callable[[Case, str], Awaitable[Run]]


class Assignment(BaseModel):
    model_config = ConfigDict(frozen=True, populate_by_name=True)
    optimization_id: str | None = Field(default=None, alias="optimizationId")
    trace_id: str = Field(alias="traceId")
    model: str
    baseline: bool


class AssignmentRequest(BaseModel):
    model_config = ConfigDict(frozen=True)
    agent: str
    baseline: str
    traceId: str


class Trial(BaseModel):
    model_config = ConfigDict(frozen=True)
    id: str
    lease: str = Field(repr=False)
    model: str
    input: str
    expected: str
    requestId: str


class Submission(BaseModel):
    model_config = ConfigDict(frozen=True)
    id: str
    lease: str = Field(repr=False)
    traceId: str | None = None
    error: str | None = None


class Receipt(BaseModel):
    model_config = ConfigDict(frozen=True)
    accepted: bool


@dataclass(frozen=True, slots=True)
class Optimizations:
    base_url: str
    key: str = field(repr=False)

    def _post(self, path: str, body: BaseModel | None) -> bytes:
        endpoint: Final = urlsplit(self.base_url)
        if endpoint.scheme not in {"http", "https"} or not endpoint.hostname or endpoint.username or endpoint.password:
            raise ConfigurationError("Use a Lens HTTP origin without embedded credentials")
        if endpoint.query or endpoint.fragment:
            raise ConfigurationError("The Lens origin cannot contain a query or fragment")
        connection: Final = (
            http.client.HTTPSConnection(endpoint.hostname, endpoint.port, timeout=10)
            if endpoint.scheme == "https"
            else http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=10)
        )
        try:
            connection.request(
                "POST",
                f"{endpoint.path.rstrip('/')}/lens/optimizations/{path}",
                body.model_dump_json().encode() if body is not None else b"{}",
                {"Authorization": f"Bearer {self.key}", "Content-Type": "application/json"},
            )
            response: Final = connection.getresponse()
            content: Final = response.read(2_000_001)
            if response.status != 200 or len(content) > 2_000_000:
                raise InfrastructureError("Lens rejected the optimization request; inspect its UI status")
            return content
        except (OSError, http.client.HTTPException) as error:
            raise InfrastructureError("Lens could not be reached") from error
        finally:
            connection.close()

    def assign(self, *, agent: str, baseline: str, trace_id: str, fail_open: bool = True) -> Assignment:
        try:
            return Assignment.model_validate_json(
                self._post("assign", AssignmentRequest(agent=agent, baseline=baseline, traceId=trace_id))
            )
        except (InfrastructureError, ValueError):
            if not fail_open:
                raise
            return Assignment(traceId=trace_id, model=baseline, baseline=True)

    async def aassign(self, *, agent: str, baseline: str, trace_id: str, fail_open: bool = True) -> Assignment:
        return await asyncio.to_thread(
            self.assign, agent=agent, baseline=baseline, trace_id=trace_id, fail_open=fail_open
        )

    def run(self, optimization_id: str, *, task: Task) -> int:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(self.arun(optimization_id, task=task))
        raise ConfigurationError("Use await lens.optimizations.arun() inside an active event loop")

    async def arun(self, optimization_id: str, *, task: Task) -> int:
        identifier: Final = str(UUID(optimization_id))
        completed: Final = tuple([True async for _ in self._trials(identifier, task)])
        return len(completed)

    async def _trials(self, identifier: str, task: Task) -> AsyncIterator[None]:
        while await self._trial(identifier, task):
            yield None

    async def _trial(self, identifier: str, task: Task) -> bool:
        raw: Final = await asyncio.to_thread(self._post, f"{identifier}/claim", None)
        if raw.strip() == b"null":
            return False
        trial: Final = Trial.model_validate_json(raw)
        case: Final = Case(trial.id, trial.input, expected=trial.expected, meta={"request_id": trial.requestId})
        try:
            async with asyncio.timeout(480):
                result: Final = await task(case, trial.model)
            trace_id: Final = result.trace.get("trace_id")
            if not trace_id:
                raise ConfigurationError("Model optimizations require a trace_id in lens.Run")
            submission: Final = Submission(id=trial.id, lease=trial.lease, traceId=trace_id)
            deadline: Final = time.monotonic() + 90
            while time.monotonic() < deadline:
                if await self._submit(identifier, submission):
                    return True
                await asyncio.sleep(2)
            raise InfrastructureError("The completed agent trace did not arrive before the runner deadline")
        except Exception:
            with suppress(InfrastructureError):
                await asyncio.to_thread(
                    self._post, f"{identifier}/submit", Submission(id=trial.id, lease=trial.lease, error="agent_failed")
                )
            raise

    async def _submit(self, identifier: str, submission: Submission) -> bool:
        receipt: Final = Receipt.model_validate_json(
            await asyncio.to_thread(self._post, f"{identifier}/submit", submission)
        )
        return receipt.accepted
