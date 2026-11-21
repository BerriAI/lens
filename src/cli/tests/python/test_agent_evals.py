import asyncio
import json
import os
import signal
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Queue

import pytest

from lens.local import load_config

litellm = pytest.importorskip("litellm")
pytest.importorskip("litellm.harness.evaluation")
if not hasattr(litellm, "aagent_eval_attempt"):
    pytest.skip("Install the agent-evals extra for the companion SDK API", allow_module_level=True)


@pytest.fixture
def model_server():
    requests = Queue()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.put((payload, {key.lower(): value for key, value in self.headers.items()}))
            messages = payload["messages"]
            prompt = next(message["content"] for message in messages if message["role"] == "user")
            if "SLOW" in prompt:
                time.sleep(2)
            judge = payload.get("response_format") == {"type": "json_object"}
            tool = "USE_TOOL" in prompt and not any(message["role"] == "tool" for message in messages)
            message = (
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [
                        {
                            "id": "lookup",
                            "type": "function",
                            "function": {"name": "lookup", "arguments": '{"value":"billing"}'},
                        }
                    ],
                }
                if tool
                else {"role": "assistant", "content": "invalid verdict" if judge else "billing"}
            )
            body = {
                "id": "controlled-response",
                "object": "chat.completion",
                "created": 0,
                "model": payload["model"],
                "choices": [{"index": 0, "message": message, "finish_reason": "tool_calls" if tool else "stop"}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8},
            }
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("x-litellm-response-cost", "0.125")
            self.end_headers()
            try:
                self.wfile.write(json.dumps(body).encode())
            except BrokenPipeError:
                pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/v1", requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)


def write_sdk_config(root, endpoint, tasks, *, options=""):
    path = root / "evals.py"
    path.write_text(
        "import sys\n"
        "from litellm import EvalTarget, EvalTask, Harness, PredicateCheck, CommandCheck, JudgeCheck\n"
        "def lookup(value: str) -> str:\n    return value\n"
        "targets = [EvalTarget(Harness.TOOL_LOOP, 'openai/controlled', tools=(lookup,))]\n"
        f"run_options = {{'api_base': {endpoint!r}, 'api_key': 'controlled-test', {options}}}\n" + tasks
    )
    return path


def run_cli(root, *arguments, env=None):
    return subprocess.run(
        [sys.executable, "-m", "lens.cli", *arguments],
        cwd=root,
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
    )


def attempts(root):
    directory = next((root / ".lens/runs").iterdir())
    return directory, tuple(json.loads(line) for line in (directory / "attempts.jsonl").read_text().splitlines())


