import asyncio
import hashlib
import os
import re
import time
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass, replace
from datetime import timedelta
from functools import reduce
from itertools import product
from pathlib import Path
from typing import Final, Literal

import httpx

from ._contract import CaseError, CaseResult, CreateEvalRun, EvalRun, Gate, Scorer, TraceRef, scorer_names
from .client import Client
from .config import Execution, Settings
from .errors import ConfigurationError, InfrastructureError
from .models import Case, Report, Run, TrialResult, validate_gate

Task = Callable[[Case], Awaitable[Run]]


def redact(message: str) -> str:
    secrets: Final = tuple(
        value
        for key, value in os.environ.items()
        if len(value) >= 8 and any(word in key.upper() for word in ("KEY", "TOKEN", "SECRET", "PASSWORD"))
    )
    return reduce(lambda text, secret: text.replace(secret, "[redacted]"), secrets, message)[:2000]


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
        if re.fullmatch(r"[a-z0-9][a-z0-9_-]*", name) is None:
            raise ConfigurationError("Eval name must use lowercase letters, digits, hyphens or underscores")
        if not callable(task) or not data or not scores:
            raise ConfigurationError("An eval requires a task, dataset and at least one scorer")
        if baseline != "main":
            raise ConfigurationError(
                "Contract v1 supports only baseline='main'; other branches need a server contract update"
            )
        if not 1 <= trials <= 10 or concurrency < 1 or timeout_per_trial.total_seconds() <= 0:
            raise ConfigurationError("Trials must be 1..10, concurrency positive, and timeout positive")
        kinds: Final = scorer_names(tuple(scores))
        configured_gate: Final = threshold if threshold is not None else gate
        validate_gate(configured_gate)
        if set(configured_gate.min) - set(kinds):
            raise ConfigurationError("Gate.min must refer to a declared scorer")
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

    def subset(self, *, finding: int | str | None = None, case_ids: Sequence[str] | None = None) -> "Eval":
        if (finding is None) == (case_ids is None):
            raise ConfigurationError("Choose exactly one subset selector: finding or case_ids")
        selected: Final = tuple(sorted(set(case_ids))) if case_ids is not None else None
        if selected == ():
            raise ConfigurationError("A case subset cannot be empty")
        selector: Final = str(finding) if finding is not None else ",".join(selected or ())
        suffix: Final = hashlib.sha256(selector.encode()).hexdigest()[:10]
        return replace(
            self,
            name=f"{self.name}-subset-{suffix}",
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

    async def arun(
        self,
        *,
        settings: Settings | None = None,
        execution: Execution | None = None,
        client: Client | None = None,
    ) -> Report:
        configured: Final = settings or Settings.load(Path.cwd())
        context: Final = execution or Execution.local()
        if not configured.project:
            raise ConfigurationError("Set [tool.lens].project to the agent.name on your traces")
        if client is not None:
            return await self._execute(client, configured, context)
        key: Final = os.environ.get("LENS_API_KEY", "")
        if not key:
            raise ConfigurationError("Set LENS_API_KEY")
        async with httpx.AsyncClient(
            base_url=configured.endpoint(),
            headers={"Authorization": f"Bearer {key}"},
            timeout=httpx.Timeout(40, connect=10),
            follow_redirects=False,
        ) as http:
            return await self._execute(Client(http), configured, context)

    async def _execute(self, client: Client, settings: Settings, execution: Execution) -> Report:
        dataset: Final = await client.resolve(self.data)
        response: Final = await client.cases(dataset)
        candidates: Final = tuple(case for case in response.cases if case.included)
        if len({case.id for case in candidates}) != len(candidates):
            raise InfrastructureError("Lens returned duplicate dataset case IDs")
        selected: Final = tuple(
            case
            for case in candidates
            if (self.case_ids is None or case.id in self.case_ids)
            and (self.finding is None or case.source.finding_id == self.finding)
        )
        if self.case_ids is not None and set(self.case_ids) - {case.id for case in selected}:
            raise ConfigurationError("Subset references cases not included in this dataset revision")
        if not selected:
            raise ConfigurationError("The eval selected no cases")
        cases: Final = tuple(case.to_case() for case in selected)
        body: Final = CreateEvalRun(
            eval=self.name,
            agent=settings.project,
            dataset_id=dataset.id,
            revision=dataset.revision,
            case_ids=tuple(case.id for case in cases),
            version=execution.version,
            branch=execution.branch,
            pr=execution.pr,
            ci_url=execution.ci_url,
            trials=self.trials,
            scorers=self.scores,
            gate=self.threshold,
        )
        fingerprint: Final = hashlib.sha256(body.model_dump_json().encode()).hexdigest()
        key: Final = f"{self.name}:{execution.version}:{execution.identity}:{fingerprint}"
        run: Final = await client.create(body, key)
        if run.eval != self.name or run.version != execution.version or run.agent != settings.project:
            raise InfrastructureError("Lens created a run for a different evaluation")
        outcomes: Final = await self._run_trials(client, run.id, cases) if run.status == "running" else ()
        finished: Final = await client.finish(run.id) if run.status == "running" else run
        completed: Final = await self._wait(client, finished)
        baseline_id: Final = completed.summary.baseline_run_id if completed.summary is not None else None
        baseline: Final = await client.get(baseline_id) if baseline_id is not None else None
        report: Final = Report(completed, baseline, outcomes)
        report.summary
        return report

    async def _run_trials(self, client: Client, run_id: str, cases: tuple[Case, ...]) -> tuple[TrialResult, ...]:
        semaphore: Final = asyncio.Semaphore(self.concurrency)
        tasks: Final = tuple(
            asyncio.create_task(self._trial(client, run_id, case, trial, semaphore))
            for case, trial in product(cases, range(self.trials))
        )
        try:
            return tuple(await asyncio.gather(*tasks))
        except BaseException:
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            raise

    async def _trial(
        self, client: Client, run_id: str, case: Case, trial: int, semaphore: asyncio.Semaphore
    ) -> TrialResult:
        async with semaphore:
            start: Final = time.monotonic()
            try:
                async with asyncio.timeout(self.timeout_per_trial.total_seconds()):
                    value: Final = await self.task(case)
                if not isinstance(value, Run):
                    raise ConfigurationError("Task must return lens.Run")
                attribute: Literal["session.id", "trace_id"] = (
                    "session.id" if "session.id" in value.trace else "trace_id"
                )
                result = CaseResult(
                    trace=TraceRef(attribute=attribute, value=value.trace[attribute]),
                    cost_usd=value.cost_usd,
                    duration_ms=int((time.monotonic() - start) * 1000),
                )
            except Exception as error:
                result = CaseResult(
                    error=CaseError(type=type(error).__name__, message=redact(str(error))),
                    duration_ms=int((time.monotonic() - start) * 1000),
                )
            await client.result(run_id, case.id, trial, result)
            return TrialResult(case.id, trial, result)

    async def _wait(self, client: Client, run: EvalRun) -> EvalRun:
        deadline: Final = time.monotonic() + self.timeout_per_trial.total_seconds() + 60
        current = run  # rebind-ok: polling replaces the previous server snapshot
        while current.status not in {"done", "failed"}:
            remaining = deadline - time.monotonic()  # rebind-ok: each poll has a new remaining deadline
            if remaining <= 0:
                raise InfrastructureError("Timed out waiting for Lens to finish scoring")
            try:
                async with asyncio.timeout(remaining):
                    current = await client.get(run.id, wait=True)
            except TimeoutError as error:
                raise InfrastructureError("Timed out waiting for Lens to finish scoring") from error
            if current.status not in {"done", "failed"}:
                await asyncio.sleep(min(0.2, remaining))
        if current.status == "failed":
            raise InfrastructureError("Lens failed to score the run; see its run page")
        if current.summary is None:
            raise InfrastructureError("Lens completed the run without a summary")
        return current
