import json
import os
import subprocess
import sys


def cli(root, endpoint, *arguments, key="lens-dev"):
    env = {**os.environ, "LENS_BASE_URL": endpoint, "LENS_API_KEY": key, "LENS_VERSION": "abc", "LENS_BRANCH": "main"}
    return subprocess.run(
        [sys.executable, "-m", "lens.cli", *arguments], cwd=root, env=env, capture_output=True, text=True, timeout=15
    )


def write_eval(root, body):
    (root / "pyproject.toml").write_text('[tool.lens]\nproject="demo"\nevals="evals"\n')
    (root / "evals").mkdir(exist_ok=True)
    (root / "evals/run.py").write_text(
        'from lens import Eval,Run,Gate,scorers\nprint("task module imported")\n'
        + f"async def task(case):\n    {body}\n"
        + "evaluation=Eval('demo',task=task,data='demo@1',scores=[scorers.task_completed()],gate=Gate(pass_rate=1))\n"
    )


def test_cli_real_http_pass_failure_and_json(endpoint, tmp_path):
    write_eval(tmp_path, "return Run(trace={'session.id':'pass'})")
    good = cli(tmp_path, endpoint, "eval", "--json")
    assert good.returncode == 0, good.stderr
    assert json.loads(good.stdout)["runs"][0]["summary"]["passed"] == 3
    assert "task module imported" in good.stderr
    write_eval(tmp_path, "raise RuntimeError('agent failed')")
    bad = cli(tmp_path, endpoint, "eval", "--json")
    assert bad.returncode == 1, bad.stderr
    assert json.loads(bad.stdout)["runs"][0]["summary"]["errors"] == 3
    forbidden = cli(tmp_path, endpoint, "eval", "--json", key="wrong")
    assert forbidden.returncode == 2
    assert json.loads(forbidden.stdout) == {"runs": []}
    assert "401" in forbidden.stderr


def test_cli_config_error_and_no_eval(endpoint, tmp_path):
    (tmp_path / "pyproject.toml").write_text('[tool.lens]\nproject="demo"\n')
    missing = cli(tmp_path, endpoint, "eval", "--json")
    assert missing.returncode == 2
    assert json.loads(missing.stdout) == {"runs": []}


def test_init_never_overwrites_user_files(endpoint, tmp_path):
    result = cli(tmp_path, endpoint, "init", "demo@1", "--project", "demo")
    assert result.returncode == 0, result.stderr
    created = (tmp_path / "evals/demo.py").read_bytes()
    duplicate = cli(tmp_path, endpoint, "init", "demo@1", "--project", "demo")
    assert duplicate.returncode == 2
    assert (tmp_path / "evals/demo.py").read_bytes() == created
    run = cli(tmp_path, endpoint, "eval", "--json")
    assert run.returncode == 1
    assert json.loads(run.stdout)["runs"][0]["summary"]["errors"] == 9
    assert (tmp_path / ".github/workflows/lens.yml").is_file()


def test_cli_timeout_is_recorded_not_a_crash(endpoint, tmp_path):
    write_eval(tmp_path, 'await __import__("asyncio").sleep(60)')
    path = tmp_path / "evals/run.py"
    path.write_text(
        "from datetime import timedelta\n"
        + path.read_text().replace("gate=Gate(", "timeout_per_trial=timedelta(milliseconds=10),gate=Gate(")
    )
    result = cli(tmp_path, endpoint, "eval", "--json")
    assert result.returncode == 1, result.stderr
    assert json.loads(result.stdout)["runs"][0]["summary"]["errors"] == 3


def test_multi_eval_preserves_completed_run_on_infrastructure_error(endpoint, tmp_path):
    write_eval(tmp_path, "return Run(trace={'session.id':'pass'})")
    (tmp_path / "evals/zbad.py").write_text(
        "from lens import Eval, Run, scorers\n"
        "async def task(case): return Run(trace={'session.id':'pass'})\n"
        "evaluation=Eval('missing',task=task,data='missing',scores=[scorers.task_completed()])\n"
    )
    result = cli(tmp_path, endpoint, "eval", "--json")
    assert result.returncode == 2
    assert len(json.loads(result.stdout)["runs"]) == 1
    assert json.loads(result.stdout)["runs"][0]["summary"]["gate"]["passed"]


def test_terminal_displays_task_results_and_authoritative_gate(endpoint, tmp_path):
    write_eval(tmp_path, "return Run(trace={'session.id':'pass'})")
    result = cli(tmp_path, endpoint, "eval")
    assert result.returncode == 0, result.stderr
    assert "case-0  1 submitted  0 task errors" in result.stdout
    assert "3/3 passed" in result.stdout


def test_init_protects_existing_workflow_before_writing(endpoint, tmp_path):
    path = tmp_path / ".github/workflows/lens.yml"
    path.parent.mkdir(parents=True)
    path.write_text("keep my workflow")
    result = cli(tmp_path, endpoint, "init", "demo", "--project", "demo")
    assert result.returncode == 2
    assert path.read_text() == "keep my workflow"
    assert not (tmp_path / "evals/demo.py").exists()


