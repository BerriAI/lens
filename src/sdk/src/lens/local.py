import importlib.util
import inspect
import sys
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from types import MappingProxyType, ModuleType
from typing import Final, Literal

from pydantic import BaseModel, ConfigDict, Field

from . import _native
from .errors import ConfigurationError


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


def default_check(task: TaskInput, output: AgentOutput) -> Verdict:
    if task.expected is None:
        return Verdict(True)
    answer: Final = output.text.strip().casefold()
    expected: Final = task.expected.strip().casefold()
    return Verdict(answer == expected, "" if answer == expected else f"expected {task.expected!r}, got {output.text!r}")


@dataclass(frozen=True, slots=True)
class LocalConfig:
    path: Path
    targets: tuple[Target, ...]
    check: Check

    def target(self, harness: str, model: str) -> Target:
        found: Final = next(
            (target for target in self.targets if target.harness == harness and target.model == model), None
        )
        if found is None:
            raise ConfigurationError(f"{harness}/{model} is not in {self.path.name} targets")
        return found


def _module(path: Path) -> ModuleType:
    spec: Final = importlib.util.spec_from_file_location(f"lens_config_{path.stem}", path)
    if spec is None or spec.loader is None:
        raise ConfigurationError(f"Cannot import {path}")
    module: Final = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    directory: Final = str(path.parent.resolve())
    if directory not in sys.path:
        sys.path.insert(0, directory)
    spec.loader.exec_module(module)
    return module


def _targets(path: Path, declared: object, only: tuple[str, ...]) -> tuple[Target, ...]:
    if not isinstance(declared, list | tuple) or not declared:
        raise ConfigurationError(f"{path.name} must define targets = [Target(harness=..., model=..., run=...)]")
    targets: Final = tuple(item for item in declared if isinstance(item, Target))
    if len(targets) != len(declared):
        raise ConfigurationError(f"Every entry in {path.name} targets must be a lens.Target")
    keys: Final = [target.key for target in targets]
    duplicates: Final = sorted({key for key in keys if keys.count(key) > 1})
    if duplicates:
        raise ConfigurationError(f"Duplicate targets in {path.name}: {', '.join(duplicates)}")
    unknown: Final = sorted(set(only) - set(keys))
    if unknown:
        raise ConfigurationError(f"--only {', '.join(unknown)} is not in {path.name}. Targets: {', '.join(keys)}")
    return tuple(target for target in targets if not only or target.key in only)


def load_config(path: Path, only: tuple[str, ...] = ()) -> LocalConfig:
    if not path.is_file():
        raise ConfigurationError(f"Config file {path} does not exist")
    module: Final = _module(path)
    targets: Final = _targets(path, getattr(module, "targets", ()), only)
    declared_check: Final[object] = getattr(module, "check", default_check)
    if not callable(declared_check):
        raise ConfigurationError(f"check in {path.name} must be a function")
    return LocalConfig(path=path, targets=targets, check=declared_check)


class _TaskPayload(BaseModel):
    model_config = ConfigDict(frozen=True)
    id: str
    prompt: str
    expected: str | None = None
    metadata: dict[str, str] = Field(default_factory=dict)


class _TargetPayload(BaseModel):
    model_config = ConfigDict(frozen=True)
    harness: str
    model: str


class _Pass(BaseModel):
    model_config = ConfigDict(frozen=True)
    status: Literal["pass"] = "pass"


class _Fail(BaseModel):
    model_config = ConfigDict(frozen=True)
    status: Literal["fail"] = "fail"
    reason: str


class _Error(BaseModel):
    model_config = ConfigDict(frozen=True)
    status: Literal["error"] = "error"
    kind: str
    message: str


class _ToolCallPayload(BaseModel):
    model_config = ConfigDict(frozen=True)
    name: str
    arguments: dict[str, object]
    error: bool


class _AgentResult(BaseModel):
    model_config = ConfigDict(frozen=True)
    outcome: _Pass | _Fail | _Error
    output: str = ""
    cost: float = 0.0
    judge_cost: float = 0.0
    tool_calls: tuple[_ToolCallPayload, ...] = ()


def _failure(error: BaseException) -> _AgentResult:
    return _AgentResult(outcome=_Error(kind=type(error).__name__, message=_native.redact(str(error))))


async def _judge(check: Check, task: TaskInput, output: AgentOutput) -> Verdict:
    raw: Final = check(task, output)
    verdict: Final[object] = await raw if inspect.isawaitable(raw) else raw
    return verdict if isinstance(verdict, Verdict) else Verdict(bool(verdict), "" if verdict else "check failed")


async def attempt(config: LocalConfig, task_json: str, target_json: str) -> str:
    payload: Final = _TaskPayload.model_validate_json(task_json)
    chosen: Final = _TargetPayload.model_validate_json(target_json)
    task: Final = TaskInput(
        id=payload.id,
        prompt=payload.prompt,
        expected=payload.expected,
        metadata=MappingProxyType(dict(payload.metadata)),
        harness=chosen.harness,
        model=chosen.model,
    )
    try:
        output: Final = await config.target(chosen.harness, chosen.model).run(task)
        if not isinstance(output, AgentOutput):
            raise ConfigurationError(f"{chosen.harness}/{chosen.model} must return AgentOutput")
        verdict: Final = await _judge(config.check, task, output)
    except Exception as error:
        return _failure(error).model_dump_json()
    calls: Final = tuple(
        _ToolCallPayload(name=call.name, arguments=dict(call.arguments), error=call.error) for call in output.tool_calls
    )
    return _AgentResult(
        outcome=_Pass() if verdict.passed else _Fail(reason=verdict.reason or "check failed"),
        output=output.text,
        cost=output.cost,
        judge_cost=output.judge_cost,
        tool_calls=calls,
    ).model_dump_json()


def _targets_json(config: LocalConfig) -> str:
    return (
        "["
        + ",".join(
            _TargetPayload(harness=target.harness, model=target.model).model_dump_json() for target in config.targets
        )
        + "]"
    )


async def run(
    command: str,
    config_path: Path,
    concurrency: int,
    fast: bool,
    tasks: str | None = None,
    resume: str | None = None,
    only: tuple[str, ...] = (),
) -> str:
    config: Final = load_config(config_path, only)

    async def invoke(task_json: str, target_json: str) -> str:
        return await attempt(config, task_json, target_json)

    return await _native.run_local(
        command,
        _targets_json(config),
        concurrency,
        fast,
        invoke,
        tasks=tasks,
        config=None if resume else str(config_path),
        resume=resume,
    )
