import asyncio
import json
import subprocess
import sys

import pytest

from lens.errors import ConfigurationError
from lens.local import AgentOutput, TaskInput, Verdict, attempt, default_check, load_config

TASK = json.dumps({"id": "t1", "prompt": "Classify T-1", "expected": "Billing", "metadata": {"tier": "gold"}})


def write_config(root, body):
    path = root / "evals.py"
    (root / "my_agent.py").write_text(
        "from lens import AgentOutput, ToolCall\n"
        "seen = []\n"
        "async def answer(task):\n"
        "    seen.append((task.id, task.model, task.harness, dict(task.metadata)))\n"
        "    return AgentOutput(text=' billing ', cost=0.25, judge_cost=0.05,\n"
        "        tool_calls=(ToolCall('lookup', {'id': 1}), ToolCall('lookup', {'id': 2}, error=True)))\n"
        "async def explode(task):\n"
        "    raise TimeoutError('upstream timed out')\n"
    )
    path.write_text("from lens import Target\nfrom my_agent import answer, explode\n" + body)
    return path


def run(config, target):
    return json.loads(asyncio.run(attempt(config, TASK, json.dumps(target))))


def test_config_imports_sibling_modules_and_records_tool_calls(tmp_path):
    config = load_config(
        write_config(tmp_path, "targets = [Target(harness='litellm', model='gpt-5.4-mini', run=answer)]\n")
    )
    result = run(config, {"harness": "litellm", "model": "gpt-5.4-mini"})
    assert result["outcome"] == {"status": "pass"}
    assert (result["cost"], result["judge_cost"], result["output"]) == (0.25, 0.05, " billing ")
    assert result["tool_calls"] == [
        {"name": "lookup", "arguments": {"id": 1}, "error": False},
        {"name": "lookup", "arguments": {"id": 2}, "error": True},
    ]
    import my_agent

    assert my_agent.seen[-1] == ("t1", "gpt-5.4-mini", "litellm", {"tier": "gold"})


def test_agent_exception_becomes_error_outcome(tmp_path):
    config = load_config(write_config(tmp_path, "targets = [Target(harness='custom', model='m', run=explode)]\n"))
    result = run(config, {"harness": "custom", "model": "m"})
    assert result["outcome"] == {"status": "error", "kind": "TimeoutError", "message": "upstream timed out"}
    assert result["tool_calls"] == []


def test_custom_check_failure_carries_reason(tmp_path):
    config = load_config(
        write_config(
            tmp_path,
            "from lens import Verdict\n"
            "targets = [Target(harness='h', model='m', run=answer)]\n"
            "async def check(task, output):\n"
            "    return Verdict(False, 'needed ' + task.expected)\n",
        )
    )
    assert run(config, {"harness": "h", "model": "m"})["outcome"] == {"status": "fail", "reason": "needed Billing"}


def test_boolean_check_is_accepted(tmp_path):
    config = load_config(
        write_config(
            tmp_path,
            "targets = [Target(harness='h', model='m', run=answer)]\ncheck = lambda task, output: False\n",
        )
    )
    assert run(config, {"harness": "h", "model": "m"})["outcome"]["status"] == "fail"


@pytest.mark.parametrize(
    ("expected", "text", "passed"),
    [("Billing", "  billing\n", True), ("billing", "sales", False), (None, "anything", True)],
)
def test_default_check_matches_expected_case_insensitively(expected, text, passed):
    task = TaskInput(id="t", prompt="p", expected=expected, metadata={}, harness="h", model="m")
    assert default_check(task, AgentOutput(text=text)).passed is passed


def test_default_check_explains_mismatch():
    task = TaskInput(id="t", prompt="p", expected="billing", metadata={}, harness="h", model="m")
    assert default_check(task, AgentOutput(text="sales")) == Verdict(False, "expected 'billing', got 'sales'")


@pytest.mark.parametrize(
    ("body", "message"),
    [
        ("targets = []\n", "must define targets"),
        (
            "targets = [Target(harness='h', model='m', run=answer), Target(harness='h', model='m', run=answer)]\n",
            "Duplicate targets in evals.py: h/m",
        ),
        ("targets = [Target(harness='h', model='m', run=answer)]\ncheck = 3\n", "check in evals.py must be a function"),
    ],
)
def test_invalid_configs_are_rejected(tmp_path, body, message):
    with pytest.raises(ConfigurationError, match=message):
        load_config(write_config(tmp_path, body))


def test_unknown_target_is_reported_as_error_outcome(tmp_path):
    config = load_config(write_config(tmp_path, "targets = [Target(harness='h', model='m', run=answer)]\n"))
    result = run(config, {"harness": "h", "model": "other"})
    assert result["outcome"]["kind"] == "ConfigurationError"


def test_config_supports_dataclasses_with_deferred_annotations(tmp_path):
    path = tmp_path / "evals.py"
    path.write_text(
        "from __future__ import annotations\n"
        "from dataclasses import dataclass\n"
        "from lens import AgentOutput, Target\n"
        "@dataclass(frozen=True)\n"
        "class Answer:\n"
        "    text: str\n"
        "async def answer(task):\n"
        "    return AgentOutput(text=Answer('billing').text)\n"
        "targets = [Target(harness='h', model='m', run=answer)]\n"
    )
    config = load_config(path)
    assert run(config, {"harness": "h", "model": "m"})["outcome"] == {"status": "pass"}


