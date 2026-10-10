import json
import os
import subprocess
import sys
import threading
from collections.abc import Iterator
from http.server import ThreadingHTTPServer
from pathlib import Path
from queue import Queue
from typing import Final

import pytest
from named_agent import handler

from lens import Lens
from lens.config import Execution
from lens.errors import ConfigurationError, GateFailed


@pytest.fixture
def named_server(endpoint: str) -> Iterator[tuple[str, Queue[tuple[str, str, str | None, bytes]]]]:
    requests: Final[Queue[tuple[str, str, str | None, bytes]]] = Queue()
    server: Final = ThreadingHTTPServer(("127.0.0.1", 0), handler(endpoint, requests))
    thread: Final = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def configure(root: Path, endpoint: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GITHUB_ACTIONS", raising=False)
    (root / "pyproject.toml").write_text(
        '[tool.lens.connections.agent]\nbase_url_env="NAMED_AGENT_URL"\nauth="bearer"\ntoken_env="NAMED_AGENT_KEY"\n'
    )
    monkeypatch.chdir(root)
    monkeypatch.setenv("NAMED_AGENT_URL", endpoint)
    monkeypatch.setenv("NAMED_AGENT_KEY", "agent-test-credential")
    monkeypatch.setenv("LENS_VERSION", "deployed-build")
    monkeypatch.setenv("LENS_BRANCH", "topic")


def test_named_client_uses_saved_mapping_and_separate_credentials(named_server, tmp_path, monkeypatch):
    endpoint, requests = named_server
    configure(tmp_path, endpoint, monkeypatch)
    monkeypatch.setenv("LENS_API_KEY", "do-not-use-this-ambient-key")
    environment: Final = dict(os.environ)
    client: Final = Lens(base_url=endpoint, api_key="lens-dev")
    report: Final = client.evals.run("named-pass", execution=Execution("deployed-build", "main", identity="named-base"))
    report.assert_passed()
    assert report.total == report.passed == 3
    assert len(report.trials) == 3
    assert {trial.result.output for trial in report.trials} == {
        "Completed Case 0",
        "Completed Case 1",
        "Completed Case 2",
    }
    assert all(trial.result.trace.attribute == "session.id" for trial in report.trials)
    assert dict(os.environ) == environment
    assert "lens-dev" not in repr(client)
    assert "lens-dev" not in repr(client.evals)
    observed: Final = tuple(requests.get_nowait() for _ in range(requests.qsize()))
    invocations: Final = tuple(call for call in observed if call[1] == "/invoke")
    assert len(invocations) == 3
    assert all(call[2] == "Bearer agent-test-credential" for call in invocations)
    assert {json.loads(call[3])["input"] for call in invocations} == {"Case 0", "Case 1", "Case 2"}
    assert len({json.loads(call[3])["request_id"] for call in invocations}) == 3
    assert all(call[2] == "Bearer lens-dev" for call in observed if call[1].startswith("/lens/"))
    assert not any(call[1].startswith("/lens/datasets/resolve") for call in observed)


async def test_named_client_async_report_preserves_server_gate(named_server, tmp_path, monkeypatch):
    endpoint, _ = named_server
    configure(tmp_path, endpoint, monkeypatch)
    report: Final = await Lens(endpoint, "lens-dev").evals.arun("named-fail")
    assert report.run.version == "deployed-build"
    assert report.run.branch == "topic"
    assert report.total == 3 and report.passed == 0
    with pytest.raises(GateFailed):
        report.assert_passed()


def test_named_client_detects_github_context_but_keeps_deployed_version(named_server, tmp_path, monkeypatch):
    endpoint, _ = named_server
    configure(tmp_path, endpoint, monkeypatch)
    event: Final = tmp_path / "event.json"
    event.write_text(json.dumps({"pull_request": {"number": 29}}))
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv("GITHUB_EVENT_PATH", str(event))
    monkeypatch.setenv("GITHUB_SHA", "checkout-only-sha")
    monkeypatch.setenv("GITHUB_HEAD_REF", "ci-topic")
    monkeypatch.setenv("GITHUB_RUN_ID", "named-github")
    monkeypatch.setenv("GITHUB_RUN_ATTEMPT", "1")
    monkeypatch.setenv("GITHUB_REPOSITORY", "example/agent")
    report: Final = Lens(endpoint, "lens-dev").evals.run("named-pass")
    assert report.run.pr == 29
    assert report.run.version == "deployed-build"
    assert report.run.branch == "ci-topic"
    result: Final = subprocess.run(
        [sys.executable, "-m", "lens.cli", "eval", "--name", "named-pass", "--ci", "--json"],
        cwd=tmp_path,
        env={**os.environ, "LENS_BASE_URL": endpoint, "LENS_API_KEY": "lens-dev"},
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.returncode == 0, result.stderr
    cli_run: Final = json.loads(result.stdout)["runs"][0]
    assert cli_run["pr"] == 29
    assert cli_run["version"] == "deployed-build"
    assert cli_run["branch"] == "ci-topic"


def test_named_client_explicit_build_context_without_identity_runs_fresh(named_server, tmp_path, monkeypatch):
    endpoint, _ = named_server
    configure(tmp_path, endpoint, monkeypatch)
    client: Final = Lens(endpoint, "lens-dev")
    execution: Final = Execution("deployed-build", "topic")
    first: Final = client.evals.run("named-pass", execution=execution)
    second: Final = client.evals.run("named-pass", execution=execution)
    assert first.run.id != second.run.id
    assert first.total == second.total == 3


async def test_named_sync_api_rejects_active_event_loop():
    with pytest.raises(ConfigurationError, match="arun"):
        Lens("http://127.0.0.1:1", "not-sent").evals.run("named-pass")


def test_named_runner_requires_trusted_local_connection_before_agent_calls(named_server, tmp_path, monkeypatch):
    endpoint, requests = named_server
    monkeypatch.chdir(tmp_path)
    with pytest.raises(ConfigurationError, match="connection"):
        Lens(endpoint, "lens-dev").evals.run("named-pass", execution=Execution("build", "topic"))
    observed: Final = tuple(requests.get_nowait() for _ in range(requests.qsize()))
    assert not any(call[1] == "/invoke" for call in observed)
    assert not any(call[0] == "POST" and call[1] == "/lens/evals/runs" for call in observed)


def test_named_cli_runs_without_importing_eval_files_and_enforces_gates(named_server, tmp_path, monkeypatch):
    endpoint, _ = named_server
    configure(tmp_path, endpoint, monkeypatch)
    (tmp_path / "evals").mkdir()
    (tmp_path / "evals/fail.py").write_text("raise AssertionError('Named runs must not import eval files')\n")
    env: Final = {**os.environ, "LENS_BASE_URL": endpoint, "LENS_API_KEY": "lens-dev"}
    for name, code in (("named-pass", 0), ("named-fail", 1)):
        result: Final = subprocess.run(
            [sys.executable, "-m", "lens.cli", "eval", "--name", name, "--json"],
            cwd=tmp_path,
            env=env,
            capture_output=True,
            text=True,
            timeout=20,
        )
        assert result.returncode == code, result.stderr
        assert json.loads(result.stdout)["runs"][0]["summary"]["gate"]["passed"] == (code == 0)


@pytest.mark.parametrize("arguments", [("evals",), ("--eval", "local-name")])
def test_named_cli_rejects_ambiguous_selection(arguments, tmp_path):
    result: Final = subprocess.run(
        [sys.executable, "-m", "lens.cli", "eval", "--name", "named-pass", *arguments],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 2
    assert "cannot be used" in result.stderr


def test_saved_python_evaluation_records_local_outputs_and_server_baseline(named_server, tmp_path):
    endpoint, requests = named_server
    client: Final = Lens(endpoint, "lens-dev")
    with client.evals.test("named-python", execution=Execution("baseline-build", "main")) as evaluation:
        assert len(evaluation.cases) == 3
        assert evaluation.report is None
        for case in evaluation.cases:
            evaluation.record(case, output=f"Python answered: {case.input}", trace_id=f"pass-{case.id}")
        baseline: Final = evaluation.finish()
        baseline.assert_passed()
    assert evaluation.report is not None
    with client.evals.test("named-python", execution=Execution("candidate-build", "topic")) as candidate:
        for case in candidate.cases:
            candidate.record(case, output="Improved answer", session_id=f"pass-{case.id}")
        report: Final = candidate.finish()
        assert report.baseline_run_id == baseline.run.id
        assert report.baseline is not None
        assert report.baseline.version == "baseline-build"
        target: Final = tmp_path / "report.json"
        report.write_json(target)
        assert json.loads(target.read_text())["runs"][0]["id"] == report.run.id
    observed: Final = tuple(requests.get_nowait() for _ in range(requests.qsize()))
    assert not any(call[1] in {"/invoke", "/fail"} for call in observed)
    assert all(json.loads(call[3]).get("agent_io") is None for call in observed if call[1] == "/lens/evals/runs")


def test_saved_python_context_records_execution_errors_without_hiding_original_exception(named_server):
    endpoint, _ = named_server
    client: Final = Lens(endpoint, "lens-dev")
    with pytest.raises(RuntimeError, match="The agent failed"):
        with client.evals.test("named-python-error", execution=Execution("build", "topic")) as evaluation:
            first: Final = evaluation.cases[0]
            evaluation.record(first, output="First case worked", trace_id="pass-first")
            raise RuntimeError("The agent failed")
    assert evaluation.report is not None
    assert evaluation.report.errors == 2
    assert evaluation.report.run.received_trials == 3
    assert {trial.result.error.type for trial in evaluation.report.trials if trial.result.error} == {"RuntimeError"}


def test_saved_python_context_rejects_missing_results_and_duplicate_recording(named_server):
    endpoint, _ = named_server
    with pytest.raises(GateFailed):
        with Lens(endpoint, "lens-dev").evals.test(
            "named-python-missing", execution=Execution("build", "topic")
        ) as evaluation:
            first: Final = evaluation.cases[0]
            evaluation.record(first, output="First case worked", trace_id="pass-first")
            with pytest.raises(ConfigurationError, match="already"):
                evaluation.record(first, output="Duplicate", trace_id="pass-duplicate")
            with pytest.raises(ConfigurationError, match="one trace"):
                evaluation.record(first, output="Ambiguous", trace_id="trace", session_id="session")
    assert evaluation.report is not None
    assert evaluation.report.errors == 2
    with pytest.raises(ConfigurationError, match="finishing"):
        evaluation.record(first, output="Too late")
    with pytest.raises(ConfigurationError, match="more than once"):
        evaluation.__enter__()


def test_saved_python_can_record_error_and_continue_other_cases(named_server):
    endpoint, _ = named_server
    with pytest.raises(GateFailed):
        with Lens(endpoint, "lens-dev").evals.test(
            "named-python-caught", execution=Execution("build", "topic")
        ) as evaluation:
            for case in evaluation.cases:
                if case.id == "case-0":
                    evaluation.record_error(case, ValueError("Bad agent input"))
                else:
                    evaluation.record(case, output="Completed", trace_id=f"pass-{case.id}")
    assert evaluation.report is not None
    assert evaluation.report.errors == 1
    assert evaluation.report.passed == 2