def test_sdk_native_tasks_trials_tools_workspace_and_suite_parity(tmp_path, model_server):
    endpoint, _ = model_server
    path = write_sdk_config(
        tmp_path,
        endpoint,
        "tasks = [\n"
        "    EvalTask('tool', 'USE_TOOL', check=PredicateCheck(lambda result: result.text == 'billing')),\n"
        "    EvalTask('fail', 'Classify', check=PredicateCheck(lambda result: False)),\n"
        "    EvalTask('workspace', 'Classify', files={'input.txt': 'before'},\n"
        "        setup=((sys.executable, '-c',\n"
        "            \"from pathlib import Path; Path('input.txt').write_text('after')\"),),\n"
        "        check=CommandCheck((sys.executable, '-c',\n"
        "            \"from pathlib import Path; assert Path('input.txt').read_text() == 'after'\"))),\n"
        "]\n",
    )
    config = load_config(path)
    direct = asyncio.run(
        litellm.aagent_eval(
            tasks=config.sdk.tasks,
            targets=config.targets,
            trials=2,
            max_concurrency=1,
            api_base=endpoint,
            api_key="controlled-test",
        )
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--trials", "2", "--concurrency", "1", "--fast")
    assert completed.returncode == 1, completed.stderr
    directory, saved = attempts(tmp_path)
    assert len(saved) == len(direct.runs) == 6
    by_attempt = {(item["task_id"], item["trial"]): item for item in saved}
    for run in direct.runs:
        result = by_attempt[(run.task_id, run.trial)]
        assert result["outcome"]["status"] == run.status.value
        assert result["stop_reason"] == run.stop_reason
        assert result["input_tokens"] == run.usage.input_tokens
        assert result["output_tokens"] == run.usage.output_tokens
        assert result["model_calls"] == run.usage.calls
        assert result["cost"] == run.cost
        assert result["judge_cost"] == run.check.judge_cost
        assert result["check_detail"] == run.check.detail
        assert len(result["tool_calls"]) == run.tool_calls
    assert by_attempt[("tool", 1)]["tool_calls"] == [
        {"name": "lookup", "arguments": {"value": "billing"}, "error": False}
    ]
    assert json.loads((directory / "meta.json").read_text())["trials"] == 2
    before = (directory / "attempts.jsonl").read_bytes()
    resumed = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert resumed.returncode == 1, resumed.stderr
    assert (directory / "attempts.jsonl").read_bytes() == before


def test_csv_uses_sdk_default_check_and_task_factory(tmp_path, model_server):
    endpoint, _ = model_server
    (tmp_path / "tasks.csv").write_text("id,prompt,expected,extra\nt1,Classify,BILLING,yes\n")
    path = write_sdk_config(tmp_path, endpoint, "")
    initial = run_cli(tmp_path, "run", "tasks.csv", "--config", "evals.py", "--fast")
    assert initial.returncode == 0, initial.stderr
    _, first = attempts(tmp_path)
    assert first[0]["outcome"] == {"status": "pass"}
    path.write_text(
        path.read_text()
        + (
            "def task(row):\n"
            "    return EvalTask(row['id'], row['prompt'], files={'extra.txt': row['extra']},\n"
            "        check=CommandCheck((sys.executable, '-c',\n"
            "            \"from pathlib import Path; assert Path('extra.txt').read_text() == 'yes'\")))\n"
        )
    )
    custom = run_cli(tmp_path, "run", "tasks.csv", "--config", "evals.py", "--fast")
    assert custom.returncode == 0, custom.stderr


@pytest.mark.parametrize(
    "task",
    [
        "EvalTask('judge', 'Classify', check=JudgeCheck('Answer must be billing', 'openai/controlled'))",
        "EvalTask('setup', 'Classify', setup=((sys.executable, '-c', 'raise SystemExit(3)'),), "
        "check=PredicateCheck(lambda result: True))",
    ],
)
def test_sdk_errors_exit_two_and_preserve_unknown_metrics(tmp_path, model_server, task):
    endpoint, _ = model_server
    write_sdk_config(tmp_path, endpoint, f"tasks = [{task}]\n")
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 2, completed.stderr
    _, saved = attempts(tmp_path)
    assert saved[0]["outcome"]["status"] == "error"
    if saved[0]["task_id"] == "judge":
        assert saved[0]["check_error"] == "Judge returned an invalid verdict"
        assert saved[0]["stop_reason"] == "done"
        assert saved[0]["judge_model_calls"] == 1
    else:
        assert saved[0]["cost"] is None
        assert saved[0]["input_tokens"] is None
        assert saved[0]["judge_cost"] is None
        assert not saved[0]["metrics_complete"]


def test_interrupt_resume_skips_completed_sdk_attempts(tmp_path, model_server):
    endpoint, requests = model_server
    write_sdk_config(
        tmp_path,
        endpoint,
        "tasks = [EvalTask('fast', 'Classify', check=PredicateCheck(lambda result: True)),\n"
        "    EvalTask('slow', 'SLOW Classify', check=PredicateCheck(lambda result: True))]\n",
    )
    process = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "lens.cli",
            "run",
            "--config",
            "evals.py",
            "--trials",
            "2",
            "--concurrency",
            "1",
            "--fast",
        ],
        cwd=tmp_path,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            paths = tuple((tmp_path / ".lens/runs").glob("*/attempts.jsonl"))
            if paths and paths[0].read_text().strip():
                break
            time.sleep(0.05)
        else:
            pytest.fail("First SDK attempt was not checkpointed")
        process.send_signal(signal.SIGINT)
        process.communicate(timeout=10)
        directory, before = attempts(tmp_path)
        assert 0 < len(before) < 4
        resumed = run_cli(tmp_path, "resume", directory.name, "--fast", "--concurrency", "1")
        assert resumed.returncode == 0, resumed.stderr
        _, after = attempts(tmp_path)
        assert len(after) == 4
        keys = [(item["task_id"], item["trial"]) for item in after]
        assert len(set(keys)) == 4
        for item in before:
            assert item in after
        assert requests.qsize() >= 4
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=5)