def test_only_keeps_the_named_targets_in_config_order(tmp_path):
    config = load_config(
        write_config(
            tmp_path,
            "targets = [Target(harness='a', model='x', run=answer), Target(harness='b', model='y', run=answer),\n"
            "    Target(harness='c', model='z', run=answer)]\n",
        ),
        only=("c/z", "a/x"),
    )
    assert [target.key for target in config.targets] == ["a/x", "c/z"]


def test_only_rejects_unknown_targets_and_lists_valid_ones(tmp_path):
    path = write_config(tmp_path, "targets = [Target(harness='a', model='x', run=answer)]\n")
    with pytest.raises(ConfigurationError, match=r"--only b/y is not in evals.py. Targets: a/x"):
        load_config(path, only=("b/y",))


def test_cli_only_filters_native_run_and_preserves_selection_on_resume(tmp_path):
    (tmp_path / "tasks.csv").write_text("id,prompt,expected\nt1,Classify T-1,billing\n")
    (tmp_path / "evals.py").write_text(
        "from pathlib import Path\n"
        "from lens import AgentOutput, Target\n"
        "async def answer(task):\n"
        "    with Path('calls.txt').open('a') as calls:\n"
        "        calls.write(task.harness + '/' + task.model + '\\n')\n"
        "    return AgentOutput(text='billing')\n"
        "targets = [Target(harness='a', model='x', run=answer), Target(harness='b', model='y', run=answer)]\n"
    )
    result = subprocess.run(
        [sys.executable, "-m", "lens.cli", "run", "tasks.csv", "--config", "evals.py", "--fast", "--only", "a/x"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode == 0, result.stderr
    directory = next((tmp_path / ".lens/runs").iterdir())
    meta = json.loads((directory / "meta.json").read_text())
    assert meta["targets"] == [{"harness": "a", "model": "x"}]
    attempts = (directory / "attempts.jsonl").read_text()
    assert json.loads(attempts)["target"] == {"harness": "a", "model": "x"}
    resumed = subprocess.run(
        [sys.executable, "-m", "lens.cli", "resume", directory.name, "--fast"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert resumed.returncode == 0, resumed.stderr
    assert json.loads((directory / "meta.json").read_text())["targets"] == meta["targets"]
    assert (directory / "attempts.jsonl").read_text() == attempts
    assert (tmp_path / "calls.txt").read_text() == "a/x\n"


@pytest.mark.parametrize("module", ["lens_cli.local", "lens.local", "lens_cli.cli"])
def test_imports_and_public_values_work_in_a_fresh_process(tmp_path, module):
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            f"import {module}; import pickle; from lens import AgentOutput; "
            "value = AgentOutput(text='billing'); assert pickle.loads(pickle.dumps(value)) == value",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("missing", [True, False])
def test_sdk_dependency_import_errors_have_install_guidance_without_traceback(tmp_path, missing):
    import os

    blocked = tmp_path / "controlled_imports"
    blocked.mkdir()
    (blocked / "litellm.py").write_text(
        "raise ModuleNotFoundError(\"No module named 'litellm'\", name='litellm')\n"
        if missing
        else "class EvalTarget:\n"
        "    def __init__(self, harness, model):\n"
        "        self.harness, self.model = harness, model\n"
        "EvalRun = EvalScorecard = EvalStatus = EvalTask = PredicateCheck = object\n"
    )
    (tmp_path / "evals.py").write_text("from litellm import EvalTarget\ntargets = [EvalTarget('tool_loop', 'model')]\n")
    env = {**os.environ, "PYTHONPATH": str(blocked) + os.pathsep + os.environ.get("PYTHONPATH", "")}
    rejected = subprocess.run(
        [sys.executable, "-m", "lens.cli", "run", "--config", "evals.py", "--fast"],
        cwd=tmp_path,
        env=env,
        text=True,
        capture_output=True,
        timeout=15,
    )
    assert rejected.returncode == 2
    assert "Cannot import evals.py" in rejected.stderr
    assert "agent-evals extra" in rejected.stderr
    assert "Traceback" not in rejected.stderr
    (tmp_path / "legacy.py").write_text(
        "from lens import Target, AgentOutput\n"
        "async def answer(task):\n    return AgentOutput('billing')\n"
        "targets = [Target('custom', 'model', answer)]\n"
    )
    (tmp_path / "tasks.csv").write_text("id,prompt,expected\nt,Classify,billing\n")
    compatible = subprocess.run(
        [sys.executable, "-m", "lens.cli", "run", "tasks.csv", "--config", "legacy.py", "--fast"],
        cwd=tmp_path,
        env=env,
        text=True,
        capture_output=True,
        timeout=15,
    )
    assert compatible.returncode == 0, compatible.stderr


def test_config_execution_errors_are_not_reclassified_as_missing_dependencies(tmp_path):
    path = tmp_path / "evals.py"
    path.write_text("raise RuntimeError('configuration bug')\n")
    with pytest.raises(RuntimeError, match="configuration bug"):
        load_config(path)


def test_callback_header_only_csv_is_rejected_before_creating_a_run(tmp_path):
    (tmp_path / "tasks.csv").write_text("id,prompt,expected\n")
    write_config(tmp_path, "targets = [Target(harness='custom', model='model', run=answer)]\n")
    rejected = subprocess.run(
        [sys.executable, "-m", "lens.cli", "run", "tasks.csv", "--config", "evals.py", "--fast"],
        cwd=tmp_path,
        text=True,
        capture_output=True,
        timeout=15,
    )
    assert rejected.returncode == 2
    assert not tuple((tmp_path / ".lens/runs").glob("*"))
