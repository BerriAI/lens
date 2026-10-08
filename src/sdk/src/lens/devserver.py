import os
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Annotated, Final
from uuid import uuid4

from fastapi import Depends, FastAPI, Header, HTTPException, Request, Response
from fastapi.responses import JSONResponse

from ._contract import CaseDiff, CaseResult, CreateEvalRun, EvalRun, GateResult, ResolvedDataset, Summary, scorer_names
from .models import CaseSource, DatasetCase, DatasetMessage, EvalCases


@dataclass(frozen=True, slots=True)
class State:
    datasets: dict[str, EvalCases]
    names: dict[str, str]
    runs: dict[str, EvalRun] = field(default_factory=dict)
    specs: dict[str, CreateEvalRun] = field(default_factory=dict)
    results: dict[str, dict[tuple[str, int], CaseResult]] = field(default_factory=dict)
    keys: dict[str, tuple[str, str]] = field(default_factory=dict)
    verdicts: dict[str, dict[str, bool]] = field(default_factory=dict)


class Fault(Exception):
    def __init__(self, status: int, code: str) -> None:
        self.status = status
        self.code = code


def sample_cases(count: int = 36) -> EvalCases:
    return EvalCases(
        dataset_id="demo",
        revision=1,
        cases=tuple(
            DatasetCase(
                id=f"case-{index}",
                messages=(DatasetMessage(role="user", content=f"Case {index}"),),
                source=CaseSource(finding_id="1"),
                meta={"priority": "high" if index < 2 else "low"},
            )
            for index in range(count)
        ),
    )


def fake_score(result: CaseResult) -> bool:
    return result.trace is not None and "pass" in result.trace.value and result.error is None


def compute_summary(state: State, run_id: str, score: Callable[[CaseResult], bool]) -> Summary:
    spec: Final = state.specs[run_id]
    dataset: Final = state.datasets[spec.dataset_id]
    cases: Final = tuple(
        case for case in dataset.cases if case.included and (spec.case_ids is None or case.id in spec.case_ids)
    )
    results: Final = state.results[run_id]
    baseline_ids: Final = tuple(
        identity
        for identity, run in state.runs.items()
        if identity != run_id
        and run.status == "done"
        and run.branch == "main"
        and state.specs[identity].eval == spec.eval
        and state.specs[identity].agent == spec.agent
        and state.specs[identity].dataset_id == spec.dataset_id
        and state.specs[identity].revision == spec.revision
        and state.specs[identity].scorers == spec.scorers
        and state.specs[identity].case_ids == spec.case_ids
    )
    baseline_id: Final = baseline_ids[-1] if baseline_ids else None
    verdicts: Final = {
        case.id: sum(score(results[(case.id, trial)]) for trial in range(spec.trials) if (case.id, trial) in results)
        > spec.trials / 2
        for case in cases
    }
    prior: Final = state.verdicts[baseline_id] if baseline_id else {}
    state.verdicts[run_id] = verdicts
    passed: Final = sum(verdicts.values())
    errors: Final = sum(
        1
        for case in cases
        for trial in range(spec.trials)
        if (case.id, trial) not in results or results[(case.id, trial)].error is not None
    )
    trial_costs: Final = tuple(result.cost_usd or 0 for result in results.values())
    cost: Final = sum(trial_costs) / len(cases)
    scores: dict[str, float] = {
        name: sum(score(result) for result in results.values()) / (len(cases) * spec.trials)
        for name in scorer_names(tuple(spec.scorers))
    }

    def diff(case: DatasetCase) -> CaseDiff:
        return CaseDiff(
            case_id=case.id,
            title=case.id,
            critical=case.meta.get("priority") == "high",
            baseline_url=f"{state.runs[baseline_id].url if baseline_id else ''}?case={case.id}",
            candidate_url=f"{state.runs[run_id].url}?case={case.id}",
        )

    regressions: Final = tuple(diff(case) for case in cases if prior.get(case.id) is True and not verdicts[case.id])
    fixed: Final = tuple(diff(case) for case in cases if prior.get(case.id) is False and verdicts[case.id])
    critical: Final = sum(case.critical for case in regressions)
    conditions: Final = (
        (
            baseline_id is not None and spec.gate.regressions is not None and len(regressions) > spec.gate.regressions,
            f"{len(regressions)} regressions (max {spec.gate.regressions})",
        ),
        (
            baseline_id is not None and spec.gate.critical is not None and critical > spec.gate.critical,
            f"{critical} critical regressions (max {spec.gate.critical})",
        ),
        (spec.gate.pass_rate is not None and passed / len(cases) < spec.gate.pass_rate, "Pass rate below minimum"),
        (spec.gate.cost_per_case is not None and cost > spec.gate.cost_per_case, "Cost per case above maximum"),
    )
    failures: Final = tuple(reason for failed, reason in conditions if failed) + tuple(
        f"{name} below minimum {minimum}" for name, minimum in spec.gate.min.items() if scores.get(name, 0) < minimum
    )
    reasons: Final = failures + (() if baseline_id else (f"no baseline on main for rev {spec.revision}",))
    return Summary(
        passed=passed,
        total=len(cases),
        pass_rate=passed / len(cases),
        cost_per_case=cost,
        scores=scores,
        errors=errors,
        baseline_run_id=baseline_id,
        baseline_version=state.runs[baseline_id].version if baseline_id else None,
        regressions=regressions,
        fixed=fixed,
        gate=GateResult(passed=not failures, reasons=reasons),
    )