def test_sdk_labels_preserve_distinct_targets_and_gateway_run_correlation(tmp_path, model_server):
    endpoint, requests = model_server
    path = write_sdk_config(
        tmp_path,
        endpoint,
        "targets = [EvalTarget(Harness.TOOL_LOOP, 'litellm_proxy/controlled', label='first'),\n"
        "    EvalTarget(Harness.TOOL_LOOP, 'litellm_proxy/controlled', label='second')]\n"
        "tasks = [EvalTask('t', 'Classify', check=PredicateCheck(lambda result: True))]\n",
    )
    config = load_config(path, only=("second",))
    assert [target.name for target in config.targets] == ["second"]
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert {item["target"]["label"] for item in saved} == {"first", "second"}
    assert all(item["run_id"] for item in saved)
    observed = tuple(requests.get_nowait() for _ in range(requests.qsize()))
    metadata = tuple(json.loads(headers["x-litellm-spend-logs-metadata"]) for _, headers in observed)
    assert {value["agent_eval_suite"] for value in metadata} == {directory.name}
    assert {value["agent_eval_run"] for value in metadata} == {item["run_id"] for item in saved}


def test_sdk_csv_requires_an_explicit_check_without_expected(tmp_path, model_server):
    endpoint, requests = model_server
    (tmp_path / "tasks.csv").write_text("id,prompt\nt,Classify\n")
    write_sdk_config(tmp_path, endpoint, "")
    completed = run_cli(tmp_path, "run", "tasks.csv", "--config", "evals.py", "--fast")
    assert completed.returncode == 2, completed.stderr
    _, saved = attempts(tmp_path)
    assert "expected values" in saved[0]["outcome"]["message"]
    assert requests.qsize() == 0


def test_resume_rejects_changed_sdk_options_but_allows_credential_rotation(tmp_path, model_server):
    endpoint, _ = model_server
    write_sdk_config(
        tmp_path,
        endpoint,
        "import os\n"
        "from types import MappingProxyType\n"
        "from litellm import ToolLoopOptions\n"
        "targets = [EvalTarget(Harness.TOOL_LOOP, 'openai/controlled', label='stable',\n"
        "    options=ToolLoopOptions(MappingProxyType({'temperature': float(os.environ['TEST_TEMPERATURE']),\n"
        "        'max_tokens': int(os.environ['TEST_MAX_TOKENS'])})))]\n"
        "run_options = {**run_options, 'api_key': os.environ['TEST_ROTATING_KEY']}\n"
        "tasks = [EvalTask('t', 'Classify', check=PredicateCheck(lambda result: True))]\n",
    )
    original = {
        **os.environ,
        "TEST_TEMPERATURE": "0.1",
        "TEST_MAX_TOKENS": "10",
        "TEST_ROTATING_KEY": "first-test-value",
    }
    baseline = run_cli(tmp_path, "run", "--config", "evals.py", "--fast", env=original)
    assert baseline.returncode == 0, baseline.stderr
    directory, _ = attempts(tmp_path)
    rotated = {**original, "TEST_ROTATING_KEY": "second-test-value"}
    unchanged = run_cli(tmp_path, "resume", directory.name, "--fast", env=rotated)
    assert unchanged.returncode == 0, unchanged.stderr
    for changed in ({**rotated, "TEST_TEMPERATURE": "0.2"}, {**rotated, "TEST_MAX_TOKENS": "20"}):
        rejected = run_cli(tmp_path, "resume", directory.name, "--fast", env=changed)
        assert rejected.returncode == 2
        assert "config" in rejected.stderr.casefold()


