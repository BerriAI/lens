use chrono::{DateTime, Utc};
use lens_contract::worker::{ModelRequest, ModelResultFinishReason, StepKind};
use lens_inference::{Prices, Usage, UsageEnvelope, completion_charge, model_result, model_step};
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::extract("extract", "Reviewed a run")]
#[case::cluster("cluster", "Compared observations")]
#[case::investigate("investigate", "Checked a pattern")]
fn model_step_records_serving_model_tokens_and_task(#[case] purpose: &str, #[case] label: &str) {
    let now: DateTime<Utc> = "2026-01-15T12:00:00Z".parse().unwrap();
    let body: ModelRequest =
        serde_json::from_value(json!({"prompt":"review","purpose":purpose})).unwrap();
    let response: UsageEnvelope = serde_json::from_value(json!({"model":"served-model","usage":{"prompt_tokens":1200,"completion_tokens":80,"total_tokens":1280},"choices":[]})).unwrap();
    let step = model_step(&response, &body, "requested-model", 0.25, now).unwrap();
    assert_eq!(step.at, now);
    assert_eq!(step.kind, StepKind::Model);
    assert_eq!(step.label.as_str(), label);
    assert_eq!(step.purpose.as_str(), purpose);
    assert_eq!(step.model.as_str(), "served-model");
    assert_eq!(step.prompt_tokens, 1200);
    assert_eq!(step.completion_tokens, 80);
    assert_eq!(step.cost, 0.25);
}

#[rstest]
#[case::absent(json!({}))]
#[case::null(json!({"model":null,"usage":null}))]
#[case::empty(json!({"model":"","usage":{}}))]
#[case::null_counts(json!({"usage":{"prompt_tokens":null,"completion_tokens":null}}))]
fn partial_provider_usage_still_allows_settlement(#[case] payload: serde_json::Value) {
    let response: UsageEnvelope = serde_json::from_value(payload).unwrap();
    let body: ModelRequest =
        serde_json::from_value(json!({"prompt":"review","purpose":"cluster"})).unwrap();
    let step = model_step(&response, &body, "analysis", 0.0, Utc::now()).unwrap();
    assert_eq!(step.model.as_str(), "analysis");
    assert_eq!(step.prompt_tokens, 0);
    assert_eq!(step.completion_tokens, 0);
    assert_eq!(step.cost, 0.0);
}

#[rstest]
#[case::cost_available(0.4, 0.4)]
#[case::free_or_unmapped(0.0, 0.9)]
#[case::invalid_negative(-0.4,0.9)]
fn noncustom_billing_uses_catalog_cost_or_reservation_estimate(
    #[case] actual: f64,
    #[case] expected: f64,
) {
    assert_eq!(
        completion_charge(None, Usage::default(), actual, 0.9),
        expected
    );
}

#[rstest]
#[case::reported(Some(20), Some(10), 0.04)]
#[case::input_only(Some(20), None, 0.02)]
#[case::output_only(None, Some(10), 0.02)]
#[case::missing(None, None, 0.0)]
fn custom_billing_uses_reported_tokens_even_when_the_charge_is_zero(
    #[case] prompt: Option<i64>,
    #[case] completion: Option<i64>,
    #[case] expected: f64,
) {
    let prices = Prices {
        input_cost_per_token: 0.001,
        output_cost_per_token: 0.002,
        ..Prices::default()
    };
    let usage = Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
    };
    assert_eq!(completion_charge(Some(&prices), usage, 3.0, 4.0), expected);
}

#[rstest]
#[case::length(Some("length"), Some(ModelResultFinishReason::Length))]
#[case::filtered(Some("content_filter"), Some(ModelResultFinishReason::ContentFilter))]
#[case::stop(Some("stop"), None)]
#[case::missing(None, None)]
#[case::other(Some("tool_calls"), None)]
fn worker_result_retains_only_actionable_finish_reasons(
    #[case] reason: Option<&str>,
    #[case] expected: Option<ModelResultFinishReason>,
) {
    let result = model_result(Some("{}"), 0.25, reason);
    assert_eq!(result.content, "{}");
    assert_eq!(result.cost, 0.25);
    assert!(!result.context_exceeded);
    assert_eq!(result.finish_reason, expected);
}

#[rstest]
fn missing_response_content_is_empty() {
    assert_eq!(model_result(None, 0.0, None).content, "");
}
