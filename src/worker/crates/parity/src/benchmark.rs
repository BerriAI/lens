use std::{collections::BTreeMap, ops::Range};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::BenchmarkError;

#[derive(Debug, Deserialize, Serialize)]
pub struct Distribution {
    pub samples_ms: Vec<f64>,
    pub p10_ms: f64,
    pub median_ms: f64,
    pub p90_ms: f64,
}

impl Distribution {
    pub fn new(mut samples_ms: Vec<f64>) -> Result<Self, BenchmarkError> {
        if samples_ms.is_empty()
            || samples_ms
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(BenchmarkError::Response("nonempty finite latency samples"));
        }
        samples_ms.sort_by(f64::total_cmp);
        let percentile = |percent: usize| samples_ms[(samples_ms.len() - 1) * percent / 100];
        Ok(Self {
            p10_ms: percentile(10),
            median_ms: percentile(50),
            p90_ms: percentile(90),
            samples_ms,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Stage {
    pub traces: usize,
    pub spans: usize,
    pub measurements: BTreeMap<String, Distribution>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub source_sha: String,
    pub artifact: String,
    pub resources: String,
    pub timestamp_ms: i64,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub passed: bool,
    pub checks: Vec<Check>,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub traces: usize,
    pub operation: String,
    pub baseline_p90_ms: f64,
    pub candidate_p90_ms: f64,
    pub allowed_p90_ms: f64,
    pub passed: bool,
}

pub fn compare(baseline: &Report, candidate: &Report) -> Result<Comparison, BenchmarkError> {
    if baseline.schema_version != 1
        || candidate.schema_version != 1
        || baseline.resources != candidate.resources
        || baseline.timestamp_ms != candidate.timestamp_ms
        || baseline.stages.len() != 2
        || candidate.stages.len() != 2
    {
        return Err(BenchmarkError::Response(
            "comparable version, fixture and resources",
        ));
    }
    let mut checks = Vec::new();
    for ((baseline, candidate), count) in baseline
        .stages
        .iter()
        .zip(&candidate.stages)
        .zip([100, 1000])
    {
        if baseline.traces != count
            || candidate.traces != count
            || baseline.spans != count * 10
            || candidate.spans != count * 10
            || baseline
                .measurements
                .keys()
                .ne(candidate.measurements.keys())
            || baseline.measurements.len() != 5
        {
            return Err(BenchmarkError::Response("identical benchmark workload"));
        }
        for (operation, base) in &baseline.measurements {
            let next = &candidate.measurements[operation];
            let before = Distribution::new(base.samples_ms.clone())?;
            let after = Distribution::new(next.samples_ms.clone())?;
            if before.samples_ms.len() < 10 || after.samples_ms.len() != before.samples_ms.len() {
                return Err(BenchmarkError::Response("paired sample counts"));
            }
            let allowed = before.p90_ms * 1.25 + 10.0;
            checks.push(Check {
                traces: count,
                operation: operation.clone(),
                baseline_p90_ms: before.p90_ms,
                candidate_p90_ms: after.p90_ms,
                allowed_p90_ms: allowed,
                passed: after.p90_ms <= allowed,
            });
        }
    }
    Ok(Comparison {
        passed: checks.iter().all(|check| check.passed),
        checks,
    })
}

pub fn trace_id(index: usize) -> String {
    format!("6c656e7362656e6368000000{:08x}", index + 1)
}

pub fn fixture(traces: Range<usize>, timestamp_ms: i64) -> Value {
    let spans: Vec<_> = traces.flat_map(|index| (1..=10).map(move |span| {
        let start = (timestamp_ms + index as i64*100 + span)*1_000_000;
        json!({
            "traceId":trace_id(index),"spanId":format!("{span:016x}"),
            "parentSpanId":if span==1 {String::new()} else {"0000000000000001".into()},
            "name":if span==1 {"benchmark_agent"} else {"benchmark_tool"},
            "startTimeUnixNano":start.to_string(),"endTimeUnixNano":(start+1_000_000).to_string(),
            "status":{"code":1},
            "attributes":[
                {"key":"openinference.span.kind","value":{"stringValue":if span==1 {"AGENT"} else {"TOOL"}}},
                {"key":"gen_ai.agent.name","value":{"stringValue":"benchmark_agent"}},
                {"key":"input.value","value":{"stringValue":format!("benchmark request {index}")}},
                {"key":"output.value","value":{"stringValue":"benchmark response"}}
            ]
        })
    })).collect();
    json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"lens-service-benchmark"}}]},"scopeSpans":[{"scope":{"name":"lens.benchmark"},"spans":spans}]}]})
}
