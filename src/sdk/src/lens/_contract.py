from typing import Annotated, Final

from pydantic import Field

from ._generated import (
    AgentIo as AgentIo,
    AgentRequest as AgentRequest,
    ApiError as ApiError,
    CalledBefore as CalledBefore,
    CaseDiff as CaseDiff,
    CaseError as CaseError,
    CaseResult as CaseResult,
    CreateEvalRun as CreateEvalRun,
    EvalDefinition as EvalDefinition,
    EvalRun as EvalRun,
    EvalSpec as EvalSpec,
    Gate as Gate,
    GateResult as GateResult,
    ImmediateCompletion as ImmediateCompletion,
    InputBinding as InputBinding,
    Judge as Judge,
    JsonPointer as JsonPointer,
    OutputMapping as OutputMapping,
    PollCompletion as PollCompletion,
    RequiredItem as RequiredItem,
    ResolvedDataset as ResolvedDataset,
    Summary as Summary,
    TaskCompleted as TaskCompleted,
    TraceRef as TraceRef,
    TraceMapping as TraceMapping,
    TrialSteps as TrialSteps,
)

CONTRACT_VERSION: Final = 1
AGENT_IO_CONTRACT_VERSION: Final = 2
Completion = Annotated[ImmediateCompletion | PollCompletion, Field(discriminator="kind")]
Scorer = Annotated[TaskCompleted | CalledBefore | Judge, Field(discriminator="kind")]


def scorer_names(scorers: tuple[Scorer, ...]) -> tuple[str, ...]:
    kinds: Final = tuple(scorer.kind for scorer in scorers)
    return tuple(
        f"{kind}_{kinds[:index].count(kind) + 1}" if kinds.count(kind) > 1 else kind for index, kind in enumerate(kinds)
    )
