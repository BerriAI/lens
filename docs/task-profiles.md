# Automatic task profiles

Lens groups tasks locally in the background. It embeds a short input preview and tool names with quantized `all-MiniLM-L6-v2`, then compares the result with up to 64 cluster representatives. The first task supplies a stable cluster ID and a short keyword label. This path makes no paid model calls

The Profiles chart and investigation filters use the saved assignment. Recorded `task_category` or `scenario` values take precedence. Refreshing the chart does not run inference. Empty inputs, unsupported scripts and unmatched tasks after the cluster limit appear as `Other / uncertain`

The default model supports English. Latin script is an input guard, not language detection; grouping quality for other Latin-script languages is unverified. A multilingual model was rejected as the default after its cold-start RSS exceeded 1 GiB. Cluster names are extracted keywords, and similar tasks can remain in separate groups. This is an exploratory grouping, not a calibrated task classifier

## Resource limits

Each server starts one separate worker process after storage is ready. It reads one run at a time, waits for a 30-second quiet period, reads at most eight input previews, and bounds the combined descriptor to 1,024 UTF-8 bytes and 128 model tokens. Inference uses one thread and batch size one. Tokenizer parallelism and inference thread spinning are disabled

After each item, the worker sleeps for four times the elapsed processing time, with a 250 ms minimum. This limits sustained work to a 20% duty cycle; it is not a hard CPU quota. The worker sleeps 30 seconds when caught up and five minutes after an error. There is no in-memory backfill queue or cache proportional to trace history

The parent samples worker RSS once per second and stops it above 512 MiB, then waits five minutes before restarting. This watchdog can observe a transient overshoot; use operating-system or container limits when a hard memory ceiling is required. The model stays outside the HTTP server process. On Linux with readable cgroup memory counters, Lens defers worker startup with less than 320 MiB spare memory and stops it when spare memory drops below 64 MiB. These are sampled guards and cannot prevent an external workload from exhausting memory between samples. Background database queries use one thread, a 64 MiB query memory limit, a three-second execution limit and response limits of 16 KiB for task inputs and 512 KiB for cluster vectors. Larger queries defer rather than increase these limits

A local macOS ARM64 probe processed 6,000 distinct English descriptions in 59.18 seconds without production pauses, including model initialization from cached weights. Peak RSS was 235,421,696 bytes (224.5 MiB). These figures cover the classifier process, not the whole application or ClickHouse, and are not a throughput guarantee

Assignments are scoped to the trace identity. Cluster representatives are also scoped to team, key, agent, service and trace ownership so keyword labels do not cross user boundaries. Both tables follow trace retention. Each replica has its own resource allowance; simultaneous replicas can race to create an extra representative at the cluster boundary. Reads remain bounded and existing assignments remain stable

## Model cache

The container image includes the model cache. Standalone binaries download approximately 24 MB of weights on first use and reuse the cache. Set `LENS_PROFILE_MODEL_CACHE` to a writable persistent directory, or prepopulate it for an offline deployment. Without this variable, Lens uses `XDG_CACHE_HOME/lens/task-profiles` or the temporary directory. Failed downloads defer grouping without blocking ingestion or investigations

Set `LENS_TASK_PROFILES_ENABLED=false` to stop automatic grouping. Existing assignments and recorded categories remain available. Model weights retain their upstream Apache-2.0 license

To reproduce the classifier measurement with a populated cache, build the example and run it under your platform's resource monitor:

```sh
cargo build --locked --manifest-path src/worker/Cargo.toml -p lens-profiles --example resource_probe
TOKENIZERS_PARALLELISM=false /usr/bin/time -l src/worker/target/debug/examples/resource_probe 6000
```

The `-l` flag is macOS-specific. On Linux, use `/usr/bin/time -v`. The probe runs without throttling to measure classification cost; the server worker applies the pauses above

## Profiles view

The Profiles tab shows one agent's runs in the selected time range as a bubble map, a table ranked by runs or spend, and a heatmap of profiles against model deployments. Each run is counted once and keeps one profile. A recorded `task_category` or `scenario` always wins. Otherwise a run takes the agent's reviewed task when a frozen enhanced taxonomy exists, and the automatic group when it does not. Runs the local worker could not group stay `Other / uncertain`, and runs it has not reached yet stay `Unclassified`

