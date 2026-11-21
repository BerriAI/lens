from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class _TaskPayload(BaseModel):
    model_config = ConfigDict(frozen=True)
    id: str
    prompt: str
    expected: str | None = None
    metadata: dict[str, str] = Field(default_factory=dict)
    trial: int = Field(default=1, ge=1)
    suite_id: str | None = None


class _TargetPayload(BaseModel):
    model_config = ConfigDict(frozen=True)
    harness: str
    model: str | None = None
    label: str | None = None

    @property
    def key(self) -> str:
        if self.label is not None:
            return self.label
        return f"{self.harness}/{self.model}" if self.model is not None else self.harness


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
    cost: float | None = None
    judge_cost: float | None = None
    input_tokens: int | None = None
    output_tokens: int | None = None
    model_calls: int | None = None
    judge_input_tokens: int | None = None
    judge_output_tokens: int | None = None
    judge_model_calls: int | None = None
    stop_reason: str | None = None
    metrics_complete: bool = False
    duration_seconds: float | None = None
    run_id: str | None = None
    check_detail: str | None = None
    check_error: str | None = None
    tool_calls: tuple[_ToolCallPayload, ...] | None = None
