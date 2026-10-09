import json
import os
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from queue import Queue
from typing import Final

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
    assert "Gate failed" in body


def test_app_reporter_uses_lens_auth_without_github_token(tmp_path: Path) -> None:
    requests: Final[Queue[tuple[str, str | None, bytes]]] = Queue()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:
            pass

        def do_POST(self) -> None:
            requests.put(
                (self.path, self.headers.get("Authorization"), self.rfile.read(int(self.headers["Content-Length"])))
            )
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"reports":[{"run_id":"run"}]}')

    server: Final = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread: Final = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        payload: Final = tmp_path / "report.json"
        payload.write_text(json.dumps({"runs": [report().run.model_dump(mode="json")]}))
        output: Final = tmp_path / "output.txt"
        env: Final = {
            **{key: value for key, value in os.environ.items() if not key.startswith("GITHUB_")},
            "LENS_BASE_URL": f"http://127.0.0.1:{server.server_port}",
            "LENS_API_KEY": "lens-test",
            "GITHUB_OUTPUT": str(output),
        }
        result: Final = subprocess.run(
            [sys.executable, "-m", "lens.github", str(payload), "--via-app"],
            cwd=tmp_path,
            env=env,
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0, result.stderr
        assert requests.get_nowait() == ("/lens/github/report", "Bearer lens-test", b'{"run_ids":["run"]}')
        assert requests.empty()
        assert "passed=true" in output.read_text()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