Requests are the agent's own model calls inside those runs. Lens resolves them with the same strict ledger matching the trace view uses, so a call counts once even when it was exported twice, and calls that share a ledger record are one request. A request that other agents in the same run also exported is counted as shared and charged to neither agent. Cost is known only when every ledger record behind a request is priced. Otherwise the view says how many requests the spend rests on and never shows the missing part as zero. Retries charge the whole request. When the attempts used different deployments, the request appears under `multiple deployments` instead of being priced as either one. The gateway's compact cost records have no delivery identity, so the view does not report deliveries

Heatmap columns are gateway deployment IDs recorded on the ledger. Older ledger records without one appear under their model group, and calls without a matching record appear as unmatched. Callers who can read every team also see configured deployments without traffic, with the capability flags the gateway declares. Compatibility compares those flags with a reviewed task's required capabilities. It is metadata only and says nothing about quality. Evaluation results are not collected yet

Each request reads at most 5,000 runs and 50,000 spans. When a range holds more, the view labels the figures as partial and shows how many runs they cover

## Enhanced discovery

Enhanced discovery is optional and makes paid model calls through your LiteLLM gateway. It is off until all four model aliases are set, and only proxy admins can start it, review its draft, freeze it or reset it. Its names and examples come from every team's runs, so only callers who can read every team see them

| Variable | Used for | Example |
| --- | --- | --- |
| `LENS_PROFILE_DISTILL_MODEL` | One-line intent per run | `claude-haiku-5-5` |
| `LENS_PROFILE_EMBEDDING_MODEL` | Embeddings of those intents | a Qwen3 embedding deployment |
| `LENS_PROFILE_SYNTHESIS_MODEL` | Naming each group | `claude-sonnet-5-5` |
| `LENS_PROFILE_DECISION_MODEL` | Ongoing classification | `typesafe/jev-latest` |

The aliases must exist on the gateway configured by `LENS_GATEWAY_API_BASE` and `LENS_GATEWAY_API_KEY`. The decision alias must be a Jev deployment (`typesafe` or a gateway `decisions` deployment). Lens refuses any other provider instead of falling back to a chat model

Discovery follows LLooM's concept induction (Lam et al., CHI 2024, https://arxiv.org/abs/2404.12259, https://github.com/michelle123lam/lloom). It reads up to 500 recent unrecorded runs of the agent and distills each into a one-line intent so codebase vocabulary does not drive the groups. It embeds the intents (Qwen3 embeddings, https://github.com/QwenLM/Qwen3-Embedding), reduces them with UMAP tuned for density and clusters them with HDBSCAN, as the UMAP clustering guide advises (https://umap-learn.readthedocs.io/en/latest/clustering.html, https://scikit-learn.org/stable/modules/generated/sklearn.cluster.HDBSCAN.html). The Rust ports are `umap-rs` and `hdbscan`. A separate two-dimensional UMAP places the discovery sample on the map. That layout comes from one stochastic run and is stored, not recomputed. Runs added later are drawn around their task and labeled as such. A model then names each of the twelve largest groups, with inclusion and exclusion criteria and required capabilities

An admin reviews the draft, corrects names, criteria and requirements, and freezes it. Freezing starts ongoing classification with Jev Choice (https://docs.typesafe.ai/primitives/choice): each unrecorded run, including the discovery sample and older runs, is compared with the frozen definitions, and a companion Noul question asks whether the run fits any of them at all. A run Jev places outside every definition becomes `Unknown`. Classification works in batches of 20 under a lease shared by replicas, and a run that fails three times is left pending. Resetting or rediscovering starts a new generation, so assignments from an earlier one never come back

This adaptation keeps LLooM's distill, cluster and synthesize steps but scores runs with Jev instead of a generative judge, and limits a discovery to one agent and 500 runs. Grouping quality has not been measured against human labels

## Sample data

`bash deploy/profiles-demo/demo.sh` builds Lens from this checkout if needed, starts it with its own ClickHouse on port 4320, and seeds three agents through the normal ingestion path. The sample covers recorded categories, a multi-agent run, retries with a charged failure, missing cost and latency, unmatched calls, and ledger records without a deployment ID. It also stores a frozen and a draft sample taxonomy with precomputed assignments, so the whole flow works without model calls. The seed only replaces rows of its own demo team and refuses to run if a `lens-demo-*` agent already has a non-sample taxonomy. Use `password` to print the admin token, `stop` to keep the data, and `reset` to delete it
