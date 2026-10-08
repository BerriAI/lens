import json

import httpx

from lens._contract import EvalRun, GateResult, Summary
from lens.github import GitHub
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


async def test_report_updates_owned_comment_and_uses_server_verdict():
    requests = []

    async def handle(request):
        requests.append(request)
        if request.method == "GET":
            return httpx.Response(
                200, json=[{"id": 4, "body": "<!-- lens:demo --> old", "user": {"login": "github-actions[bot]"}}]
            )
        return httpx.Response(201, json={})

    async with httpx.AsyncClient(base_url="https://api.github.com", transport=httpx.MockTransport(handle)) as http:
        await GitHub(http, "org/repo").publish(report(), "sha")
        await GitHub(http, "org/repo").publish(report(), "sha")
    assert sum(request.method == "PATCH" for request in requests) == 2
    assert not any(request.method == "POST" and "/comments" in request.url.path for request in requests)
    checks = [json.loads(request.content) for request in requests if request.url.path.endswith("check-runs")]
    assert all(check["conclusion"] == "neutral" and check["head_sha"] == "sha" for check in checks)


async def test_comment_marker_in_human_comment_is_not_overwritten():
    requests = []

    async def handle(request):
        requests.append(request)
        if request.method == "GET":
            return httpx.Response(200, json=[{"id": 4, "body": "<!-- lens:demo -->", "user": {"login": "person"}}])
        return httpx.Response(201, json={})

    async with httpx.AsyncClient(base_url="https://api.github.com", transport=httpx.MockTransport(handle)) as http:
        await GitHub(http, "org/repo").publish(report(), "sha")
    assert not any(request.method == "PATCH" for request in requests)
    assert any(request.method == "POST" and "/comments" in request.url.path for request in requests)


def test_markdown_escapes_remote_mentions_and_markup():
    value = report()
    summary = value.summary.model_copy(
        update={"gate": GateResult(passed=False, reasons=("@everyone <script> [click]",))}
    )
    body = markdown(Report(value.run.model_copy(update={"summary": summary})))
    assert "@everyone" not in body and "<script>" not in body
    assert "gate failed" in body
