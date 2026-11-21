from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, field
from types import MappingProxyType


@dataclass(frozen=True, slots=True)
class ToolCall:
    name: str
    arguments: Mapping[str, object] = field(default_factory=lambda: MappingProxyType({}))
    error: bool = False


@dataclass(frozen=True, slots=True)
class AgentOutput:
    text: str
    cost: float = 0.0
    tool_calls: tuple[ToolCall, ...] = ()
    judge_cost: float = 0.0


@dataclass(frozen=True, slots=True)
class TaskInput:
    id: str
    prompt: str
    expected: str | None
    metadata: Mapping[str, str]
    harness: str
    model: str


@dataclass(frozen=True, slots=True)
class Verdict:
    passed: bool
    reason: str = ""


Agent = Callable[[TaskInput], Awaitable[AgentOutput]]
Check = Callable[[TaskInput, AgentOutput], object]


@dataclass(frozen=True, slots=True)
class Target:
    harness: str
    model: str
    run: Agent

    @property
    def key(self) -> str:
        return f"{self.harness}/{self.model}"
