use std::collections::BTreeSet;

use lens_contract::eval::{
    CaseError, CaseResult, CreateEvalRun, DEFAULT_TIMEOUT_PER_TRIAL_MS, EvalRun, Gate,
    InvalidCaseResult, RunStatus, Scorer, TraceAttribute, TraceRef, scorer_names,
};
use rstest::rstest;
use serde_json::{Value, json};

const SCHEMA: &str = include_str!("../../../../../schema/lens.v1.json");
const CREATE_RUN: &str = include_str!("../fixtures/lens_eval/create_run.json");
const RESULT_TRACE: &str = include_str!("../fixtures/lens_eval/case_result_trace.json");
const RESULT_ERROR: &str = include_str!("../fixtures/lens_eval/case_result_error.json");
const RUN_DONE: &str = include_str!("../fixtures/lens_eval/eval_run_done.json");
const RUN_NO_BASELINE: &str = include_str!("../fixtures/lens_eval/eval_run_no_baseline.json");

fn schema_properties(definition: &str) -> BTreeSet<String> {
    let schema: Value = serde_json::from_str(SCHEMA).expect("schema is JSON");
    schema["$defs"][definition]["properties"]
        .as_object()
        .expect("definition has properties")
        .keys()
        .cloned()
        .collect()
}

fn serialized_keys(value: &impl serde::Serialize) -> BTreeSet<String> {
    serde_json::to_value(value)
        .expect("serializes")
        .as_object()
        .expect("is an object")
        .keys()
        .cloned()
        .collect()
}

