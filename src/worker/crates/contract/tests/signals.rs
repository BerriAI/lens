use lens_contract::signals::{
    NoulAnswer, Signal, SignalAttempt, SignalConfig, SignalData, SignalEvidence, SignalFlag,
    StoredTraceSignal,
};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
fn defaults_match_recorded_python_configuration() {
    let recorded: Value =
        serde_json::from_str(include_str!("fixtures/signals_default.json")).unwrap();
    assert_eq!(
        serde_json::to_value(SignalConfig::default()).unwrap(),
        recorded["response"]["body"]
    );
    assert_eq!(
        serde_json::from_value::<SignalConfig>(json!({})).unwrap(),
        SignalConfig::default()
    );
}

#[rstest]
#[case::empty_id("id", json!(""))]
#[case::uppercase("id", json!("A"))]
#[case::dash("id", json!("a-b"))]
#[case::digit_start("id", json!("1a"))]
#[case::unicode_id("id", json!("雪"))]
#[case::long_id("id", json!("a".repeat(65)))]
#[case::empty_name("name", json!(""))]
#[case::long_name("name", json!("雪".repeat(61)))]
#[case::short_question("question", json!("ab"))]
#[case::long_question("question", json!("雪".repeat(501)))]
fn invalid_signal_fields_are_rejected(#[case] field: &str, #[case] value: Value) {
    let mut signal = json!({"id":"a","name":"Name","question":"Question?"});
    signal[field] = value;
    assert!(serde_json::from_value::<Signal>(signal).is_err());
}

#[rstest]
#[case::minimum("a", "雪", "雪雪雪")]
#[case::maximum(&"a".repeat(64), &"雪".repeat(60), &"雪".repeat(500))]
#[case::underscore("a_1", "Name", "Why?")]
fn signal_bounds_count_unicode_characters(
    #[case] id: &str,
    #[case] name: &str,
    #[case] question: &str,
) {
    let value = json!({"id":id,"name":name,"question":question});
    let signal: Signal = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(signal).unwrap(), value);
}

#[rstest]
#[case::lower(json!(0.05), true)]
#[case::upper(json!(0.95), true)]
#[case::numeric_string(json!("0.7"), true)]
#[case::below(json!(0.049), false)]
#[case::above(json!(0.951), false)]
#[case::nan(json!("NaN"), false)]
#[case::infinite(json!("inf"), false)]
#[case::null(json!(null), false)]
#[case::object(json!({}), false)]
fn threshold_validation_preserves_range_and_coercion(#[case] value: Value, #[case] valid: bool) {
    assert_eq!(
        serde_json::from_value::<SignalConfig>(json!({"threshold":value})).is_ok(),
        valid
    );
}

#[rstest]
#[case::empty(0, true)]
#[case::maximum(20, true)]
#[case::overflow(21, false)]
fn signal_count_is_bounded(#[case] count: usize, #[case] valid: bool) {
    let signals = (0..count)
        .map(|i| json!({"id":format!("a{i}"),"name":"A","question":"Why?"}))
        .collect::<Vec<_>>();
    assert_eq!(
        serde_json::from_value::<SignalConfig>(json!({"signals":signals})).is_ok(),
        valid
    );
}

#[rstest]
fn duplicate_signal_ids_are_rejected() {
    let signal = json!({"id":"a","name":"A","question":"Why?"});
    assert!(serde_json::from_value::<SignalConfig>(json!({"signals":[signal,signal]})).is_err());
}

#[rstest]
#[case::zero(json!(0),Some(0.0))]
#[case::one(json!(1),Some(1.0))]
#[case::coerced(json!("0.4"),Some(0.4))]
#[case::boolean(json!(true),Some(1.0))]
#[case::negative(json!(-0.1),None)]
#[case::large(json!(1.1),None)]
#[case::nan(json!("NaN"),None)]
#[case::null(json!(null),None)]
fn scores_are_validated_for_both_storage_and_decisions(
    #[case] value: Value,
    #[case] expected: Option<f64>,
) {
    let data = serde_json::from_value::<SignalData>(json!({"scores":{"a":value},"ignored":true}));
    assert_eq!(data.ok().map(|data| data.scores["a"]), expected);
    let answer =
        serde_json::from_value::<NoulAnswer>(json!({"type":"noul","noul":value,"ignored":true}));
    assert_eq!(answer.ok().map(|NoulAnswer::Noul { noul }| noul), expected);
}

#[rstest]
#[case::wrong_type(json!({"type":"score","noul":0.5}))]
#[case::missing_type(json!({"noul":0.5}))]
#[case::missing_score(json!({"type":"noul"}))]
fn decisions_require_the_noul_answer_shape(#[case] value: Value) {
    assert!(serde_json::from_value::<NoulAnswer>(value).is_err());
}

#[rstest]
#[case::utc("2026-01-02T03:04:05Z")]
#[case::offset("2026-01-02T04:04:05+01:00")]
#[case::legacy_t("2026-01-02T03:04:05")]
#[case::legacy_space("2026-01-02 03:04:05")]
fn stored_timestamps_normalize_legacy_naive_values(#[case] timestamp: &str) {
    let row:StoredTraceSignal=serde_json::from_value(json!({"trace_id":"t","config_key":"k","span_count":1,"claimed_until":timestamp,"classified_at":timestamp,"data":[]})).unwrap();
    let expected = "2026-01-02T03:04:05Z".parse().unwrap();
    assert_eq!(row.claimed_until, Some(expected));
    assert_eq!(row.classified_at, Some(expected));
    assert_eq!(row.trace_ref, "");
    assert_eq!(row.data, json!([]));
}

#[rstest]
#[case::pending(json!({"status":"pending","model":"m"}))]
#[case::missing_model(json!({"status":"classified"}))]
#[case::unknown_field(json!({"status":"classified","model":"m","extra":true}))]
fn attempts_require_completed_status_and_model(#[case] value: Value) {
    assert!(serde_json::from_value::<SignalAttempt>(value).is_err());
}

#[rstest]
fn previously_stored_signals_remain_readable_without_evidence() {
    let value = json!({"status":"classified","model":"m","scores":{"a":0.9}});
    let attempt: SignalAttempt = serde_json::from_value(value.clone()).unwrap();
    let data: SignalData = serde_json::from_value(value).unwrap();
    let flag: SignalFlag =
        serde_json::from_value(json!({"signal_id":"a","name":"A","score":0.9})).unwrap();
    assert!(attempt.evidence.is_empty());
    assert!(data.evidence.is_empty());
    assert!(flag.evidence.is_none());
    assert_eq!(
        serde_json::to_value(flag).unwrap(),
        json!({"signal_id":"a","name":"A","score":0.9})
    );
}

#[rstest]
fn signal_evidence_survives_the_storage_and_public_contract() {
    let evidence = SignalEvidence {
        span_id: "problem".into(),
        quote: "Please stop repeating this 🗿".into(),
    };
    let value =
        json!({"status":"classified","model":"m","scores":{"a":0.9},"evidence":{"a":evidence}});
    let attempt: SignalAttempt = serde_json::from_value(value).unwrap();
    let data: SignalData = serde_json::from_value(serde_json::to_value(attempt).unwrap()).unwrap();
    let flag = SignalFlag {
        signal_id: "a".into(),
        name: "A".into(),
        score: data.scores["a"],
        evidence: data.evidence.get("a").cloned(),
    };
    assert_eq!(flag.evidence, Some(evidence));
    assert_eq!(
        serde_json::to_value(flag).unwrap()["evidence"]["span_id"],
        "problem"
    );
}
