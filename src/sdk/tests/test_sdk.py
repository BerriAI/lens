import asyncio
import json
from datetime import timedelta

import httpx
import pytest

from lens import Case, Eval, Gate, Run, judge, scorers
from lens._contract import CaseError, CaseResult, CreateEvalRun, TraceRef
from lens.client import Client
from lens.config import Execution, Settings
from lens.devserver import create_app, sample_cases
from lens.discovery import discover
from lens.errors import ApiFailure, ConfigurationError, GateFailed, InfrastructureError
from lens.models import DatasetCase, DatasetMessage
from lens.reporting import conclusion, markdown


async def good(case: Case) -> Run:
    return Run(trace={"session.id": f"pass-{case.id}"}, cost_usd=0.1)


@pytest.fixture
async def harness():
    app = create_app()
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app=app), base_url="http://test", headers={"Authorization": "Bearer lens-dev"}
    ) as http:
        yield Client(http, retry_delay=0), app.state.store


def execution(branch="main", identity="one", version="abc"):
    return Execution(version, branch, 123 if branch != "main" else None, identity=identity)


def evaluation(task=good, **kwargs):
    return Eval("demo", task=task, data="demo@1", scores=[scorers.task_completed()], **kwargs)


async def run_eval(client, evaluation=None, context=None):
    return await (evaluation or globals()["evaluation"]()).arun(
        client=client, settings=Settings(project="demo"), execution=context or execution()
    )


async def test_full_baseline_regression_fix_and_repeat(harness):
    client, state = harness
    base = await run_eval(client, evaluation(trials=3))
    assert base.passed == 36
    assert state.runs[base.run.id].received_trials == 108
    assert base.summary.baseline_run_id is None

    async def regress(case):
        return Run(trace={"session.id": "fail" if case.id == "case-0" else "pass"})

    bad = await run_eval(client, evaluation(regress, trials=3), execution("pr", "two"))
    assert bad.summary.baseline_run_id == base.run.id
    assert [case.case_id for case in bad.summary.regressions] == ["case-0"]
    assert bad.summary.regressions[0].critical
    with pytest.raises(GateFailed, match="regressions"):
        bad.assert_passed()
    fixed = await run_eval(client, evaluation(trials=3), execution("pr", "three"))
    fixed.assert_passed()
    repeat = await run_eval(client, evaluation(trials=3), execution("pr", "four"))
    assert repeat.run.id != fixed.run.id
    assert not repeat.summary.regressions
    assert "36/36" in markdown(bad) and "35/36" in markdown(bad)


async def test_trial_exception_timeout_and_wrong_return_are_results(harness):
    client, state = harness

    async def broken(case):
        if case.id == "case-0":
            raise RuntimeError("task failure")
        if case.id == "case-1":
            await asyncio.sleep(1)
        if case.id == "case-2":
            return None
        return await good(case)

    report = await run_eval(
        client, evaluation(broken, timeout_per_trial=timedelta(milliseconds=20), gate=Gate(pass_rate=1))
    )
    assert report.summary.errors == 3
    assert report.passed == 33
    assert not report.summary.gate.passed
    assert {result.error.type for result in state.results[report.run.id].values() if result.error} == {
        "RuntimeError",
        "TimeoutError",
        "ConfigurationError",
    }


async def test_concurrency_is_bounded(harness):
    client, state = harness
    active = set()
    maximum = []

    async def slow(case):
        active.add(asyncio.current_task())
        maximum.append(len(active))
        await asyncio.sleep(0.005)
        active.remove(asyncio.current_task())
        return await good(case)

    await run_eval(client, evaluation(slow, trials=3, concurrency=4))
    assert max(maximum) == 4
    assert not active


async def test_majority_ties_missing_trials_and_duplicate_put(harness):
    client, state = harness
    body = CreateEvalRun(
        eval="demo",
        agent="demo",
        dataset_id="demo",
        revision=1,
        case_ids=("case-0",),
        version="abc",
        branch="main",
        trials=2,
        scorers=(scorers.task_completed(),),
        gate=Gate(pass_rate=1),
    )
    run = await client.create(body, "same-request")
    assert (await client.create(body, "same-request")).id == run.id
    passed = CaseResult(trace=TraceRef(value="pass"))
    await client.result(run.id, "case-0", 0, passed)
    await client.result(run.id, "case-0", 0, passed)
    assert (await client.get(run.id)).received_trials == 1
    await client.finish(run.id)
    done = await client.get(run.id)
    assert done.summary.passed == 0
    assert done.summary.errors == 1
    assert not done.summary.gate.passed
    assert (await client.finish(run.id)).id == run.id
    with pytest.raises(ApiFailure) as failure:
        await client.result(run.id, "case-0", 1, passed)
    assert failure.value.code == "run_closed"


