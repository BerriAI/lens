import json
from dataclasses import asdict, dataclass
from types import TracebackType
from typing import Final, Self

from pydantic import TypeAdapter

from . import _native
from ._contract import CaseError, CaseResult, EvalRun, TraceRef
from .config import Execution
from .errors import ConfigurationError
from .models import Case, Report


@dataclass(frozen=True, slots=True)
class TestCase(Case):
    trial: int = 0

    def _json(self) -> str:
        return json.dumps(
            {
                "id": self.id,
                "input": self.input,
                "followups": self.followups,
                "meta": dict(self.meta),
                "expected": self.expected,
                "trial": self.trial,
            }
        )


class Evaluation:
    def __init__(self, name: str, base_url: str, key: str, execution: Execution) -> None:
        self._native = _native.SavedEvaluation(name, base_url, key, json.dumps(asdict(execution)))
        self._cases = TypeAdapter(tuple[TestCase, ...]).validate_json(self._native.cases())
        self._report: Report | None = None
        self._entered = False

    @property
    def cases(self) -> tuple[TestCase, ...]:
        return self._cases

    @property
    def run(self) -> EvalRun:
        return EvalRun.model_validate_json(self._native.run())

    @property
    def report(self) -> Report | None:
        return self._report

    def __enter__(self) -> Self:
        if self._entered:
            raise ConfigurationError("An evaluation context cannot be entered more than once")
        self._entered = True
        return self

    def __exit__(
        self,
        exception_type: type[BaseException] | None,
        exception: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        if exception is None:
            self.assert_passed()
            return
        try:
            self._finish(CaseError(type=type(exception).__name__, message=_native.redact(str(exception))))
        except Exception as cleanup_error:
            exception.add_note("Lens could not finalize the run: " + _native.redact(str(cleanup_error)))

    def record(
        self,
        case: TestCase,
        *,
        output: str,
        trace_id: str | None = None,
        session_id: str | None = None,
        cost_usd: float | None = None,
        duration_ms: int | None = None,
    ) -> None:
        if trace_id is not None and session_id is not None:
            raise ConfigurationError("Provide one trace_id or session_id, not both")
        trace: Final = (
            TraceRef(attribute="trace_id", value=trace_id)
            if trace_id is not None
            else TraceRef(attribute="session.id", value=session_id)
            if session_id is not None
            else None
        )
        result: Final = CaseResult(trace=trace, output=output, cost_usd=cost_usd, duration_ms=duration_ms)
        self._native.record(case._json(), result.model_dump_json())

    def record_error(self, case: TestCase, error: Exception) -> None:
        result: Final = CaseResult(error=CaseError(type=type(error).__name__, message=_native.redact(str(error))))
        self._native.record(case._json(), result.model_dump_json(exclude={"output"}))

    def finish(self) -> Report:
        return self._finish(None)

    def _finish(self, error: CaseError | None) -> Report:
        payload: Final = error.model_dump_json() if error is not None else "null"
        self._report = TypeAdapter(Report).validate_json(self._native.finish(payload))
        return self._report

    def assert_passed(self) -> None:
        self.finish().assert_passed()
