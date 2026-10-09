import asyncio
import json
import os
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import asdict, dataclass, replace
from datetime import timedelta
from pathlib import Path
from typing import Final, Literal

from pydantic import TypeAdapter

from . import _native
from ._contract import CaseError, CaseResult, Gate, Scorer, TraceRef
from .config import Execution, Settings
from .errors import ConfigurationError
from .models import Case, Report, Run

Task = Callable[[Case], Awaitable[Run]]
redact = _native.redact


@dataclass(frozen=True, slots=True, init=False)
class Eval:
    name: str
    task: Task
    data: str
    scores: tuple[Scorer, ...]
    baseline: str
    trials: int
    threshold: Gate
    concurrency: int
    timeout_per_trial: timedelta
    finding: str | None
    case_ids: tuple[str, ...] | None

    def __init__(
        self,
        name: str,
        *,
        task: Task,
        data: str,
        scores: Sequence[Scorer],
        baseline: str = "main",
        trials: int = 1,
        gate: Gate = Gate(),
        concurrency: int = 8,
        timeout_per_trial: timedelta = timedelta(minutes=20),
        finding: str | None = None,
        case_ids: tuple[str, ...] | None = None,
        threshold: Gate | None = None,
    ) -> None:
        if not callable(task) or not data or not scores:
            raise ConfigurationError("An eval requires a task, dataset and at least one scorer")
        configured_gate: Final = threshold if threshold is not None else gate
        for key, value in (
            ("name", name),
            ("task", task),
            ("data", data),
            ("scores", tuple(scores)),
            ("baseline", baseline),
            ("trials", trials),
            ("threshold", configured_gate.model_copy(deep=True)),
            ("concurrency", concurrency),
            ("timeout_per_trial", timeout_per_trial),
            ("finding", finding),
            ("case_ids", case_ids),
        ):
            object.__setattr__(self, key, value)
        _native.validate_eval(self._spec())

    def subset(self, *, finding: int | str | None = None, case_ids: Sequence[str] | None = None) -> "Eval":
        if (finding is None) == (case_ids is None):
            raise ConfigurationError("Choose exactly one subset selector: finding or case_ids")
        selected: Final = tuple(sorted(set(case_ids))) if case_ids is not None else None
        if selected == ():
            raise ConfigurationError("A case subset cannot be empty")
        return replace(
            self,
            name=_native.subset_name(self.name, str(finding) if finding is not None else None, selected),
            finding=str(finding) if finding is not None else None,
            case_ids=selected,
        )

    def gate(self, gate: Gate) -> "Eval":
        return replace(self, threshold=gate)

    def run(self, *, settings: Settings | None = None, execution: Execution | None = None) -> Report:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(self.arun(settings=settings, execution=execution))
        raise ConfigurationError("Use await eval.arun() inside an active event loop")

    def _spec(self) -> str:
        return json.dumps(
            {
                "name": self.name,
                "data": self.data,
                "scores": [score.model_dump(mode="json") for score in self.scores],
                "baseline": self.baseline,
                "trials": self.trials,
                "gate": self.threshold.model_dump(mode="json"),
                "concurrency": self.concurrency,
                "timeout_seconds": self.timeout_per_trial.total_seconds(),
                "finding": self.finding,
                "case_ids": self.case_ids,
            }
        )

    async def arun(self, *, settings: Settings | None = None, execution: Execution | None = None) -> Report:
        configured: Final = settings or Settings.load(Path.cwd())
        context: Final = execution or Execution.local()
        running: Final = set[asyncio.Task[Run]]()
        stopped: Final = asyncio.Event()

        async def invoke(payload: str) -> str:
            if stopped.is_set():
                return CaseResult(error=CaseError(type="CancelledError", message="Evaluation stopped")).model_dump_json(
                    exclude={"output"}
                )
            case: Final = TypeAdapter(Case).validate_json(payload)

            async def call() -> Run:
                return await self.task(case)

            child: Final = asyncio.create_task(call())
            running.add(child)
            try:
                value: Final = await asyncio.wait_for(child, timeout=self.timeout_per_trial.total_seconds())
                if not isinstance(value, Run):
                    raise ConfigurationError("Task must return lens.Run")
                attribute: Literal["session.id", "trace_id"] = (
                    "session.id" if "session.id" in value.trace else "trace_id"
                )
                return CaseResult(
                    trace=TraceRef(attribute=attribute, value=value.trace[attribute]), cost_usd=value.cost_usd
                ).model_dump_json(exclude={"output"})
            except Exception as error:
                return CaseResult(
                    error=CaseError(type=type(error).__name__, message=redact(str(error)))
                ).model_dump_json(exclude={"output"})
            finally:
                running.discard(child)

        try:
            result: Final = await _native.evaluate(
                self._spec(),
                configured.project,
                configured.endpoint(),
                os.environ.get("LENS_API_KEY", ""),
                json.dumps(asdict(context)),
                invoke,
            )
            return TypeAdapter(Report).validate_json(result)
        finally:
            stopped.set()
            pending: Final = tuple(running)
            for child in pending:
                child.cancel()
            await asyncio.gather(*pending, return_exceptions=True)
