# Compare models and harnesses with Lens

Run the same tasks through different models, harnesses and your own agent, then compare pass rate, cost and latency. This walkthrough compares GPT-5.6 Luna and Sol in LiteLLM Tool Loop and Codex, then adds a custom agent to the same report.

## 1. Set up

This walkthrough uses the source preview in this checkout. These commands are not in a published `lens-evals` package yet.

You need Python 3.11+, [uv](https://docs.astral.sh/uv/getting-started/installation/), [Rust 1.99.0](https://www.rust-lang.org/tools/install), and an [OpenAI API key](https://platform.openai.com/api-keys) with access to both models. The shell commands below use Bash or Zsh.

From the Lens repository root:

```bash
uv sync --project src/sdk --extra agent-evals --frozen
source src/sdk/.venv/bin/activate
```

Install the [Codex CLI](https://github.com/openai/codex#installing-and-running-codex-cli) if you do not have it. With Node.js and npm installed:

```bash
npm install -g @openai/codex
codex --version
```

Set `OPENAI_API_KEY` through your secret manager or enter it below. `read` hides your input:

```bash
read -rs OPENAI_API_KEY
export OPENAI_API_KEY
```

## 2. Choose your models, harnesses and tasks

Save this as `evals.py` in the repository root:

```python
from itertools import product

from litellm import EvalTarget, EvalTask, Harness, PredicateCheck

models = {
    "luna": "openai/gpt-5.6-luna",
    "sol": "openai/gpt-5.6-sol",
}
harnesses = [Harness.TOOL_LOOP, Harness.CODEX]

targets = [
    EvalTarget(harness, model, label=f"{name}/{harness.value}")
    for (name, model), harness in product(models.items(), harnesses)
]

tasks = [
    EvalTask(
        "billing",
        "Classify this ticket: I was charged twice. "
        "Reply only billing, technical, or sales.",
        check=PredicateCheck(
            lambda result: result.text.strip().casefold() == "billing"
        ),
    )
]

run_options = {"max_turns": 4, "timeout": 120}
```

You define the task and its pass check once. `product` pairs each model with each harness, giving you four targets: `luna/tool_loop`, `luna/codex`, `sol/tool_loop` and `sol/codex`. To compare another model or supported harness, change `models` or `harnesses`.

The check passes when the agent finishes and answers `billing`. The SDK runs each attempt in a fresh workspace and applies the same check to each target.

## 3. Run the comparison

From the same directory:

```bash
lens run --config evals.py --trials 2
```

This runs eight attempts: one task × two models × two harnesses × two trials. The attempts make real model calls and incur API charges.

Watch the progress, then compare the four target rows in the report. Check `PASS`, `FAIL` and `ERR` before comparing `COST` and `P50` latency. A failed check counts as a failure; an execution or judging problem counts as an error. Unavailable metrics stay unknown.

For logs or CI, add `--fast` to print the report without the live progress view. The command exits `0` when the attempts pass, `1` when checks fail, or `2` when execution or judging errors occur.

## 4. Add your own agent

To compare an agent you already run, inherit from `EvalAgent` and implement `run`. Add this to the end of `evals.py`:

```python
from litellm import (
    AgentInput,
    AgentOutput,
    EvalAgent,
    acompletion,
)


class MyAgent(EvalAgent):
    async def run(self, task: AgentInput) -> AgentOutput:
        response = await acompletion(
            model=models["luna"],
            messages=[
                {"role": "user", "content": task.prompt},
            ],
        )
        text = response.choices[0].message.content or ""
        return AgentOutput(text=text)


targets.append(MyAgent("my-agent"))
```

Replace the body of `run` with your agent's own code. You can call a service, run a tool loop, or use another framework. Return its answer as `AgentOutput(text=...)`.

You receive the task prompt, trial number, metadata and prepared workspace through `AgentInput`. Use `task.sandbox` for file or command work so the evaluator can check the same workspace. Return cost and usage when your agent exposes them; otherwise those metrics stay unknown. Keep attempt state inside `run`, since the evaluator can call the same instance concurrently.

Run the same command again:

```bash
lens run --config evals.py --trials 2
```

You now get five target rows and ten attempts. `my-agent` uses the same task and pass check as the four model/harness targets. The evaluator handles setup, checks and workspace cleanup for your agent too. The timeout applies to its `run` method; the harness-specific `max_turns` option does not control your agent's loop.

## 5. Inspect, repeat or resume

Print the latest saved report:

```bash
lens show
```

Find the inputs, outputs and check details in `.lens/runs/<run_id>/attempts.jsonl`. The terminal prints the run ID and save location.

Rerun one target while you work on a failure:

```bash
lens run --config evals.py --only luna/codex --trials 2
```

If you stop a run with Ctrl-C, continue its unfinished attempts with the run ID:

```bash
lens resume <run_id>
```

Keep the config and tasks unchanged when resuming. Start a new run after changing agent code, client settings or remote services too.

Replace the sample ticket with your own tasks and pass checks. See the [CSV example](examples/local_eval/README.md) for tasks that call a Python tool. To use your LiteLLM Gateway, set the [gateway connection](https://docs.litellm.ai/docs/harness/gateway) and replace the model strings with `litellm_proxy/<model-group>` names supported by your chosen harnesses.
