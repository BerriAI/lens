from __future__ import annotations

import hashlib
import inspect
import json
from collections.abc import Callable, Mapping
from dataclasses import dataclass, fields, is_dataclass
from enum import Enum
from pathlib import Path
from types import MappingProxyType, ModuleType
from typing import Final

from litellm import EvalRun, EvalScorecard, EvalStatus, EvalTarget, EvalTask, PredicateCheck, aagent_eval_attempt
from litellm.harness import CommandCheck, JudgeCheck, Result, ToolCall, ToolResult
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter

from lens import _native
from lens.errors import ConfigurationError

from .models import Target
from .payloads import _AgentResult, _Error, _Fail, _Pass, _TaskPayload, _ToolCallPayload

_MAPPING: Final[TypeAdapter[Mapping[str, object]]] = TypeAdapter(Mapping[str, object])
_OBJECT: Final[TypeAdapter[object]] = TypeAdapter(object)


class _RunOptions(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")
    max_turns: int | None = Field(default=None, ge=1)
    timeout: float = Field(default=300, gt=0)
    setup_timeout: float = Field(default=120, gt=0)
    api_base: str | None = None
    api_key: str | None = None


@dataclass(frozen=True, slots=True)
class SDKConfig:
    tasks: tuple[EvalTask, ...]
    task_factory: Callable[[Mapping[str, str | None]], EvalTask] | None
    options: _RunOptions

    @classmethod
    def load(cls, module: ModuleType) -> SDKConfig:
        declared: Final[object] = getattr(module, "tasks", ())
        if not isinstance(declared, list | tuple) or not all(isinstance(task, EvalTask) for task in declared):
            raise ConfigurationError("Config tasks must contain only litellm.EvalTask values")
        tasks: Final = tuple(task for task in declared if isinstance(task, EvalTask))
        if len({task.id for task in tasks}) != len(tasks):
            raise ConfigurationError("Config tasks must have unique ids")
        factory: Final[object] = getattr(module, "task", None)
        if factory is not None and not callable(factory):
            raise ConfigurationError("Config task must be a function accepting a CSV row")
        return cls(tasks, factory, _RunOptions.model_validate(getattr(module, "run_options", {})))

    def task(self, payload: _TaskPayload) -> EvalTask:
        if self.tasks:
            found: Final = next((task for task in self.tasks if task.id == payload.id), None)
            if found is None:
                raise ConfigurationError(f"Task {payload.id!r} is not in the config")
            return found
        if self.task_factory is not None:
            row: Final = MappingProxyType(
                {**payload.metadata, "id": payload.id, "prompt": payload.prompt, "expected": payload.expected}
            )
            task: Final = self.task_factory(row)
            if not isinstance(task, EvalTask):
                raise ConfigurationError("Config task(row) must return a litellm.EvalTask")
            if task.id != payload.id:
                raise ConfigurationError("Config task(row) must preserve the CSV task id")
            return task

        if payload.expected is None:
            raise ConfigurationError("SDK CSV rows need expected values or a task(row) factory with an explicit check")
        expected: Final = payload.expected.strip().casefold()

        def matches(result: Result) -> bool:
            return result.text.strip().casefold() == expected

        return EvalTask(id=payload.id, prompt=payload.prompt, check=PredicateCheck(matches))

    def fingerprint(self, targets: tuple[Target | EvalTarget, ...], source: bytes) -> str:
        identity: Final = {
            "source": hashlib.sha256(source).hexdigest(),
            "targets": tuple(_identity(target) for target in targets),
            "run_options": _identity(self.options.model_dump(exclude={"api_key"})),
        }
        return hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()

    def task_data(self) -> str:
        return json.dumps(
            tuple(
                {
                    "id": task.id,
                    "prompt": task.prompt,
                    "metadata": {"sdk_task_sha256": _task_hash(task)},
                }
                for task in self.tasks
            )
        )


def _callable_identity(value: Callable[..., object], ancestors: tuple[object, ...]) -> object:
    try:
        source: Final = inspect.getsource(value)
        closure: Final = _MAPPING.validate_python(inspect.getclosurevars(value).nonlocals)
        defaults: Final = {
            parameter.name: _OBJECT.validate_python(parameter.default)
            for parameter in inspect.signature(value).parameters.values()
            if parameter.default is not inspect.Parameter.empty
        }
        return {"source": source, "closure": _identity(closure, ancestors), "defaults": _identity(defaults, ancestors)}
    except (OSError, TypeError):
        return {"type": type(value).__module__ + "." + type(value).__qualname__}


def _identity(value: object, ancestors: tuple[object, ...] = ()) -> object:
    cycle: Final = next((index for index, ancestor in enumerate(ancestors) if ancestor is value), None)
    if cycle is not None:
        return {"cycle": cycle}
    parents: Final = (*ancestors, value)
    if isinstance(value, Mapping):
        return {str(key): _identity(item, parents) for key, item in value.items()}
    if is_dataclass(value) and not isinstance(value, type):
        return _identity(
            {field.name: _OBJECT.validate_python(getattr(value, field.name)) for field in fields(value)}, parents
        )
    if isinstance(value, tuple | list):
        return tuple(_identity(item, parents) for item in value)
    if isinstance(value, Enum):
        return _identity(value.value, parents)
    if callable(value):
        return _callable_identity(value, parents)
    if isinstance(value, bytes):
        return {"sha256": hashlib.sha256(value).hexdigest()}
    if isinstance(value, Path):
        return str(value)
    if value is None or isinstance(value, str | int | float | bool):
        return value
    return {"type": type(value).__module__ + "." + type(value).__qualname__}


def _check_identity(task: EvalTask) -> object:
    check: Final = task.check
    if isinstance(check, CommandCheck):
        return ("command", check.command, check.timeout)
    if isinstance(check, JudgeCheck):
        return ("judge", check.rubric, check.model, check.timeout)
    return ("predicate", _identity(check.predicate))


def _task_hash(task: EvalTask) -> str:
    files: Final = tuple(
        (name, hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest())
        for name, value in sorted(task.files.items())
    )
    identity: Final = (task.id, task.prompt, str(task.workspace), files, task.setup, _check_identity(task))
    return hashlib.sha256(json.dumps(identity).encode()).hexdigest()


def sdk_targets(path: Path, declared: list[object] | tuple[object, ...]) -> tuple[EvalTarget, ...]:
    targets: Final = tuple(target for target in declared if isinstance(target, EvalTarget))
    if len(targets) != len(declared):
        raise ConfigurationError(f"Every entry in {path.name} targets must be a lens.Target or litellm.EvalTarget")
    return targets


def _outcome(run: EvalRun) -> _Pass | _Fail | _Error:
    match run.status:
        case EvalStatus.PASS:
            return _Pass()
        case EvalStatus.FAIL:
            return _Fail(reason=_native.redact(run.detail or "check failed"))
        case EvalStatus.ERROR:
            return _Error(kind="AgentEvalError", message=_native.redact(run.detail or "agent evaluation failed"))


def _result(run: EvalRun) -> _AgentResult:
    result: Final = run.result
    check: Final = run.check
    summary: Final = EvalScorecard(run.run_id, (run,)).summaries[0]
    failed_tools: Final = (
        frozenset(event.id for event in result.events if isinstance(event, ToolResult) and event.is_error)
        if result is not None
        else frozenset()
    )
    calls: Final = (
        tuple(
            _ToolCallPayload(name=event.name, arguments=dict(event.input), error=event.id in failed_tools)
            for event in result.events
            if isinstance(event, ToolCall)
        )
        if result is not None
        else ()
    )
    return _AgentResult(
        outcome=_outcome(run),
        output=result.text if result is not None else "",
        cost=run.cost,
        judge_cost=check.judge_cost if check is not None else summary.judge_cost if result is not None else None,
        input_tokens=result.usage.input_tokens if result is not None else None,
        output_tokens=result.usage.output_tokens if result is not None else None,
        model_calls=result.usage.calls if result is not None else None,
        judge_input_tokens=(check.judge_usage.input_tokens if check is not None else 0 if result is not None else None),
        judge_output_tokens=(
            check.judge_usage.output_tokens if check is not None else 0 if result is not None else None
        ),
        judge_model_calls=check.judge_usage.calls if check is not None else 0 if result is not None else None,
        stop_reason=run.stop_reason,
        metrics_complete=summary.metrics_complete,
        duration_seconds=run.duration,
        run_id=run.run_id,
        check_detail=_native.redact(check.detail) if check is not None else None,
        check_error=_native.redact(check.error) if check is not None and check.error is not None else None,
        tool_calls=calls,
    )


async def sdk_attempt(config: SDKConfig, payload: _TaskPayload, target: Target | EvalTarget) -> str:
    try:
        if not isinstance(target, EvalTarget):
            raise ConfigurationError("SDK configs require litellm.EvalTarget values")
        run: Final = await aagent_eval_attempt(
            config.task(payload),
            target,
            trial=payload.trial,
            suite_id=payload.suite_id,
            max_turns=config.options.max_turns,
            timeout=config.options.timeout,
            setup_timeout=config.options.setup_timeout,
            api_base=config.options.api_base,
            api_key=config.options.api_key,
        )
        return _result(run).model_dump_json()
    except Exception as error:
        return _AgentResult(
            outcome=_Error(kind=type(error).__name__, message=_native.redact(str(error)))
        ).model_dump_json()
