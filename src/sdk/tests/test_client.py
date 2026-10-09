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
    (root / "pyproject.toml").write_text(
        '[tool.lens.connections.agent]\nbase_url_env="NAMED_AGENT_URL"\nauth="bearer"\ntoken_env="NAMED_AGENT_KEY"\n'
    )
    monkeypatch.chdir(root)
    monkeypatch.delenv("GITHUB_ACTIONS", raising=False)
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
    monkeypatch.setenv("GITHUB_HEAD_REF", "topic")
    monkeypatch.setenv("GITHUB_RUN_ID", "named-github")
    monkeypatch.setenv("GITHUB_RUN_ATTEMPT", "1")
    monkeypatch.setenv("GITHUB_REPOSITORY", "example/agent")
    report: Final = Lens(endpoint, "lens-dev").evals.run("named-pass")
    assert report.run.pr == 29
    assert report.run.version == "deployed-build"
    assert report.run.branch == "topic"
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
    assert cli_run["branch"] == "topic"


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