def test_sdk_result_mapping_preserves_stopped_run_and_unknown_judge_metrics():
    from litellm import CheckResult, EvalRun, EvalScorecard, EvalStatus
    from litellm.harness import Result, Usage

    from lens_cli.sdk import _result

    result = Result("partial", None, [], [], Usage(5, 3, 1), 0.25, "max_turns", "session")
    stopped = EvalRun("run", "task", "target", 2, EvalStatus.FAIL, result, None, 2.5, "max turns")
    mapped = _result(stopped)
    expected = EvalScorecard("suite", (stopped,)).summaries[0]
    assert mapped.stop_reason == "max_turns"
    assert mapped.duration_seconds == stopped.duration
    assert mapped.metrics_complete == expected.metrics_complete
    assert mapped.judge_cost == expected.judge_cost
    assert mapped.judge_model_calls == expected.judge_usage.calls
    failed_judge = EvalRun(
        "run",
        "task",
        "target",
        2,
        EvalStatus.ERROR,
        result,
        CheckResult(False, "judge unavailable", judge_cost=None, error="judge unavailable"),
        2.5,
        "judge unavailable",
    )
    unknown = _result(failed_judge)
    assert unknown.cost == result.cost
    assert unknown.judge_cost is None
    assert unknown.judge_input_tokens is None
    assert unknown.judge_output_tokens is None
    assert unknown.judge_model_calls is None
    assert not unknown.metrics_complete


