"""Lens eval contract. Shared by the Lens proxy (Ishaan) and the litellm-lens SDK (Moe).

Lives at litellm/proxy/lens/eval_contract.py and is vendored into the SDK as
lens/_contract.py. Pydantic only, no other imports. Any change ships to both sides
in the same PR, and golden fixtures in tests/fixtures/lens_eval/ must still validate.
"""

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field

CONTRACT_VERSION = 1  # sent as the X-Lens-Contract header, Lens rejects unknown majors with 409


class Record(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")


# Scorers. Lens computes every score from the trace; the agent never returns one.


class TaskCompleted(Record):
    kind: Literal["task_completed"] = "task_completed"


class CalledBefore(Record):
    kind: Literal["called_before"] = "called_before"
    first: str = Field(min_length=1)  # tool name that must appear...
    then: str = Field(min_length=1)  # ...before every call to this one. Passes if `then` is never called.


class Judge(Record):
    kind: Literal["judge"] = "judge"
    prompt: str = Field(min_length=1)
    model: str = ""  # empty means the Lens default judge model


Scorer = Annotated[TaskCompleted | CalledBefore | Judge, Field(discriminator="kind")]


def scorer_name(s: TaskCompleted | CalledBefore | Judge) -> str:
    """Stable key for scores and Gate.min. With several judges, Lens keys them judge_1, judge_2 in order."""
    return s.kind


class Gate(Record):
    """Every set field must hold for the gate to pass. None means not checked."""

    regressions: int | None = 0
    critical: int | None = 0
    pass_rate: float | None = Field(default=None, ge=0, le=1)
    cost_per_case: float | None = Field(default=None, ge=0)
    min: dict[str, float] = {}  # scorer name -> minimum mean score, e.g. {"called_before": 0.95}


# Run lifecycle: POST runs -> PUT results (any order, idempotent) -> POST finish -> GET until done.


class CreateEvalRun(Record):
    eval: str = Field(min_length=1, pattern=r"^[a-z0-9][a-z0-9_-]*$")  # e.g. "moyai-regressions" or "finding_1"
    agent: str = Field(min_length=1)  # matches agent.name on the traces
    dataset_id: str
    revision: int = Field(ge=1)
    case_ids: tuple[str, ...] | None = None  # subset; None means every included case
    version: str = Field(min_length=1)  # git SHA, matches agent.version on the traces
    branch: str = Field(min_length=1)
    pr: int | None = None
    ci_url: str = ""
    trials: int = Field(default=1, ge=1, le=10)
    timeout_per_trial_ms: int = Field(default=1_200_000, gt=0, le=18_446_744_073_709_551_615)
    scorers: tuple[Scorer, ...] = Field(min_length=1)
    gate: Gate = Gate()


class TraceRef(Record):
    """How Lens finds the trace for one trial. Exactly one key, an OTel attribute on the root span."""

    attribute: Literal["session.id", "trace_id"] = "session.id"
    value: str = Field(min_length=1)


class CaseError(Record):
    type: str  # exception class name
    message: str = Field(max_length=2000)


class CaseResult(Record):
    """PUT /lens/evals/runs/{run_id}/results/{case_id}/{trial}. Exactly one of trace or error."""

    trace: TraceRef | None = None
    error: CaseError | None = None
    cost_usd: float | None = Field(default=None, ge=0)  # None means Lens sums cost from the trace
    duration_ms: int | None = Field(default=None, ge=0)


class CaseDiff(Record):
    case_id: str
    title: str
    critical: bool
    baseline_url: str  # Lens deep link to the baseline trial
    candidate_url: str


class GateResult(Record):
    passed: bool
    reasons: tuple[str, ...] = ()  # human readable, one per failed condition, e.g. "3 regressions (max 0)"


class Summary(Record):
    passed: int
    total: int
    pass_rate: float
    cost_per_case: float
    scores: dict[str, float]  # scorer name -> mean over cases
    errors: int  # trials that returned CaseError or whose trace never closed
    baseline_run_id: str | None  # None means no comparable main run yet: regressions is empty, gate.reasons says so
    baseline_version: str | None
    regressions: tuple[CaseDiff, ...]
    fixed: tuple[CaseDiff, ...]
    gate: GateResult


class EvalRun(Record):
    id: str
    status: Literal["running", "scoring", "done", "failed"]
    eval: str
    agent: str
    version: str
    branch: str
    pr: int | None
    ci_url: str = ""
    url: str  # Lens page for this run
    expected_trials: int  # len(cases) * trials
    received_trials: int
    summary: Summary | None = None  # set once status == "done"
    failure: str = ""  # set when status == "failed" (Lens-side problem, CLI exits 2)


class ResolvedDataset(Record):
    """GET /lens/datasets/resolve?name=moyai-regressions[&revision=7]"""

    id: str
    name: str
    revision: int


class ApiError(Record):
    detail: str
    code: Literal[
        "dataset_not_found",
        "revision_not_found",
        "run_not_found",
        "run_closed",  # result or finish after finish
        "unknown_case",
        "contract_version",
        "unauthorized",
    ]
