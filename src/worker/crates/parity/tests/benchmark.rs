use std::collections::BTreeMap;

use lens_parity::benchmark::{Distribution, Report, Stage, compare, fixture, trace_id};
use rstest::{fixture, rstest};

#[fixture]
fn report() -> Report {
    Report {
        schema_version: 1,
        source_sha: "a".repeat(40),
        artifact: "sha256:example".into(),
        resources: "service2cpu2GiB;storage2cpu2GiB;arm64".into(),
        timestamp_ms: 1_000_000,
        stages: [100, 1000]
            .into_iter()
            .map(|traces| Stage {
                traces,
                spans: traces * 10,
                measurements: [
                    "ingest_10_traces",
                    "receipt",
                    "list_all_traces",
                    "trace_details",
                    "span_content",
                ]
                .into_iter()
                .map(|name| (name.to_owned(), Distribution::new(vec![20.0; 11]).unwrap()))
                .collect::<BTreeMap<_, _>>(),
            })
            .collect(),
    }
}

#[rstest]
#[case::ordered(vec![1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0,11.0],(2.0,6.0,10.0))]
#[case::unsorted(vec![10.0,9.0,8.0,7.0,6.0,5.0,4.0,3.0,2.0,1.0,0.0],(1.0,5.0,9.0))]
fn samples_define_all_reported_percentiles(
    #[case] samples: Vec<f64>,
    #[case] expected: (f64, f64, f64),
) {
    let result = Distribution::new(samples).unwrap();
    assert_eq!((result.p10_ms, result.median_ms, result.p90_ms), expected);
}

#[rstest]
#[case::empty(vec![])]
#[case::negative(vec![-1.0])]
#[case::infinity(vec![f64::INFINITY])]
#[case::nan(vec![f64::NAN])]
fn invalid_samples_never_qualify(#[case] samples: Vec<f64>) {
    assert!(Distribution::new(samples).is_err());
}

#[rstest]
#[case::same(20.0, true)]
#[case::budget_edge(35.0, true)]
#[case::over_budget(35.01, false)]
fn comparison_checks_predeclared_budget_from_raw_samples(
    report: Report,
    #[case] latency: f64,
    #[case] passed: bool,
) {
    let mut candidate: Report =
        serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
    candidate.stages[1]
        .measurements
        .get_mut("trace_details")
        .unwrap()
        .samples_ms = vec![latency; 11];
    let result = compare(&report, &candidate).unwrap();
    assert_eq!(result.passed, passed);
    assert_eq!(result.checks.len(), 10);
    let changed = result
        .checks
        .iter()
        .find(|check| check.traces == 1000 && check.operation == "trace_details")
        .unwrap();
    assert_eq!(changed.allowed_p90_ms, 35.0);
    assert_eq!(changed.candidate_p90_ms, latency);
}

#[rstest]
#[case::too_few(9, false)]
#[case::minimum(10, true)]
#[case::above_minimum(11, true)]
fn paired_runs_require_at_least_ten_samples(
    report: Report,
    #[case] count: usize,
    #[case] accepted: bool,
) {
    let mut paired = report;
    for stage in &mut paired.stages {
        for distribution in stage.measurements.values_mut() {
            distribution.samples_ms = vec![20.0; count];
        }
    }
    assert_eq!(compare(&paired, &paired).is_ok(), accepted);
}

#[rstest]
#[case::schema("/schema_version",serde_json::json!(2))]
#[case::resources("/resources",serde_json::json!("different"))]
#[case::fixture("/timestamp_ms",serde_json::json!(1001))]
#[case::stage_count("/stages",serde_json::json!([]))]
#[case::trace_loss("/stages/0/traces",serde_json::json!(99))]
#[case::span_loss("/stages/0/spans",serde_json::json!(999))]
#[case::sample_count("/stages/0/measurements/receipt/samples_ms",serde_json::json!([1.0]))]
#[case::operations("/stages/0/measurements",serde_json::json!({}))]
fn comparison_rejects_incomparable_or_incomplete_runs(
    report: Report,
    #[case] pointer: &str,
    #[case] replacement: serde_json::Value,
) {
    let mut candidate = serde_json::to_value(&report).unwrap();
    *candidate.pointer_mut(pointer).unwrap() = replacement;
    assert!(compare(&report, &serde_json::from_value(candidate).unwrap()).is_err());
}

#[rstest]
fn fixed_fixture_preserves_trace_graph_and_exported_content() {
    let payload = fixture(9..11, 1_000_000);
    let spans = payload["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap();
    assert_eq!(spans.len(), 20);
    assert_eq!(spans[0]["traceId"], trace_id(9));
    assert_eq!(spans[10]["traceId"], trace_id(10));
    assert_eq!(trace_id(9).len(), 32);
    assert_eq!(spans[0]["spanId"], "0000000000000001");
    assert_eq!(spans[0]["parentSpanId"], "");
    assert_eq!(spans[1]["parentSpanId"], "0000000000000001");
    assert_eq!(spans[0]["startTimeUnixNano"], "1000901000000");
    assert_eq!(spans[0]["endTimeUnixNano"], "1000902000000");
    assert_eq!(
        spans[0]["attributes"][2]["value"]["stringValue"],
        "benchmark request 9"
    );
    assert_eq!(
        spans[0]["attributes"][3]["value"]["stringValue"],
        "benchmark response"
    );
}