def test_sdk_selected_target_resumes_without_repeating_completed_attempts(tmp_path, model_server):
    endpoint, requests = model_server
    write_sdk_config(
        tmp_path,
        endpoint,
        "import os\n"
        "targets = [EvalTarget(Harness.TOOL_LOOP, 'openai/controlled', label='first'),\n"
        "    EvalTarget(Harness.TOOL_LOOP, os.environ.get('TEST_SELECTED_MODEL', 'openai/controlled'),\n"
        "        label='second')]\n"
        "tasks = [EvalTask('t', 'Classify', check=PredicateCheck(lambda result: True))]\n",
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--only", "second", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert len(saved) == 1
    assert saved[0]["target"]["label"] == "second"
    before = (directory / "attempts.jsonl").read_bytes()
    resumed = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert resumed.returncode == 0, resumed.stderr
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert requests.qsize() == 1
    changed = run_cli(
        tmp_path,
        "resume",
        directory.name,
        "--fast",
        env={**os.environ, "TEST_SELECTED_MODEL": "openai/changed"},
    )
    assert changed.returncode == 2
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert requests.qsize() == 1


@pytest.mark.parametrize("recursive", [False, True])
def test_sdk_fingerprints_preserve_stop_strings_and_recursive_callable_tools(tmp_path, model_server, recursive):
    endpoint, requests = model_server
    tool = (
        "def build():\n"
        "    def lookup(value: str) -> str:\n"
        "        return lookup('resolved') if value == 'billing' else 'billing'\n"
        "    return lookup\n"
        "lookup = build()\n"
        if recursive
        else "def lookup(value: str) -> str:\n    return value\n"
    )
    write_sdk_config(
        tmp_path,
        endpoint,
        "from litellm import ToolLoopOptions\n"
        + tool
        + "targets = [EvalTarget(Harness.TOOL_LOOP, 'openai/controlled', tools=(lookup,),\n"
        "    options=ToolLoopOptions({'stop': ['http://[']}))]\n"
        "tasks = [EvalTask('t', 'USE_TOOL', check=PredicateCheck(lambda result: result.text == 'billing'))]\n",
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert saved[0]["outcome"] == {"status": "pass"}
    assert len(saved[0]["tool_calls"]) == 1
    observed = tuple(requests.get_nowait() for _ in range(requests.qsize()))
    assert observed and all(payload["stop"] == ["http://["] for payload, _ in observed)
    before = (directory / "attempts.jsonl").read_bytes()
    resumed = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert resumed.returncode == 0, resumed.stderr
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert requests.qsize() == 0


@pytest.mark.parametrize("default_argument", [False, True])
def test_resume_rejects_changed_predicate_data_named_expected_key(tmp_path, model_server, default_argument):
    endpoint, requests = model_server
    predicate = (
        "lambda result, expected_key=expected_key: result.text == expected_key"
        if default_argument
        else "lambda result: result.text == expected_key"
    )
    write_sdk_config(
        tmp_path,
        endpoint,
        "import os\n"
        "def grade(expected_key):\n"
        f"    return PredicateCheck({predicate})\n"
        "tasks = [EvalTask('t', 'Classify',\n"
        "    check=grade(os.environ.get('TEST_EXPECTED_CATEGORY', 'billing')))]\n",
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, _ = attempts(tmp_path)
    before = (directory / "attempts.jsonl").read_bytes()
    changed = run_cli(
        tmp_path,
        "resume",
        directory.name,
        "--fast",
        env={**os.environ, "TEST_EXPECTED_CATEGORY": "technical"},
    )
    assert changed.returncode == 2
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert requests.qsize() == 1


def test_resume_rejects_a_changed_scalar_global_read_by_a_check(tmp_path, model_server):
    endpoint, requests = model_server
    write_sdk_config(
        tmp_path,
        endpoint,
        "import os\n"
        "EXPECTED = os.environ.get('TEST_EXPECTED_CATEGORY', 'billing')\n"
        "tasks = [EvalTask('t', 'Classify',\n"
        "    check=PredicateCheck(lambda result: result.text == EXPECTED))]\n",
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, _ = attempts(tmp_path)
    before = (directory / "attempts.jsonl").read_bytes()
    changed = run_cli(
        tmp_path,
        "resume",
        directory.name,
        "--fast",
        env={**os.environ, "TEST_EXPECTED_CATEGORY": "technical"},
    )
    assert changed.returncode == 2
    assert "Eval config changed" in changed.stderr
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert requests.qsize() == 1


def test_sdk_header_only_csv_is_rejected_before_creating_a_run(tmp_path, model_server):
    endpoint, requests = model_server
    (tmp_path / "tasks.csv").write_text("id,prompt,expected\n")
    write_sdk_config(tmp_path, endpoint, "")
    rejected = run_cli(tmp_path, "run", "tasks.csv", "--config", "evals.py", "--fast")
    assert rejected.returncode == 2
    assert not tuple((tmp_path / ".lens/runs").glob("*"))
    assert requests.qsize() == 0


def write_custom_config(root, endpoint, *, mixed=True, options="", task_source=None, agent_body=None):
    path = root / "evals.py"
    path.write_text(
        "import sys\n"
        "from pathlib import Path\n"
        "from litellm import (AgentInput, AgentOutput, EvalAgent, EvalTarget, EvalTask,\n"
        "    Harness, PredicateCheck, CommandCheck, JudgeCheck)\n"
        "class MyAgent(EvalAgent):\n"
        "    async def run(self, task: AgentInput) -> AgentOutput:\n"
        + (agent_body or "        return AgentOutput('billing')\n")
        + "custom = MyAgent('my-agent')\n"
        + (
            "targets = [custom, EvalTarget(Harness.TOOL_LOOP, 'openai/controlled', label='tool-loop')]\n"
            if mixed
            else "targets = [custom]\n"
        )
        + f"run_options = {{'api_base': {endpoint!r}, 'api_key': 'controlled-test', {options}}}\n"
        + (
            "tasks = [EvalTask('t', 'Classify', check=PredicateCheck(lambda result: result.text == 'billing'))]\n"
            if task_source is None
            else task_source
        )
    )
    return path


def test_custom_agent_and_sdk_target_share_tasks_checks_and_unknown_metrics(tmp_path, model_server):
    endpoint, requests = model_server
    write_custom_config(tmp_path, endpoint)
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--trials", "2", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert len(saved) == 4
    custom = tuple(item for item in saved if item["target"]["label"] == "my-agent")
    assert len(custom) == 2
    assert all(item["target"] == {"harness": "custom", "model": None, "label": "my-agent"} for item in custom)
    assert all(item["outcome"] == {"status": "pass"} for item in custom)
    for item in custom:
        assert item["cost"] is None
        assert item["input_tokens"] is None
        assert item["output_tokens"] is None
        assert item["model_calls"] is None
        assert item["tool_calls"] is None
        assert not item["metrics_complete"]
    assert requests.qsize() == 2
    report = json.loads((directory / "summary.json").read_text())
    custom_stats = next(stat for stat in report["stats"] if stat["target"]["label"] == "my-agent")
    assert custom_stats["model_cost"] is None
    assert custom_stats["tool_calls_per_case"] is None


def test_custom_agent_uses_prepared_workspace_for_setup_and_command_check(tmp_path, model_server):
    endpoint, requests = model_server
    write_custom_config(
        tmp_path,
        endpoint,
        mixed=False,
        agent_body=(
            "        result = await task.sandbox.run((sys.executable, '-c',\n"
            "            \"from pathlib import Path; assert Path('input.txt').read_text() == 'ready'; "
            "Path('result.txt').write_text('billing')\"))\n"
            "        if result.exit_code != 0:\n"
            "            raise RuntimeError(result.stderr)\n"
            "        return AgentOutput('billing')\n"
        ),
        task_source=(
            "tasks = [EvalTask('workspace', 'Classify', files={'input.txt': 'initial'},\n"
            "    setup=((sys.executable, '-c',\n"
            "        \"from pathlib import Path; Path('input.txt').write_text('ready')\"),),\n"
            "    check=CommandCheck((sys.executable, '-c',\n"
            "        \"from pathlib import Path; assert Path('result.txt').read_text() == 'billing'\")))]\n"
        ),
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    _, saved = attempts(tmp_path)
    assert saved[0]["outcome"] == {"status": "pass"}
    assert "exit 0" in saved[0]["check_detail"]
    assert not (tmp_path / "result.txt").exists()
    assert requests.qsize() == 0


def test_selecting_custom_agent_preserves_sdk_checks_and_resume(tmp_path, model_server):
    endpoint, requests = model_server
    write_custom_config(
        tmp_path,
        endpoint,
        agent_body=(
            "        with Path('calls.txt').open('a') as calls:\n"
            "            calls.write(str(task.trial) + '\\n')\n"
            "        return AgentOutput('billing')\n"
        ),
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--only", "my-agent", "--trials", "2", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert len(saved) == 2
    assert all(item["target"]["label"] == "my-agent" for item in saved)
    assert sorted((tmp_path / "calls.txt").read_text().splitlines()) == ["1", "2"]
    before = (directory / "attempts.jsonl").read_bytes()
    resumed = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert resumed.returncode == 0, resumed.stderr
    assert (directory / "attempts.jsonl").read_bytes() == before
    assert sorted((tmp_path / "calls.txt").read_text().splitlines()) == ["1", "2"]
    assert requests.qsize() == 0


def test_custom_agent_implementation_change_rejects_resume_without_inspecting_client_state(tmp_path, model_server):
    endpoint, requests = model_server
    path = write_custom_config(tmp_path, endpoint, mixed=False)
    path.write_text(
        path.read_text()
        + (
            "import os\n"
            "class OtherAgent(EvalAgent):\n"
            "    async def run(self, task: AgentInput) -> AgentOutput:\n"
            "        return AgentOutput('technical')\n"
            "custom.client = {'credential': os.environ.get('TEST_CLIENT_STATE', 'initial')}\n"
            "targets = [OtherAgent('my-agent') if os.environ.get('TEST_AGENT_IMPL') else custom]\n"
        )
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, _ = attempts(tmp_path)
    unchanged = run_cli(
        tmp_path, "resume", directory.name, "--fast", env={**os.environ, "TEST_CLIENT_STATE": "rotated"}
    )
    assert unchanged.returncode == 0, unchanged.stderr
    rejected = run_cli(tmp_path, "resume", directory.name, "--fast", env={**os.environ, "TEST_AGENT_IMPL": "other"})
    assert rejected.returncode == 2
    assert requests.qsize() == 0


@pytest.mark.parametrize(
    ("body", "expected_code", "status"),
    [
        ("        return AgentOutput('technical')\n", 1, "fail"),
        ("        raise RuntimeError('custom agent failed')\n", 2, "error"),
    ],
)
def test_custom_agent_failures_keep_sdk_sibling_results(tmp_path, model_server, body, expected_code, status):
    endpoint, requests = model_server
    write_custom_config(tmp_path, endpoint, agent_body=body)
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == expected_code, completed.stderr
    _, saved = attempts(tmp_path)
    assert len(saved) == 2
    outcomes = {item["target"]["label"]: item["outcome"]["status"] for item in saved}
    assert outcomes == {"my-agent": status, "tool-loop": "pass"}
    assert requests.qsize() == 1


def test_custom_agent_supports_csv_default_predicate_and_reported_metrics(tmp_path, model_server):
    endpoint, requests = model_server
    (tmp_path / "tasks.csv").write_text("id,prompt,expected\nt,Classify,billing\n")
    write_custom_config(
        tmp_path,
        endpoint,
        mixed=False,
        agent_body=(
            "        from litellm.harness import Usage\n"
            "        return AgentOutput('billing', cost=0.25, usage=Usage(5, 3, 1), events=())\n"
        ),
        task_source="",
    )
    completed = run_cli(tmp_path, "run", "tasks.csv", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    _, saved = attempts(tmp_path)
    assert saved[0]["outcome"] == {"status": "pass"}
    assert (saved[0]["cost"], saved[0]["input_tokens"], saved[0]["output_tokens"], saved[0]["model_calls"]) == (
        0.25,
        5,
        3,
        1,
    )
    assert saved[0]["tool_calls"] == []
    assert saved[0]["metrics_complete"]
    assert requests.qsize() == 0


def test_custom_agent_uses_sdk_judge_and_preserves_judge_error_metrics(tmp_path, model_server):
    endpoint, requests = model_server
    write_custom_config(
        tmp_path,
        endpoint,
        mixed=False,
        task_source=(
            "tasks = [EvalTask('t', 'Classify',\n"
            "    check=JudgeCheck('Answer must be billing', 'openai/controlled'))]\n"
        ),
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 2, completed.stderr
    _, saved = attempts(tmp_path)
    assert saved[0]["outcome"]["status"] == "error"
    assert saved[0]["check_error"] == "Judge returned an invalid verdict"
    assert saved[0]["judge_model_calls"] == 1
    assert saved[0]["cost"] is None
    assert saved[0]["model_calls"] is None
    assert saved[0]["tool_calls"] is None
    assert requests.qsize() == 1


def test_custom_agent_fingerprint_does_not_inspect_a_captured_client(tmp_path, model_server):
    endpoint, requests = model_server
    path = write_custom_config(tmp_path, endpoint, mixed=False)
    path.write_text(
        path.read_text()
        + (
            "def build():\n"
            "    class Client(dict):\n"
            "        def items(self):\n"
            "            raise RuntimeError('client state must not be inspected')\n"
            "        def answer(self, prompt):\n"
            "            return 'billing'\n"
            "    client = Client()\n"
            "    class CapturedAgent(EvalAgent):\n"
            "        async def run(self, task: AgentInput) -> AgentOutput:\n"
            "            return AgentOutput(client.answer(task.prompt))\n"
            "    return CapturedAgent('my-agent')\n"
            "targets = [build()]\n"
        )
    )
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, saved = attempts(tmp_path)
    assert saved[0]["outcome"] == {"status": "pass"}
    resumed = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert resumed.returncode == 0, resumed.stderr
    assert requests.qsize() == 0


def test_imported_custom_agent_constructor_change_rejects_resume(tmp_path, model_server):
    endpoint, requests = model_server
    implementation = tmp_path / "agent_impl.py"
    implementation.write_text(
        "from litellm import AgentInput, AgentOutput, EvalAgent\n"
        "class ImportedAgent(EvalAgent):\n"
        "    def __init__(self):\n"
        "        super().__init__('my-agent')\n"
        "        self.answer = 'billing'\n"
        "    async def run(self, task: AgentInput) -> AgentOutput:\n"
        "        return AgentOutput(self.answer)\n"
    )
    path = write_custom_config(tmp_path, endpoint, mixed=False)
    path.write_text(path.read_text() + "from agent_impl import ImportedAgent\ntargets = [ImportedAgent()]\n")
    completed = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert completed.returncode == 0, completed.stderr
    directory, _ = attempts(tmp_path)
    before = (directory / "attempts.jsonl").read_bytes()
    implementation.write_text(implementation.read_text().replace("'billing'", "'technical'"))
    rejected = run_cli(tmp_path, "resume", directory.name, "--fast")
    assert rejected.returncode == 2
    assert (directory / "attempts.jsonl").read_bytes() == before
    fresh = run_cli(tmp_path, "run", "--config", "evals.py", "--fast")
    assert fresh.returncode == 1, fresh.stderr
    assert requests.qsize() == 0
