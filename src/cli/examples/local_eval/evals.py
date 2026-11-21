import os
from collections.abc import Mapping
from typing import Final

from litellm import EvalOutput, EvalTarget, EvalTask, Harness, PredicateCheck

from agent import INSTRUCTIONS, lookup_ticket

MODEL_A: Final = os.environ.get("LENS_MODEL_A", "litellm_proxy/model-a")
MODEL_B: Final = os.environ.get("LENS_MODEL_B", "litellm_proxy/model-b")

targets: Final = [
    EvalTarget(Harness.TOOL_LOOP, MODEL_A, tools=(lookup_ticket,)),
    EvalTarget(Harness.TOOL_LOOP, MODEL_B, tools=(lookup_ticket,)),
    EvalTarget(Harness.PI, MODEL_A, tools=(lookup_ticket,)),
]
run_options: Final = {"max_turns": 8}


def task(row: Mapping[str, str | None]) -> EvalTask:
    expected: Final = (row["expected"] or "").strip().casefold()

    def matches(result: EvalOutput) -> bool:
        return result.text.strip().strip(".").casefold() == expected

    return EvalTask(
        id=row["id"] or "",
        prompt=INSTRUCTIONS + "\n\n" + (row["prompt"] or ""),
        check=PredicateCheck(matches),
    )
