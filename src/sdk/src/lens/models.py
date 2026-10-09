import json
import math
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from types import MappingProxyType
from typing import Final, Literal

from pydantic import BaseModel, ConfigDict, Field

from ._contract import CaseDiff, CaseResult, EvalRun, Gate, GateResult, Summary
from .errors import ConfigurationError, GateFailed, InfrastructureError


@dataclass(frozen=True, slots=True)
class Case:
    id: str
    input: str
    followups: tuple[str, ...] = ()
    meta: Mapping[str, str] = field(default_factory=dict)
    expected: str = ""

    def __post_init__(self) -> None:
        object.__setattr__(self, "meta", MappingProxyType(dict(self.meta)))
        object.__setattr__(self, "followups", tuple(self.followups))


@dataclass(frozen=True, slots=True)
class Run:
    trace: Mapping[str, str]
    cost_usd: float | None = None

    def __post_init__(self) -> None:
        if len(self.trace) != 1 or next(iter(self.trace)) not in {"session.id", "trace_id"}:
            raise ConfigurationError("Run.trace must contain exactly one of session.id or trace_id")
        if not all(isinstance(value, str) and value.strip() for value in self.trace.values()):
            raise ConfigurationError("Run.trace requires a nonempty string reference")
        if self.cost_usd is not None and (not math.isfinite(self.cost_usd) or self.cost_usd < 0):
            raise ConfigurationError("Run.cost_usd must be finite and nonnegative")
        object.__setattr__(self, "trace", MappingProxyType(dict(self.trace)))


class WireRecord(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")


class DatasetMessage(WireRecord):
    role: Literal["system", "user", "assistant", "tool"]
    content: str


class CaseSource(WireRecord):
    finding_id: str = ""
    trace_id: str = ""
    trace_ref: str = ""
    span_id: str = ""
    lens_id: str = ""


class DatasetCase(WireRecord):
    id: str = Field(min_length=1)
    messages: tuple[DatasetMessage, ...]
    expected: str = ""
    included: bool = True
    source: CaseSource = CaseSource()
    meta: dict[str, str] = Field(default_factory=dict)

    def to_case(self) -> Case:
        user_messages: Final = tuple(message.content for message in self.messages if message.role == "user")
        if not user_messages:
            raise ConfigurationError(f"Dataset case {self.id} has no user input")
        return Case(self.id, user_messages[0], user_messages[1:], self.meta, self.expected)


class EvalCases(WireRecord):
    dataset_id: str
    revision: int = Field(ge=1)
    cases: tuple[DatasetCase, ...]


class DatasetSummary(WireRecord):
    id: str
    name: str
    revision: int
    agent_name: str = ""


@dataclass(frozen=True, slots=True)
class TrialResult:
    case_id: str
    trial: int
    result: CaseResult


@dataclass(frozen=True, slots=True)
class Report:
    run: EvalRun
    baseline: EvalRun | None = None
    trials: tuple[TrialResult, ...] = ()

    @property
    def summary(self) -> Summary:
        if self.run.status != "done" or self.run.summary is None:
            raise InfrastructureError("Lens returned a run without a completed summary")
        return self.run.summary

    @property
    def url(self) -> str:
        return self.run.url

    @property
    def passed(self) -> int:
        return self.summary.passed

    @property
    def total(self) -> int:
        return self.summary.total

    @property
    def pass_rate(self) -> float:
        return self.summary.pass_rate

    @property
    def cost_per_case(self) -> float:
        return self.summary.cost_per_case

    @property
    def scores(self) -> Mapping[str, float]:
        return MappingProxyType(dict(self.summary.scores))

    @property
    def errors(self) -> int:
        return self.summary.errors

    @property
    def baseline_run_id(self) -> str | None:
        return self.summary.baseline_run_id

    @property
    def baseline_version(self) -> str | None:
        return self.summary.baseline_version

    @property
    def regressions(self) -> tuple[CaseDiff, ...]:
        return tuple(self.summary.regressions)

    @property
    def fixed(self) -> tuple[CaseDiff, ...]:
        return tuple(self.summary.fixed)

    @property
    def gate(self) -> GateResult:
        return self.summary.gate

    def assert_passed(self) -> None:
        if not self.summary.gate.passed:
            raise GateFailed("; ".join(self.summary.gate.reasons) or "Lens gate failed")

    def write_json(self, path: str | Path) -> None:
        Path(path).write_text(json.dumps({"runs": [self.run.model_dump(mode="json")]}), encoding="utf-8")


def validate_gate(gate: Gate) -> None:
    if any(value is not None and value < 0 for value in (gate.regressions, gate.critical)):
        raise ConfigurationError("Gate counts must be nonnegative")
    if any(not math.isfinite(value) or not 0 <= value <= 1 for value in gate.min.values()):
        raise ConfigurationError("Gate minimum scores must be finite and between 0 and 1")
    if gate.cost_per_case is not None and not math.isfinite(gate.cost_per_case):
        raise ConfigurationError("Gate cost must be finite")
