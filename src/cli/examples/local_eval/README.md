# Local eval example

Classifies 20 support tickets as billing, technical or sales. Eight of them only give a ticket id, so the agent has to call `lookup_ticket` first. `evals.py` runs the same tasks across two harnesses and two models:

| Harness | Model |
| --- | --- |
| LiteLLM tool loop (`agent.litellm_agent`) | `claude-sonnet-5-5` |
| LiteLLM tool loop | `gpt-5.4-mini` |
| OpenAI Agents SDK (`agent.openai_agents_agent`) | `gpt-5.4-mini` |

These make real model calls, so export `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` first. A full run makes 60 case attempts and incurs provider charges

```bash
cd src/cli/examples/local_eval
uv run --project ../../../sdk --with litellm --with openai-agents lens run tasks.csv --config evals.py
```

Results are saved under `.lens/runs/<run_id>/`. Pick up an interrupted run, or print a saved report again:

```bash
uv run --project ../../../sdk --with litellm --with openai-agents lens resume <run_id>
uv run --project ../../../sdk lens show
```

Add `--fast` to print the report without the live progress view, for example in CI logs
