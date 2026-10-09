import asyncio
import json
import os
import sys
from importlib.metadata import distribution
from pathlib import Path

import lens
from lens import Case, Eval, Gate, Run, scorers
from lens.config import Execution, Settings
from lens.evaluation import Task


async def main() -> None:
    provenance = json.loads(distribution("lens-evals").read_text("direct_url.json") or "{}")
    assert provenance["archive_info"]["hashes"]["sha256"] == os.environ["LENS_SMOKE_WHEEL_SHA256"]
    assert Path(lens.__file__).is_relative_to(Path(sys.prefix))
    settings = Settings(project="release-qualification", base_url=os.environ["LENS_BASE_URL"])

    async def good(case: Case) -> Run:
        assert case.input == "Qualify the packaged SDK"
        assert case.expected == "Tests precede publication"
        return Run(trace={"trace_id": os.environ["LENS_SMOKE_GOOD_TRACE"]})

    async def bad(case: Case) -> Run:
        return Run(trace={"trace_id": os.environ["LENS_SMOKE_BAD_TRACE"]})

    def evaluation(task: Task) -> Eval:
        return Eval(
            "packaged-runtime",
            task=task,
            data="Packaged SDK@1",
            scores=[scorers.called_before("run_tests", "publish")],
            gate=Gate(pass_rate=1, regressions=0),
        )

    baseline = (
        await evaluation(good)
        .gate(Gate(pass_rate=1, regressions=None, critical=None))
        .arun(settings=settings, execution=Execution("baseline", "main", identity="baseline"))
    )
    assert baseline.total == baseline.passed == 1, f"{baseline.summary} {baseline.trials}"
    baseline.assert_passed()
    execution = Execution("candidate", "feature", identity="candidate")
    candidate = await evaluation(bad).arun(settings=settings, execution=execution)
    assert candidate.total == 1 and candidate.passed == 0
    assert candidate.gate.passed is False
    assert candidate.baseline_run_id == baseline.run.id
    assert len(candidate.regressions) == 1
    repeated = await evaluation(bad).arun(settings=settings, execution=execution)
    assert repeated.run.id == candidate.run.id
    assert repeated.summary == candidate.summary
    print(
        json.dumps(
            {
                "wheel_provenance_verified": True,
                "dataset_revision": 1,
                "baseline_passed": True,
                "regression_detected": True,
                "repeat_idempotent": True,
            }
        )
    )


if __name__ == "__main__":
    asyncio.run(main())
