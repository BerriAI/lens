import os
from typing import Final

from lens import Lens


def test_agent_regressions() -> None:
    lens: Final = Lens(base_url=os.environ["LENS_BASE_URL"], api_key=os.environ["LENS_API_KEY"])
    report: Final = lens.evals.run("moyai-coding-regressions")
    report.assert_passed()


if __name__ == "__main__":
    test_agent_regressions()
