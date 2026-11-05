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