async def test_subset_is_immutable_and_has_independent_identity(harness):
    client, state = harness
    original = evaluation()
    subset = original.subset(case_ids=["case-0", "case-1"]).gate(Gate(pass_rate=1))
    assert original.case_ids is None
    assert original.threshold.pass_rate is None
    assert subset.name != original.name
    result = await run_eval(client, subset)
    assert result.total == 2
    assert original.subset(finding=1).finding == "1"
    with pytest.raises(ConfigurationError, match="not included"):
        await run_eval(client, original.subset(case_ids=["unknown"]))


async def test_distinct_revisions_and_scorers_do_not_reuse_baseline(harness):
    client, state = harness
    await run_eval(client)
    changed = Eval("demo", task=good, data="demo@1", scores=[judge("Check completion")])
    result = await run_eval(client, changed, execution("pr", "changed"))
    assert result.summary.baseline_run_id is None
    assert conclusion(result) == "neutral"
    state.datasets["demo"] = sample_cases().model_copy(update={"revision": 2})
    newer = Eval("demo", task=good, data="demo@2", scores=[scorers.task_completed()])
    result = await run_eval(client, newer, execution("pr", "revision"))
    assert result.summary.baseline_run_id is None


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
        {"gate": Gate(regressions=-1)},
        {"gate": Gate(min={"unknown": 0.5})},
        {"gate": Gate(min={"task_completed": float("nan")})},
    ],
)
def test_invalid_configuration_fails_early(overrides):
    with pytest.raises(ConfigurationError):
        evaluation(**overrides)


async def test_credentials_redacted_from_trial_errors(harness, monkeypatch):
    client, state = harness
    monkeypatch.setenv("TEST_API_KEY", "super-secret-test-key")

    async def fail(case):
        raise RuntimeError("authorization: super-secret-test-key")

    report = await run_eval(client, evaluation(fail))
    assert all(result.error.message == "authorization: [redacted]" for result in state.results[report.run.id].values())


async def test_sync_api_does_not_hide_active_loop():
    with pytest.raises(ConfigurationError, match="arun"):
        evaluation().run()


async def test_result_xor_is_enforced_before_http(harness):
    client, _ = harness
    for result in (CaseResult(), CaseResult(trace=TraceRef(value="trace"), error=CaseError(type="X", message="X"))):
        with pytest.raises(ConfigurationError, match="exactly one"):
            await client.result("run", "case", 0, result)


async def test_lost_create_ack_reuses_idempotency_key(harness):
    client, state = harness
    application = client.http._transport
    attempts = []

    async def flaky(request):
        if request.url.path == "/lens/evals/runs" and request.method == "POST":
            attempts.append(request.headers["Idempotency-Key"])
            response = await application.handle_async_request(request)
            if len(attempts) == 1:
                raise httpx.ReadError("lost acknowledgement", request=request)
            return response
        return await application.handle_async_request(request)

    async with httpx.AsyncClient(
        base_url="http://test", transport=httpx.MockTransport(flaky), headers={"Authorization": "Bearer lens-dev"}
    ) as http:
        result = await run_eval(Client(http, retry_delay=0))
    assert result.passed == 36
    assert len(state.runs) == 1
    assert len(attempts) == 2 and attempts[0] == attempts[1]


async def test_server_errors_are_not_reported_as_task_failures(harness):
    client, state = harness
    transport = client.http._transport

    async def fail_upload(request):
        if request.method == "PUT":
            return httpx.Response(401, json={"code": "unauthorized"})
        return await transport.handle_async_request(request)

    async with httpx.AsyncClient(
        base_url="http://test", transport=httpx.MockTransport(fail_upload), headers={"Authorization": "Bearer lens-dev"}
    ) as http:
        with pytest.raises(ApiFailure):
            await run_eval(Client(http, retry_delay=0))
    assert all(run.status == "running" for run in state.runs.values())


