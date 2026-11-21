from __future__ import annotations

import hashlib
import importlib.util
import inspect
import sys
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType, ModuleType
from typing import TYPE_CHECKING, Final

from lens import _native
from lens.errors import ConfigurationError

from .models import Agent as Agent
from .models import AgentOutput, Check, Target, TaskInput, ToolCall, Verdict
from .payloads import _AgentResult, _Error, _Fail, _Pass, _TargetPayload, _TaskPayload, _ToolCallPayload

if TYPE_CHECKING:
    from litellm import EvalAgent, EvalTarget

    from .sdk import SDKConfig

__all__ = [
    "Agent",
    "AgentOutput",
    "Check",
    "Target",
    "TaskInput",
    "ToolCall",
    "Verdict",
    "LocalConfig",
    "attempt",
    "default_check",
    "load_config",
    "run",
]


def default_check(task: TaskInput, output: AgentOutput) -> Verdict:
    if task.expected is None:
        return Verdict(True)
    answer: Final = output.text.strip().casefold()
    expected: Final = task.expected.strip().casefold()
    return Verdict(answer == expected, "" if answer == expected else f"expected {task.expected!r}, got {output.text!r}")


@dataclass(frozen=True, slots=True)
class LocalConfig:
    path: Path
    targets: tuple[Target | EvalTarget | EvalAgent, ...]
    check: Check
    sdk: SDKConfig | None = None

    def target(self, harness: str, model: str | None, label: str | None = None) -> Target | EvalTarget | EvalAgent:
        found: Final = next(
            (
                target
                for target in self.targets
                if _target_payload(target) == _TargetPayload(harness=harness, model=model, label=label)
            ),
            None,
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
    try:
        spec.loader.exec_module(module)
    except ImportError as error:
        raise _import_failure(path, error) from error
    return module


def _import_failure(path: Path, error: ImportError) -> ConfigurationError:
    detail: Final = _native.redact(str(error))
    return ConfigurationError(
        f"Cannot import {path.name}: {detail}. Install the missing config dependency. "
        "SDK configs require Lens's agent-evals extra: uv sync --project src/sdk --extra agent-evals --frozen"
    )


def target_key(target: Target | EvalTarget | EvalAgent) -> str:
    return target.key if isinstance(target, Target) else target.name


def _targets(path: Path, declared: object, only: tuple[str, ...]) -> tuple[Target | EvalTarget | EvalAgent, ...]:
    if not isinstance(declared, list | tuple) or not declared:
        raise ConfigurationError(f"{path.name} must define targets = [Target(harness=..., model=..., run=...)]")
    if all(isinstance(item, Target) for item in declared):
        targets: tuple[Target | EvalTarget | EvalAgent, ...] = tuple(
            item for item in declared if isinstance(item, Target)
        )
    else:
        try:
            from .sdk import sdk_targets
        except ImportError as error:
            raise _import_failure(path, error) from error

        targets = sdk_targets(path, declared)
    keys: Final = [target_key(target) for target in targets]
    duplicates: Final = sorted({key for key in keys if keys.count(key) > 1})
    if duplicates:
        raise ConfigurationError(f"Duplicate targets in {path.name}: {', '.join(duplicates)}")
    unknown: Final = sorted(set(only) - set(keys))
    if unknown:
        raise ConfigurationError(f"--only {', '.join(unknown)} is not in {path.name}. Targets: {', '.join(keys)}")
    return tuple(target for target in targets if not only or target_key(target) in only)


def load_config(path: Path, only: tuple[str, ...] = ()) -> LocalConfig:
    if not path.is_file():
        raise ConfigurationError(f"Config file {path} does not exist")
    module: Final = _module(path)
    targets: Final = _targets(path, getattr(module, "targets", ()), only)
    declared_check: Final[object] = getattr(module, "check", default_check)
    if not callable(declared_check):
        raise ConfigurationError(f"check in {path.name} must be a function")
    if all(isinstance(target, Target) for target in targets):
        return LocalConfig(path=path, targets=targets, check=declared_check)
    from .sdk import SDKConfig

    return LocalConfig(path=path, targets=targets, check=declared_check, sdk=SDKConfig.load(module))


def _failure(error: BaseException) -> _AgentResult:
    return _AgentResult(outcome=_Error(kind=type(error).__name__, message=_native.redact(str(error))), tool_calls=())


async def _judge(check: Check, task: TaskInput, output: AgentOutput) -> Verdict:
    raw: Final = check(task, output)
    verdict: Final[object] = await raw if inspect.isawaitable(raw) else raw
    return verdict if isinstance(verdict, Verdict) else Verdict(bool(verdict), "" if verdict else "check failed")


async def attempt(config: LocalConfig, task_json: str, target_json: str) -> str:
    payload: Final = _TaskPayload.model_validate_json(task_json)
    chosen: Final = _TargetPayload.model_validate_json(target_json)
    if config.sdk is not None:
        from .sdk import sdk_attempt

        return await sdk_attempt(config.sdk, payload, config.target(chosen.harness, chosen.model, chosen.label))
    try:
        target: Final = config.target(chosen.harness, chosen.model, chosen.label)
        if not isinstance(target, Target):
            raise ConfigurationError("Cannot run SDK target as a callback")
        task: Final = TaskInput(
            id=payload.id,
            prompt=payload.prompt,
            expected=payload.expected,
            metadata=MappingProxyType(dict(payload.metadata)),
            harness=target.harness,
            model=target.model,
            trial=payload.trial,
        )
        output: Final = await target.run(task)
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
        metrics_complete=True,
    ).model_dump_json()


def _target_payload(target: Target | EvalTarget | EvalAgent) -> _TargetPayload:
    if isinstance(target, Target):
        return _TargetPayload(harness=target.harness, model=target.model)
    from .sdk import sdk_target_payload

    return sdk_target_payload(target)


def _targets_json(config: LocalConfig) -> str:
    return "[" + ",".join(_target_payload(target).model_dump_json(exclude_none=True) for target in config.targets) + "]"


async def run(
    command: str,
    config_path: Path,
    concurrency: int,
    fast: bool,
    tasks: str | None = None,
    resume: str | None = None,
    only: tuple[str, ...] = (),
    trials: int = 1,
) -> str:
    config: Final = load_config(config_path, only)

    if tasks is None and config.sdk is None and resume is None:
        raise ConfigurationError("Callback configs require a tasks CSV file")
    task_data: Final = config.sdk.task_data() if config.sdk is not None and config.sdk.tasks else None
    if tasks is not None and task_data is not None:
        raise ConfigurationError("Use a tasks CSV file or config-defined tasks, not both")

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
        task_data=task_data,
        trials=trials,
        config_hash=(
            config.sdk.fingerprint(config.targets, config_path.read_bytes())
            if config.sdk is not None
            else hashlib.sha256(config_path.read_bytes()).hexdigest()
        ),
    )
