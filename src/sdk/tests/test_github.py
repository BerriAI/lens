from lens._contract import EvalRun, GateResult, Summary
from lens.models import Report
from lens.reporting import markdown


def report():
    summary = Summary(
        passed=1,
        total=1,
        pass_rate=1,
        cost_per_case=0.1,
        scores={"task_completed": 1},
        errors=0,
        baseline_run_id=None,
        baseline_version=None,
        regressions=(),
        fixed=(),
        gate=GateResult(passed=True, reasons=("no baseline on main for rev 1",)),
    )
    return Report(
        EvalRun(
            id="run",
            status="done",
            eval="demo",
            agent="demo",
            version="sha",
            branch="topic",
            pr=7,
            url="https://lens.example/runs/run",
            expected_trials=1,
            received_trials=1,
            summary=summary,
        )
    )


def test_markdown_escapes_remote_mentions_and_markup():
    value = report()
    summary = value.summary.model_copy(
        update={"gate": GateResult(passed=False, reasons=("@everyone <script> [click]",))}
    )
    body = markdown(Report(value.run.model_copy(update={"summary": summary})))
    assert "@everyone" not in body and "<script>" not in body
    assert "gate failed" in body
