from typing import Annotated, Final

from pydantic import Field

from ._generated import (
    ApiError as ApiError,
    CalledBefore as CalledBefore,
    CaseDiff as CaseDiff,
    CaseError as CaseError,
    CaseResult as CaseResult,
    CreateEvalRun as CreateEvalRun,
    EvalRun as EvalRun,
    Gate as Gate,
    GateResult as GateResult,
    Judge as Judge,
    ResolvedDataset as ResolvedDataset,
    Summary as Summary,
    TaskCompleted as TaskCompleted,
    TraceRef as TraceRef,
)

CONTRACT_VERSION: Final = 1
Scorer = Annotated[TaskCompleted | CalledBefore | Judge, Field(discriminator="kind")]


def scorer_names(scorers: tuple[Scorer, ...]) -> tuple[str, ...]:
    kinds: Final = tuple(scorer.kind for scorer in scorers)
    return tuple(
        f"{kind}_{kinds[:index].count(kind) + 1}" if kinds.count(kind) > 1 else kind for index, kind in enumerate(kinds)
    )
