# Investigation benchmark

This example runs the real `pipeline::analyze`, agent tool loop, grouping, exact citation validation and consolidation against synthetic trace pages. It uses the production `AnalysisModels` transport for live model calls. It does not benchmark ClickHouse, the HTTP server, the customer deployment or the remote model server's CPU/GPU memory

The nine fixtures have three material causes: false shipment confirmation after declined payment, a committed purchase above the user's budget, and false readiness after a failed delegated safety check. Six negative controls cover honest failure, an affordable purchase, recovered delegation, minor writing style, an archived prompt-injection example and a normal outcome. The operational evidence is near the end of the third of four long spans. Background inventory text is repetitive, so these are controlled long-context stress fixtures, not a claim of customer representativeness or comprehensive quality coverage

Lengths are measured with `cl100k_base`, a reproducible surrogate, not the target model's tokenizer. Actual prompt/completion token counts come from the inference transport. The fixture SHA-256 covers the execution metadata and full original trace parts. The report includes expected labels and operational source text so reviewers can audit them without reading megabytes of padding

## Build and compare

Build the same harness on the baseline revision and the candidate revision with identical compiler/profile settings. Keep both binaries, record their hashes and do not change fixture generation, model, settings or hardware between the paired runs

```sh
cd src/worker
CARGO_NET_GIT_FETCH_WITH_CLI=true CARGO_BUILD_JOBS=4 \
  CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  cargo build --locked --release -p litellm-lens --example investigation_bench
cp target/release/examples/investigation_bench /path/to/candidate-bench

/usr/bin/time -v -o /path/to/baseline.resources \
  /path/to/baseline-bench --mode record --model claude-haiku-5-5 \
  --sessions 9 --tokens 50000 --concurrency 2 --budget 2.5 --calls 100 \
  --seconds 900 --tape /path/to/tape --output /path/to/baseline.json

/usr/bin/time -v -o /path/to/candidate.resources \
  /path/to/candidate-bench --mode live --model claude-haiku-5-5 \
  --sessions 9 --tokens 50000 --concurrency 2 --budget 2.5 --calls 100 \
  --seconds 900 --tape /path/to/tape --output /path/to/candidate.json
```

For zero-spend paired trials, run each binary with `--mode replay` against the baseline tape. Requests are keyed by the complete serialized model request hash, and an exact request comparison is mandatory. Replay fails closed on a cache miss. It sleeps for the recorded inference duration, so report replay separately from independent live inference. Do not present replay latency as a new live inference result or its recorded cost as newly paid spend. Repeat in alternating order and report individual trials as well as the median/range

Use `--tokens 50000`, `200000` and `500000` for a size sweep. `--sessions` is bounded to 27. `--io-ms` optionally adds the same explicitly disclosed page latency to both runs; it is an emulation, not measured production storage latency. Settings default to concurrency 2, at most 100 model attempts, 900 seconds and a $1 catalog-estimated cap. Paid modes must be chosen explicitly. Before dispatch, the runner reserves the worst-case prepared request estimate, including output headroom, under a shared lock. There are no automatic paid benchmark retries

`--mode evidence` runs a zero-inference subsystem workload using production evidence APIs. It fingerprints each full trace and searches for the late `USER:` marker, then validates four exact quotes per session through `agent::Output`. It reports Reading and Checking page counts, times and a deterministic response hash. It does not measure the model's Reading or confidence-evaluation time and its citation checks are not independent quality judgments. Use it for the larger size sweep without model spending, and compare with and without a disclosed simulated page delay

The investigation context includes a fixed tool-use example instructing every model to search for `USER:` before assessing a session. This layout hint contains no expected labels. Freeze it across baseline and candidate. Results without this hint and results with it are different configurations and must not be compared as optimization legs

## On-prem use

Run the binary inside the approved environment. Set `BENCH_PROVIDER=openai_compatible`, `BENCH_API_BASE`, `BENCH_API_KEY` and `--model` to the approved deployment. Custom deployments also need `BENCH_INPUT_RATE` and `BENCH_OUTPUT_RATE` in dollars per token when the model has no catalog pricing. Rates of zero mean no inference billing is recorded, not that GPU compute is free. Never copy customer traces, credentials or result tapes off-prem

Live mode otherwise uses `ANTHROPIC_API_KEY` or `BENCH_API_KEY`. Credentials remain in environment variables and are never included in tapes/reports. Tapes contain complete synthetic prompts and model outputs and should be treated as sensitive if this harness is adapted to approved sanitized data

## Interpretation

The report records pipeline wall time, stage wall intervals, per-call purpose/duration/tokens/catalog cost, content-page calls, completed reviews, coverage, raw findings, errors and basic quality scores. Stage wall intervals include overlapping work and should not be summed with per-call duration. `/usr/bin/time -v` records process CPU and peak RSS including fixture setup and tokenization, which pipeline wall time excludes. It does not measure the remote inference service or aggregate concurrent child-process RSS

Quality scoring matches a cause only when its check ID and an exact supporting quote from the expected operational span agree. It counts duplicate/extra findings as false positives, reports missing material causes and checks every quote against its cited original execution/span. This mechanical score does not establish semantic evidence sufficiency or incident-level recall. Review the actual findings against the expected source text, including minor findings, counterexamples and omitted occurrences, before claiming quality is maintained. Empty findings are not an improvement. Incomplete/error runs are not successful quality samples

Review expected labels before running and freeze them. Record whether review was by a human/domain expert or an independent model. AI review is not customer approval. Customer model/hardware/version, inference throughput and KV cache settings, prompts/checks, sampling and concurrency must be recorded before extrapolating these measurements to any reported multi-hour investigation