def test_action_reporter_real_http_upserts_and_writes_outputs(endpoint, tmp_path):
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    calls = []
    comments = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def respond(self, status, value):
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(value).encode())

        def do_GET(self):
            calls.append(("GET", self.path, None))
            self.respond(200, comments)

        def do_POST(self):
            value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append(("POST", self.path, value))
            if self.path.endswith("/comments"):
                comments.append({"id": 1, "body": value["body"], "user": {"login": "github-actions[bot]"}})
            self.respond(201, {})

        def do_PATCH(self):
            value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            calls.append(("PATCH", self.path, value))
            self.respond(200, {})

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        write_eval(tmp_path, "return Run(trace={'session.id':'pass'})")
        baseline = cli(tmp_path, endpoint, "eval", "--json")
        assert baseline.returncode == 0
        write_eval(tmp_path, "raise RuntimeError('intentional regression')")
        event = tmp_path / "event.json"
        event.write_text(json.dumps({"pull_request": {"number": 7}}))
        output = tmp_path / "output.txt"
        env = {
            **os.environ,
            "LENS_API_KEY": "lens-dev",
            "LENS_BASE_URL": endpoint,
            "GITHUB_TOKEN": "test-token",
            "GITHUB_REPOSITORY": "org/repo",
            "GITHUB_SHA": "sha",
            "GITHUB_REF_NAME": "topic",
            "GITHUB_HEAD_REF": "topic",
            "GITHUB_RUN_ID": "99",
            "GITHUB_EVENT_PATH": str(event),
            "GITHUB_API_URL": f"http://127.0.0.1:{server.server_port}",
            "GITHUB_OUTPUT": str(output),
        }
        failed = subprocess.run(
            [sys.executable, "-m", "lens.cli", "eval", "--ci", "--json"],
            cwd=tmp_path,
            env=env,
            text=True,
            capture_output=True,
            timeout=15,
        )
        assert failed.returncode == 1, failed.stderr
        value = json.loads(failed.stdout)
        assert value["runs"][0]["pr"] == 7
        value["runs"][0]["pr"] = 8
        value["runs"][0]["version"] = "untrusted-local-sha"
        report = tmp_path / "report.json"
        report.write_text(json.dumps(value))
        for _ in range(2):
            result = subprocess.run(
                [sys.executable, "-m", "lens.github", str(report)],
                cwd=tmp_path,
                env=env,
                text=True,
                capture_output=True,
                timeout=10,
            )
            assert result.returncode == 0, result.stderr
        assert len(comments) == 1
        assert sum(method == "PATCH" for method, _, _ in calls) == 1
        checks = [body for method, path, body in calls if path.endswith("/check-runs")]
        assert len(checks) == 2
        assert all(check["conclusion"] == "failure" for check in checks)
        assert all(check["head_sha"] == "sha" for check in checks)
        assert any(path == "/repos/org/repo/issues/7/comments" for _, path, _ in calls)
        assert not any(path == "/repos/org/repo/issues/8/comments" for _, path, _ in calls)
        assert "3 regressions" in comments[0]["body"]
        assert output.read_text().count("passed=false") == 2
        urls = next(line.split("=", 1)[1] for line in output.read_text().splitlines() if line.startswith("run-urls="))
        assert json.loads(urls) == [value["runs"][0]["url"]]
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def test_doctor_does_not_call_task(endpoint, tmp_path):
    write_eval(tmp_path, "raise AssertionError('doctor called task')")
    checked = cli(tmp_path, endpoint, "doctor", "--json")
    assert checked.returncode == 0, checked.stderr
    assert json.loads(checked.stdout)["ok"]
    assert "task module imported" in checked.stderr
    forbidden = cli(tmp_path, endpoint, "doctor", "--json", key="wrong")
    assert forbidden.returncode == 2
    assert not json.loads(forbidden.stdout)["ok"]


def test_same_size_edits_and_reverts_run_current_source(endpoint, tmp_path):
    write_eval(tmp_path, "return Run(trace={'session.id':'pass'})")
    source = tmp_path / "evals/run.py"
    original = source.read_text()
    timestamp = source.stat().st_mtime
    baseline = cli(tmp_path, endpoint, "eval", "--json")
    assert baseline.returncode == 0, baseline.stderr
    source.write_text(original.replace("'pass'", "'fail'"))
    os.utime(source, (timestamp, timestamp))
    broken = cli(tmp_path, endpoint, "eval", "--json")
    assert broken.returncode == 1, broken.stderr
    source.write_text(original)
    os.utime(source, (timestamp, timestamp))
    restored = cli(tmp_path, endpoint, "eval", "--json")
    assert restored.returncode == 0, restored.stderr
