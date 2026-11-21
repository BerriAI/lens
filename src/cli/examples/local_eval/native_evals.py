from typing import Final

from evals import targets as targets
from litellm import EvalTask, PredicateCheck
from litellm.harness import Result


def billing(result: Result) -> bool:
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