async def test_dataset_legacy_resolution_and_exact_revision():
    visited = []

    async def legacy(request):
        visited.append(str(request.url))
        if request.url.path.endswith("/resolve"):
            return httpx.Response(404, json={"detail": "Dataset not found"})
        if request.url.path == "/lens/datasets":
            return httpx.Response(200, json=[{"id": "uuid", "name": "real", "revision": 9}])
        return httpx.Response(200, json={"dataset_id": "uuid", "revision": 7, "cases": []})

    async with httpx.AsyncClient(base_url="http://test", transport=httpx.MockTransport(legacy)) as http:
        client = Client(http)
        resolved = await client.resolve("real@7")
        assert resolved.revision == 7
        assert (await client.cases(resolved)).revision == 7
        with pytest.raises(InfrastructureError, match="different"):
            await client.cases(resolved.model_copy(update={"revision": 8}))
    assert any("/revisions/7/cases" in url for url in visited)


async def test_malformed_and_redirect_response_fail_safely():
    for response in (
        httpx.Response(200, text="<html>login</html>"),
        httpx.Response(302, headers={"location": "https://evil"}),
    ):
        async with httpx.AsyncClient(
            base_url="http://test", transport=httpx.MockTransport(lambda request: response)
        ) as http:
            with pytest.raises(InfrastructureError):
                await Client(http).get("run")


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


async def test_report_public_summary_and_multiple_judges(harness):
    client, _ = harness
    e = Eval(
        "demo",
        task=good,
        data="demo",
        scores=[judge("First"), judge("Second")],
        gate=Gate(min={"judge_1": 1, "judge_2": 1}),
    )
    report = await run_eval(client, e)
    assert report.scores == {"judge_1": 1, "judge_2": 1}
    assert report.errors == 0 and report.total == report.passed == 36
    assert report.baseline_run_id is None and report.baseline_version is None
    assert report.regressions == report.fixed == ()
    assert report.gate.passed and report.pass_rate == 1 and report.cost_per_case > 0


async def test_failed_backend_is_infrastructure_error(harness):
    client, state = harness
    result = await run_eval(client)
    state.runs[result.run.id] = result.run.model_copy(update={"status": "failed", "failure": "backend error"})
    with pytest.raises(InfrastructureError, match="failed to score"):
        await run_eval(client)


async def test_lost_finish_ack_and_scoring_poll_recover(harness):
    client, state = harness
    application = client.http._transport
    finishes = []
    gets = []

    async def flaky(request):
        response = await application.handle_async_request(request)
        if request.url.path.endswith("/finish"):
            finishes.append(request.method)
            if len(finishes) == 1:
                raise httpx.ReadError("lost finish response", request=request)
        if request.method == "GET" and "/evals/runs/" in request.url.path:
            gets.append(request.method)
            if len(gets) == 1:
                payload = json.loads(await response.aread())
                return httpx.Response(200, json={**payload, "status": "scoring", "summary": None})
        return response

    async with httpx.AsyncClient(
        base_url="http://test", transport=httpx.MockTransport(flaky), headers={"Authorization": "Bearer lens-dev"}
    ) as http:
        report = await run_eval(Client(http, retry_delay=0))
    assert report.passed == 36 and len(state.runs) == 1
    assert len(finishes) == 2 and len(gets) >= 2


async def test_retryable_server_error_and_exhausted_transport():
    attempts = []

    async def limited(request):
        attempts.append(request)
        return httpx.Response(429 if len(attempts) < 3 else 204)

    async with httpx.AsyncClient(base_url="http://test", transport=httpx.MockTransport(limited)) as http:
        assert (await Client(http, retry_delay=0).request("PUT", "/result")).status_code == 204
    assert len(attempts) == 3

    async def offline(request):
        raise httpx.ConnectError("offline")

    async with httpx.AsyncClient(base_url="http://test", transport=httpx.MockTransport(offline)) as http:
        with pytest.raises(InfrastructureError, match="deadline"):
            await Client(http, retry_delay=0).request("GET", "/test")


async def test_finding_selector_and_strict_gate_without_baseline(harness):
    client, _ = harness
    report = await run_eval(client, evaluation().subset(finding=1), execution("topic"))
    assert report.total == 36 and conclusion(report) == "neutral"

    async def fail(case):
        raise RuntimeError("failed")

    missing = await run_eval(client, evaluation(fail, gate=Gate(pass_rate=1)), execution("topic", "failed"))
    assert not missing.gate.passed
    assert conclusion(missing) == "neutral"
    with pytest.raises(GateFailed):
        missing.assert_passed()