def create_app(
    dataset: EvalCases | None = None,
    *,
    dataset_name: str = "demo",
    api_key: str = "lens-dev",
    score: Callable[[CaseResult], bool] = fake_score,
) -> FastAPI:
    selected: Final = dataset or sample_cases()
    state: Final = State({selected.dataset_id: selected}, {dataset_name: selected.dataset_id})

    async def authorize(request: Request, x_lens_contract: Annotated[str | None, Header()] = None) -> None:
        if request.headers.get("authorization") != f"Bearer {api_key}":
            raise Fault(401, "unauthorized")
        if x_lens_contract != "1":
            raise Fault(409, "contract_version")

    app: Final = FastAPI(dependencies=[Depends(authorize)])
    app.state.store = state

    @app.exception_handler(Fault)
    async def handle_error(request: Request, error: Fault) -> JSONResponse:
        return JSONResponse({"detail": error.code, "code": error.code}, status_code=error.status)

    @app.get("/lens/datasets/resolve")
    async def resolve(name: str, revision: int | None = None) -> ResolvedDataset:
        if name not in state.names:
            raise Fault(404, "dataset_not_found")
        data: Final = state.datasets[state.names[name]]
        if revision is not None and data.revision != revision:
            raise Fault(404, "revision_not_found")
        return ResolvedDataset(id=data.dataset_id, name=name, revision=data.revision)

    @app.get("/lens/datasets/{dataset_id}/revisions/{revision}/cases")
    async def cases(dataset_id: str, revision: int) -> EvalCases:
        if dataset_id not in state.datasets:
            raise Fault(404, "dataset_not_found")
        data: Final = state.datasets[dataset_id]
        if data.revision != revision:
            raise Fault(404, "revision_not_found")
        return data

    @app.post("/lens/evals/runs", status_code=201)
    async def create(spec: CreateEvalRun, request: Request, idempotency_key: Annotated[str, Header()]) -> EvalRun:
        if idempotency_key in state.keys:
            body, existing_id = state.keys[idempotency_key]
            if body != spec.model_dump_json():
                raise HTTPException(409, "Idempotency key reused with a different body")
            return state.runs[existing_id]
        data: Final = await cases(spec.dataset_id, spec.revision)
        ids: Final = tuple(
            case.id for case in data.cases if case.included and (spec.case_ids is None or case.id in spec.case_ids)
        )
        if not ids or (spec.case_ids is not None and set(spec.case_ids) != set(ids)):
            raise Fault(422, "unknown_case")
        run_id: Final = uuid4().hex
        run: Final = EvalRun(
            id=run_id,
            status="running",
            eval=spec.eval,
            agent=spec.agent,
            version=spec.version,
            branch=spec.branch,
            pr=spec.pr,
            url=f"{str(request.base_url).rstrip('/')}/lens/evals/runs/{run_id}",
            expected_trials=len(ids) * spec.trials,
            received_trials=0,
        )
        state.runs[run_id] = run
        state.specs[run_id] = spec
        state.results[run_id] = {}
        state.keys[idempotency_key] = (spec.model_dump_json(), run_id)
        return run

    def lookup(run_id: str) -> EvalRun:
        if run_id not in state.runs:
            raise Fault(404, "run_not_found")
        return state.runs[run_id]

    @app.put("/lens/evals/runs/{run_id}/results/{case_id}/{trial}", status_code=204)
    async def result(run_id: str, case_id: str, trial: int, value: CaseResult) -> Response:
        run: Final = lookup(run_id)
        if run.status != "running":
            raise Fault(409, "run_closed")
        spec: Final = state.specs[run_id]
        valid: Final = {case.id for case in state.datasets[spec.dataset_id].cases if case.included}
        if case_id not in valid or (spec.case_ids is not None and case_id not in spec.case_ids):
            raise Fault(422, "unknown_case")
        if not 0 <= trial < spec.trials or (value.trace is None) == (value.error is None):
            raise HTTPException(422, "Expected a valid trial index and exactly one trace or error")
        state.results[run_id][(case_id, trial)] = value
        state.runs[run_id] = run.model_copy(update={"received_trials": len(state.results[run_id])})
        return Response(status_code=204)

    @app.post("/lens/evals/runs/{run_id}/finish", status_code=202)
    async def finish(run_id: str) -> EvalRun:
        run: Final = lookup(run_id)
        if run.status != "running":
            raise Fault(409, "run_closed")
        scoring: Final = run.model_copy(update={"status": "scoring"})
        state.runs[run_id] = run.model_copy(update={"status": "done", "summary": compute_summary(state, run_id, score)})
        return scoring

    @app.get("/lens/evals/runs/{run_id}")
    async def get(run_id: str, wait: int = 0) -> EvalRun:
        return lookup(run_id)

    @app.get("/lens/evals/runs")
    async def listing(eval: str = "", agent: str = "", branch: str = "", limit: int = 20) -> tuple[EvalRun, ...]:
        return tuple(
            run
            for run in reversed(tuple(state.runs.values()))
            if (not eval or run.eval == eval)
            and (not agent or run.agent == agent)
            and (not branch or run.branch == branch)
        )[: max(0, min(limit, 100))]

    return app


def serve(host: str, port: int, dataset_file: Path | None) -> None:
    import uvicorn

    from .errors import ConfigurationError

    if host not in {"127.0.0.1", "::1", "localhost"}:
        raise ConfigurationError("The development server binds only to loopback")
    data: Final = EvalCases.model_validate_json(dataset_file.read_text()) if dataset_file else None
    print("Development server: synthetic scoring only; never use its verdict as a production evaluation")
    uvicorn.run(create_app(data, api_key=os.environ.get("LENS_API_KEY", "lens-dev")), host=host, port=port)
