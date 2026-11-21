from typing import Final

from evals import targets as targets
from litellm import EvalOutput, EvalTask, PredicateCheck


def billing(result: EvalOutput) -> bool:
    return result.text.strip().casefold() == "billing"


tasks: Final = [
    EvalTask(
        id="billing",
        prompt=(
            "Classify this ticket as billing, technical or sales. Reply with only the category word: "
            "I was charged twice"
        ),
        check=PredicateCheck(billing),
    )
]
