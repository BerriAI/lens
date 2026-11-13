# Model optimizations

Open **Lens > Model optimizations** in the gateway. Create a Live, Shadow, or Offline comparison, choose an agent, its current model, and candidates from the gateway catalog. Results survive Lens restarts

Use the Lens server, UI package, and SDK from this change together. Install the SDK from the checked-out source with `pip install './src/sdk[examples]'`; the published preview does not include this API yet. The existing [Lens deployment](../deploy/lens/README.md) must have a working gateway connection and agent tracing

Model discovery and replay use the server's configured gateway key. Admins create and control comparisons; admin viewers can read results. The catalog describes that execution key's access. Live assignments additionally filter candidates against the calling gateway key's permitted models. The gateway enforces inference permissions on every request

## Live

Choose the traffic percentage and either existing 0–10 feedback or an LLM judge. The percentage is the total share assigned to candidates, divided among eligible candidates; the rest remains on the baseline. Pick a judge model and rubric when using an LLM judge. Baseline and candidate judging percentages can differ

Add one model-selection call before starting a traced agent run. Use the same trace ID when exporting its spans, and keep the returned model for that run's model calls:

```python
import os
from lens import Lens

lens = Lens(os.environ["LENS_BASE_URL"], os.environ["LENS_API_KEY"])
assignment = lens.optimizations.assign(
    agent=agent_name,
    baseline=current_model,
    trace_id=trace_id,
)
model = assignment.model
```

`LENS_BASE_URL` is the gateway origin; `LENS_API_KEY` is the caller's existing gateway key. Use `await lens.optimizations.aassign(...)` in async code. Assignment failures return the supplied baseline by default. Set `fail_open=False` to surface setup failures

Start a run and open the comparison to verify that its model and measured cost appear. Feedback joins the trace's existing feedback records. No feedback means no quality score; Lens does not invent one. Live experiments with the same agent, team, and baseline cannot overlap

## Shadow

Choose the traffic percentage, candidates, and judge. Lens samples completed traces automatically and replays the final LLM request. It retains prior recorded context when scoring each final answer and excludes the original final answer from the candidate's judge context

For faithful replay, record the complete Chat Completions request JSON in the final span's `lens.replay.request` attribute. Include messages, tool-call IDs, function definitions, and any sampling or response-format settings. Do not include credentials. Lens removes the original model, streaming flag, and metadata before supplying its own model and internal evaluation identity

Plain recorded text conversations can replay with gateway defaults. Missing tool definitions, multimodal messages, hosted tools, unsupported request settings, parallel final calls, and traces over 500 spans are not silently approximated. Use a full-agent comparison for these cases. A captured request is limited to 1 MB

A Shadow result measures the final call's cost and quality. It does not predict the cost of changing every earlier model call in the agent

## Offline

Select a dataset revision and **Final call** to replay each included case with both the selected baseline and candidate models. Both are fresh requests scored with the same rubric. Dataset requests remain pinned to that revision; editing a saved case invalidates its previous request snapshot and trace source so old inputs cannot silently be replayed

Select **Full agent** when a comparison needs tools, application state, or multiple model calls. Run the task in the environment that already has those dependencies:

```python
import os
from lens import Lens, Run
from my_agent import agent

lens = Lens(os.environ["LENS_BASE_URL"], os.environ["LENS_API_KEY"])

async def task(case, model):
    result = await agent.run(
        case.input,
        model=model,
        trace_attributes={"lens.optimization.request_id": case.meta["request_id"]},
    )
    return Run(trace={"trace_id": result.trace_id})

await lens.optimizations.arun(optimization_id, task=task)
```

Adapt `my_agent` to your agent's interface. Export the trial ID on its root span, use the selected agent name, and export a fresh completed trace. Lens checks the trial ID, creation time, team, agent, and final requested model before accepting the result. The task runs locally; Lens does not call a user-supplied runner URL. Each task has eight minutes to run, followed by up to 90 seconds for trace ingestion. Lost or expired work is reported as uncertain and is not automatically repeated

[The executable example](../src/sdk/examples/optimizations.py) includes a small gateway-backed agent and OTLP export. Set `LENS_BASE_URL`, `LENS_API_KEY`, `LITELLM_PROXY_API_BASE`, `LITELLM_PROXY_API_KEY`, `LENS_TRACE_ENDPOINT`, `LENS_TRACE_API_KEY`, and `LENS_OPTIMIZATION_AGENT` in your shell. Use tracing and inference credentials for the same team and user so gateway spend can join to the trace. Set `LENS_OPTIMIZATION_BASELINE` for `python src/sdk/examples/optimizations.py live`, or `LENS_OPTIMIZATION_ID` for `python src/sdk/examples/optimizations.py offline`. Keep all credential values in your environment or secret manager

This callback path reuses the SDK's `Case` and `Run` contract. Saved HTTP `agent_io` definitions do not yet accept candidate-model overrides

## Read results

The cost/quality plot and model table use measured results. Missing cost or quality remains missing. Provider failures, incomplete answers, fallback models, and judge failures are visible and prevent a migration recommendation. Costs represent serving spend; replay and judging incur additional gateway charges

Recommendations need at least 30 scored observations and a 95% normal-approximation lower bound within the selected score tolerance. Replay uses paired score differences; Live uses independent groups. Feedback also needs at least 50% coverage and similar coverage between groups. These checks do not remove selection bias or make an LLM judge equivalent to user feedback

Pause stops new assignments and queued model calls; a call already in flight may finish. Finish freezes the collected results and marks unfinished work as stopped. An in-flight request can still incur charges. Feedback or spend that arrives after finishing is not added to that snapshot. The initial limits are 100 saved comparisons, eight candidates, 500 included dataset cases, 5,000 Live assignments per comparison, and roughly 4,500 collected result rows for ongoing comparisons. Reaching a sample limit pauses collection. Gateway errors remain visible; create a new comparison to retry an uncertain provider outcome
