# Measure the current Lens service

The `service-benchmark` binary exercises the running HTTP service and verifies stored data before reporting latency. Run it against two isolated deployments to compare a baseline and a candidate. It sends synthetic telemetry and makes no model calls

Historical gateway-bundled Lens measurements remain in the [scalability guide](https://docs.litellm.ai/docs/proxy/lens/scalability). They describe their original code and deployment. They are not current standalone Rust capacity results

## Prepare comparable deployments

Build the runner once from this checkout:

```sh
cargo build --manifest-path src/worker/Cargo.toml --locked --release \
  -p lens-parity --bin service-benchmark
```

Use separate empty ClickHouse databases and volumes for the baseline and candidate. Never point this workload at production. The runner writes 1,000 traces and 10,000 spans with fixed IDs, and requires the selected time window to be empty before starting. Reusing those IDs in a populated database would make a new run incomparable

Pin both service images and the ClickHouse image by digest. Keep architecture, CPU limits, memory limits, storage, network, worker settings and database settings identical. Record those limits in `resources` and retain the actual container configuration alongside the reports. Run the services sequentially on the same otherwise idle host. Equal description strings are necessary for comparison, but the runner cannot enforce host isolation or resource limits

Create an administrator credential and a dedicated tracing key through each deployment's supported setup. Store each private configuration outside the repository with mode `0600`. This is the input shape, using placeholders rather than working credentials:

```json
{
  "base_url": "http://127.0.0.1:4318/",
  "admin_token": "<private administrator credential>",
  "tracing_key": "<private tracing credential>",
  "source_sha": "<40-character source commit>",
  "artifact": "sha256:<exact service image or binary hash>",
  "resources": "<architecture; Lens CPU/memory; ClickHouse CPU/memory; storage and image digest>",
  "timestamp_ms": 1791417600000
}
```

Keep the same `timestamp_ms` in both configurations and choose a window valid for both services' retention settings. The fixture spans the next 100 seconds. Include a trailing slash in `base_url`, retain a deployment's API prefix, and never embed credentials in that URL. `source_sha` identifies the base source revision; retain a diff or source snapshot if the tested artifact includes uncommitted changes. The artifact hash identifies the bytes actually tested

For gateway-bundled Lens, set `base_url` to the gateway's public read API and add `ingestion_url` with the separate Lens endpoint. The runner sends OTLP and delivery receipts to that endpoint with the tracing key. When `ingestion_url` is omitted, both flows use `base_url`. Compare the full original gateway and Rust service with standalone Lens, and account for both original application containers in the resource budget

## Run and compare

From the repository root, with the baseline running:

```sh
src/worker/target/release/service-benchmark run /private/path/baseline.json > baseline-report.json
```

Stop the baseline, start the candidate with its own empty database and the same limits, then run:

```sh
src/worker/target/release/service-benchmark run /private/path/candidate.json > candidate-report.json
src/worker/target/release/service-benchmark compare baseline-report.json candidate-report.json > comparison.json
```

The runner first ingests 100 traces, then grows the same database to 1,000 traces. Each trace contains one agent span and nine tool spans. It records 10 initial and 90 subsequent ingestion batches of 10 traces. At each size, it warms receipt, complete paginated trace listing, trace details and root-span content once, then records 11 samples for each operation

Every trace-list sample must contain exactly the expected trace IDs and span counts without duplicates. The representative receipt must acknowledge all 10 span IDs, the trace details must contain every expected span, and root content must match the exported request and response. An HTTP failure or missing content ends the run instead of producing a passing report

Reports contain raw latency samples, p10, median and p90, plus the source, artifact and resource identifiers. They do not contain credentials. For this bounded comparison, the predeclared threshold is `candidate p90 <= baseline p90 × 1.25 + 10 ms` for every operation at both sizes. All 10 checks must pass. The comparator recalculates percentiles from raw samples and rejects different resource descriptions, fixtures, workloads or sample counts

This threshold is a regression check for the stated workload, not a throughput or production capacity promise. The run has one client, a small fixed trace shape and warm read samples. Concurrent ingestion, large payloads, investigation execution, state compaction, recovery and provider latency need separate evidence. Keep unsuccessful runs and explain environmental interference before repeating a comparison

## Runner qualification

The focused behavior suite checks percentile calculations, the comparison threshold, incomplete reports and the fixed trace fixture:

```sh
cargo test --manifest-path src/worker/Cargo.toml --locked -p lens-parity --test benchmark
```

The comparator and fixture mutation qualification caught all 46 viable mutations, with one unviable mutation. The HTTP runner completed a real development-service smoke at both sizes with no missing records. That shared-host smoke establishes that the workload runs; a baseline/candidate result requires the paired procedure above
