import asyncio
import json
from datetime import timedelta

import pytest
from pydantic import ValidationError

from lens import Case, Eval, Gate, Run, judge, scorers
from lens.config import Execution, Settings
from lens.discovery import discover
from lens.errors import ConfigurationError, GateFailed
from lens.models import DatasetCase, DatasetMessage
from lens.reporting import conclusion


async def good(case: Case) -> Run:
    return Run(trace={"session.id": f"pass-{case.id}"}, cost_usd=0.1)


def evaluation(task=good, **kwargs):
    return Eval("demo", task=task, data="demo@1", scores=[scorers.task_completed()], **kwargs)


def test_case_mapping_immutability_and_trace_validation():
    wire = DatasetCase(
        id="real",
        messages=(
            DatasetMessage(role="system", content="omit"),
            DatasetMessage(role="user", content="first"),
            DatasetMessage(role="assistant", content="omit"),
            DatasetMessage(role="tool", content="omit"),
            DatasetMessage(role="user", content="second"),
        ),
        meta={"repo_url": "https://example.com/repo"},
        expected="done",
    )
    case = wire.to_case()
    assert (case.input, case.followups, case.expected) == ("first", ("second",), "done")
    with pytest.raises(TypeError):
        case.meta["repo_url"] = "changed"
    for trace in ({}, {"other": "abc"}, {"session.id": "a", "trace_id": "b"}, {"session.id": ""}):
        with pytest.raises(ConfigurationError):
            Run(trace=trace)
    with pytest.raises(ConfigurationError):
        Run(trace={"trace_id": "abc"}, cost_usd=float("nan"))


@pytest.mark.parametrize(
    "overrides",
    [
        {"trials": 0},
        {"concurrency": 0},
        {"baseline": "release"},
        {"timeout_per_trial": timedelta(0)},
        {"gate": Gate(min={"unknown": 0.5})},
        {"gate": Gate(min={"task_completed": float("nan")})},
    ],
)
def test_invalid_configuration_fails_early(overrides):
    with pytest.raises(ConfigurationError):
        evaluation(**overrides)


def test_negative_gate_thresholds_are_rejected_by_the_contract():
    with pytest.raises(ValidationError):
        Gate(regressions=-1)


async def test_sync_api_does_not_hide_active_loop():
    with pytest.raises(ConfigurationError, match="arun"):
        evaluation().run()


def test_discovery_only_imports_evals_and_rejects_duplicate_names(tmp_path):
    template = (
        "from lens import Eval, Run, scorers\n"
        "async def task(case): return Run(trace={'session.id':'pass'})\n"
        "eval = Eval('demo', task=task, data='demo', scores=[scorers.task_completed()])\n"
    )
    (tmp_path / "eval.py").write_text(template)
    assert len(discover(tmp_path)) == 1
    (tmp_path / "other.py").write_text(template)
    with pytest.raises(ConfigurationError, match="unique"):
        discover(tmp_path)


def test_github_identity_uses_execution_not_only_sha(tmp_path, monkeypatch):
    path = tmp_path / "event.json"
    path.write_text(json.dumps({"pull_request": {"number": 17}}))
    for name, value in {
        "GITHUB_EVENT_PATH": str(path),
        "GITHUB_SHA": "merge-sha",
        "GITHUB_HEAD_REF": "topic",
        "GITHUB_REF_NAME": "17/merge",
        "GITHUB_RUN_ID": "101",
        "GITHUB_RUN_ATTEMPT": "2",
        "GITHUB_REPOSITORY": "org/repo",
    }.items():
        monkeypatch.setenv(name, value)
    result = Execution.github()
    assert result.version == "merge-sha" and result.branch == "topic" and result.pr == 17
    assert result.identity == "101:2"


async def test_native_engine_public_lifecycle(native_endpoint, monkeypatch):
    monkeypatch.setenv("LENS_API_KEY", "lens-dev")
    settings = Settings(project="demo", base_url=native_endpoint)
    baseline = await evaluation(trials=3).arun(settings=settings, execution=Execution("sha", "main", identity="base"))
    assert baseline.passed == baseline.total == 36
    assert len(baseline.trials) == 108

    async def broken(case):
        if case.id == "case-0":
            raise RuntimeError("agent failed")
        return await good(case)

    candidate = await evaluation(broken, trials=3).arun(
        settings=settings, execution=Execution("sha", "topic", pr=1, identity="bad")
    )
    assert candidate.errors == 3
    assert [case.case_id for case in candidate.regressions] == ["case-0"]
    assert candidate.baseline_run_id == baseline.run.id
    with pytest.raises(GateFailed):
        candidate.assert_passed()
    again = await evaluation(trials=3).arun(settings=settings, execution=Execution("sha", "topic", identity="again"))
    again.assert_passed()
    assert again.regressions == ()


async def test_native_callbacks_timeout_and_cancel_do_not_leak(native_endpoint, monkeypatch):
    monkeypatch.setenv("LENS_API_KEY", "lens-dev")
    active = set()
    maximum = []

    async def slow(case):
        current = asyncio.current_task()
        active.add(current)
        maximum.append(len(active))
        try:
            await asyncio.sleep(30)
            return await good(case)
        finally:
            await asyncio.sleep(0.02)
            active.remove(current)

    configured = evaluation(slow, concurrency=3, timeout_per_trial=timedelta(milliseconds=30), gate=Gate(pass_rate=1))
    settings = Settings(project="demo", base_url=native_endpoint)
    result = await configured.arun(settings=settings, execution=Execution("sha", "main", identity="timeouts"))
    assert result.errors == 36
    assert max(maximum) == 3
    assert not active
    pending = asyncio.create_task(
        evaluation(slow, concurrency=2).arun(settings=settings, execution=Execution("sha", "main", identity="cancel"))
    )
    while not active:
        await asyncio.sleep(0.005)
    pending.cancel()
    with pytest.raises(asyncio.CancelledError):
        await pending
    await asyncio.sleep(0.02)
    assert not active


async def test_native_subsets_judges_and_metadata(native_endpoint, monkeypatch):
    monkeypatch.setenv("LENS_API_KEY", "lens-dev")
    seen = []

    async def inspect(case):
        seen.append(case)
        return await good(case)

    configured = Eval(
        "judges",
        task=inspect,
        data="demo",
        scores=[judge("First"), judge("Second")],
        gate=Gate(min={"judge_1": 1, "judge_2": 1}),
    )
    selected = configured.subset(case_ids=["case-0", "case-1"])
    report = await selected.arun(
        settings=Settings(project="demo", base_url=native_endpoint),
        execution=Execution("sha", "topic", pr=1, identity="judges"),
    )
    assert configured.case_ids is None
    assert report.total == 2
    assert report.scores == {"judge_1": 1, "judge_2": 1}
    assert seen[0].input.startswith("Case ")
    assert seen[0].meta["priority"] == "high"
    assert report.gate.passed
    assert conclusion(report) == "neutral"


async def test_native_task_errors_redact_secrets(native_endpoint, monkeypatch):
    monkeypatch.setenv("LENS_API_KEY", "lens-dev")
    monkeypatch.setenv("AGENT_API_KEY", "secret-agent-key-value")

    async def broken(case):
        raise RuntimeError("request failed: secret-agent-key-value")

    report = (
        await evaluation(broken)
        .subset(case_ids=["case-0"])
        .arun(
            settings=Settings(project="demo", base_url=native_endpoint),
            execution=Execution("sha", "main", identity="redact"),
        )
    )
    assert report.trials[0].result.error.message == "request failed: [redacted]"