#[rstest]
#[case::create_run(CREATE_RUN)]
#[case::eval_run_done(RUN_DONE)]
#[case::eval_run_no_baseline(RUN_NO_BASELINE)]
#[case::case_result_trace(RESULT_TRACE)]
#[case::case_result_error(RESULT_ERROR)]
fn golden_fixtures_round_trip(#[case] fixture: &str) {
    let original: Value = serde_json::from_str(fixture).expect("fixture is JSON");
    let reparsed: Value = match original.get("status") {
        Some(_) => serde_json::to_value(
            serde_json::from_value::<EvalRun>(original.clone()).expect("parses"),
        ),
        None if original.get("eval").is_some() => serde_json::to_value(
            serde_json::from_value::<CreateEvalRun>(original.clone()).expect("parses"),
        ),
        None => serde_json::to_value(
            serde_json::from_value::<CaseResult>(original.clone()).expect("parses"),
        ),
    }
    .expect("serializes");

    let original_keys = original
        .as_object()
        .expect("object")
        .keys()
        .collect::<BTreeSet<_>>();
    let kept = reparsed
        .as_object()
        .expect("object")
        .iter()
        .filter(|(key, _)| original_keys.contains(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    assert_eq!(Value::Object(kept), original);
}

#[rstest]
#[case::create_eval_run("CreateEvalRun", serialized_keys(&serde_json::from_str::<CreateEvalRun>(CREATE_RUN).unwrap()))]
#[case::eval_run("EvalRun", serialized_keys(&serde_json::from_str::<EvalRun>(RUN_DONE).unwrap()))]
#[case::case_result("CaseResult", serialized_keys(&serde_json::from_str::<CaseResult>(RESULT_TRACE).unwrap()))]
#[case::gate("Gate", serialized_keys(&Gate::default()))]
fn rust_types_match_the_checked_in_schema(
    #[case] definition: &str,
    #[case] rust_keys: BTreeSet<String>,
) {
    assert_eq!(rust_keys, schema_properties(definition));
}

#[test]
fn checked_in_schema_is_generated_from_the_rust_types() {
    let checked_in: Value = serde_json::from_str(SCHEMA).unwrap();
    assert_eq!(
        lens_contract::schema::eval_contract(),
        checked_in,
        "schema/lens.v1.json drifted, run npm run generate:eval-contract"
    );
}

#[test]
fn timeout_per_trial_defaults_to_twenty_minutes() {
    let run: CreateEvalRun = serde_json::from_str(CREATE_RUN).expect("parses");
    assert_eq!(run.timeout_per_trial_ms, DEFAULT_TIMEOUT_PER_TRIAL_MS);
    assert_eq!(DEFAULT_TIMEOUT_PER_TRIAL_MS, 20 * 60 * 1000);

    let schema: Value = serde_json::from_str(SCHEMA).expect("schema is JSON");
    assert_eq!(
        schema["$defs"]["CreateEvalRun"]["properties"]["timeout_per_trial_ms"]["default"],
        json!(DEFAULT_TIMEOUT_PER_TRIAL_MS)
    );
}

#[test]
fn timeout_per_trial_is_kept_when_sent() {
    let mut body: Value = serde_json::from_str(CREATE_RUN).expect("parses");
    body["timeout_per_trial_ms"] = json!(90_000);
    let run: CreateEvalRun = serde_json::from_value(body).expect("parses");
    assert_eq!(run.timeout_per_trial_ms, 90_000);
}

#[rstest]
#[case::task_completed(json!({"kind": "task_completed"}), "task_completed")]
#[case::called_before(json!({"kind": "called_before", "first": "run_tests", "then": "open_pr"}), "called_before")]
#[case::judge_default_model(json!({"kind": "judge", "prompt": "Did it finish?"}), "judge")]
fn scorers_parse_by_kind(#[case] body: Value, #[case] kind: &str) {
    let scorer: Scorer = serde_json::from_value(body.clone()).expect("parses");
    assert_eq!(scorer.kind(), kind);
    assert_eq!(
        serde_json::to_value(&scorer).expect("serializes")["kind"],
        json!(kind)
    );
}

#[rstest]
#[case::unknown_kind(json!({"kind": "vibes"}))]
#[case::extra_field(json!({"kind": "task_completed", "extra": 1}))]
#[case::called_before_missing_then(json!({"kind": "called_before", "first": "a"}))]
fn invalid_scorers_are_rejected(#[case] body: Value) {
    assert!(serde_json::from_value::<Scorer>(body).is_err());
}

#[test]
fn unknown_fields_on_create_run_are_rejected() {
    let mut body: Value = serde_json::from_str(CREATE_RUN).expect("parses");
    body["surprise"] = json!(true);
    assert!(serde_json::from_value::<CreateEvalRun>(body).is_err());
}

#[test]
fn repeated_scorer_kinds_get_numbered_names() {
    let scorers: Vec<Scorer> = serde_json::from_value(json!([
        {"kind": "judge", "prompt": "a"},
        {"kind": "task_completed"},
        {"kind": "judge", "prompt": "b"},
    ]))
    .expect("parses");
    assert_eq!(
        scorer_names(&scorers),
        ["judge_1", "task_completed", "judge_2"]
    );
}

#[test]
fn default_gate_blocks_regressions_and_critical_only() {
    let gate = Gate::default();
    assert_eq!((gate.regressions, gate.critical), (Some(0), Some(0)));
    assert_eq!((gate.pass_rate, gate.cost_per_case), (None, None));
    assert!(gate.min.is_empty());
}

#[test]
fn trace_ref_defaults_to_session_id() {
    let trace: TraceRef = serde_json::from_value(json!({"value": "run_1"})).expect("parses");
    assert_eq!(trace.attribute, TraceAttribute::SessionId);
}

fn trace() -> Option<TraceRef> {
    Some(TraceRef {
        attribute: TraceAttribute::TraceId,
        value: "abc".into(),
    })
}

fn error(message: &str) -> Option<CaseError> {
    Some(CaseError {
        r#type: "RuntimeError".into(),
        message: message.into(),
    })
}

#[rstest]
#[case::trace_only(CaseResult { trace: trace(), ..CaseResult::default() }, Ok(()))]
#[case::error_only(CaseResult { error: error("boom"), ..CaseResult::default() }, Ok(()))]
#[case::neither(
    CaseResult::default(),
    Err(InvalidCaseResult::NeedsExactlyOneOfTraceOrError)
)]
#[case::both(
    CaseResult { trace: trace(), error: error("boom"), ..CaseResult::default() },
    Err(InvalidCaseResult::NeedsExactlyOneOfTraceOrError)
)]
#[case::blank_trace(
    CaseResult { trace: Some(TraceRef { attribute: TraceAttribute::SessionId, value: " ".into() }), ..CaseResult::default() },
    Err(InvalidCaseResult::EmptyTraceValue)
)]
#[case::message_at_limit(CaseResult { error: error(&"x".repeat(2000)), ..CaseResult::default() }, Ok(()))]
#[case::message_over_limit(
    CaseResult { error: error(&"x".repeat(2001)), ..CaseResult::default() },
    Err(InvalidCaseResult::ErrorMessageTooLong)
)]
#[case::negative_cost(
    CaseResult { trace: trace(), cost_usd: Some(-0.01), ..CaseResult::default() },
    Err(InvalidCaseResult::NegativeOrNonFiniteCost)
)]
#[case::infinite_cost(
    CaseResult { trace: trace(), cost_usd: Some(f64::INFINITY), ..CaseResult::default() },
    Err(InvalidCaseResult::NegativeOrNonFiniteCost)
)]
fn case_result_validation(
    #[case] result: CaseResult,
    #[case] expected: Result<(), InvalidCaseResult>,
) {
    assert_eq!(result.validate(), expected);
}

#[test]
fn run_status_uses_snake_case() {
    assert_eq!(
        serde_json::to_value(RunStatus::Scoring).expect("serializes"),
        json!("scoring")
    );
}
