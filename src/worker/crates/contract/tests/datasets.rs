use lens_contract::datasets::{BuildRequest, BuildSource, DatasetCase, DatasetRole, SkipReason};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
fn case_defaults_are_materialized_in_the_legacy_field_order() {
    let case: DatasetCase = serde_json::from_value(json!({
        "id": "case", "messages": [{"role":"user", "content":"hello"}], "source": {}
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_string(&case).unwrap(),
        r#"{"id":"case","messages":[{"role":"user","content":"hello","name":"","tool_calls":[]}],"reply":"","tool_calls":[],"expected":"","included":true,"source":{"trace_id":"","trace_ref":"","span_id":"","finding_id":"","lens_id":""},"agent_version":""}"#
    );
}

#[rstest]
#[case::system("system", DatasetRole::System)]
#[case::user("user", DatasetRole::User)]
#[case::assistant("assistant", DatasetRole::Assistant)]
#[case::tool("tool", DatasetRole::Tool)]
fn message_roles_round_trip(#[case] wire: &str, #[case] role: DatasetRole) {
    assert_eq!(
        serde_json::from_value::<DatasetRole>(json!(wire)).unwrap(),
        role
    );
    assert_eq!(serde_json::to_value(role).unwrap(), json!(wire));
}

#[rstest]
#[case::duplicate("duplicate", SkipReason::Duplicate)]
#[case::no_content("no_content", SkipReason::NoContent)]
#[case::too_large("too_large", SkipReason::TooLarge)]
#[case::over_limit("over_limit", SkipReason::OverLimit)]
#[case::invalid("invalid", SkipReason::Invalid)]
fn skip_reasons_round_trip(#[case] wire: &str, #[case] reason: SkipReason) {
    assert_eq!(
        serde_json::from_value::<SkipReason>(json!(wire)).unwrap(),
        reason
    );
    assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
}

#[rstest]
#[case::trace(json!({"kind":"trace","trace_id":"t"}), json!({"kind":"trace","trace_id":"t","trace_ref":"","span_id":""}))]
#[case::finding(json!({"kind":"finding","lens_id":"l","finding_ids":["f"]}), json!({"kind":"finding","lens_id":"l","finding_ids":["f"]}))]
#[case::text(json!({"kind":"text","text":"question"}), json!({"kind":"text","text":"question"}))]
fn build_sources_preserve_tags_and_defaults(#[case] source: Value, #[case] expected: Value) {
    let request: BuildRequest = serde_json::from_value(json!({"sources":[source]})).unwrap();
    assert_eq!(request.dataset_id, "");
    assert_eq!(serde_json::to_value(&request.sources[0]).unwrap(), expected);
}

#[rstest]
#[case::missing_tag(json!({"text":"q"}))]
#[case::unknown_tag(json!({"kind":"file","text":"q"}))]
#[case::unknown_field(json!({"kind":"text","text":"q","path":"x"}))]
#[case::null_text(json!({"kind":"text","text":null}))]
fn invalid_source_shapes_are_rejected(#[case] source: Value) {
    assert!(serde_json::from_value::<BuildSource>(source).is_err());
}

#[rstest]
#[case::case_extra(json!({"id":"c","messages":[],"source":{},"extra":1}))]
#[case::message_extra(json!({"id":"c","messages":[{"role":"user","content":"q","extra":1}],"source":{}}))]
#[case::source_extra(json!({"id":"c","messages":[],"source":{"extra":1}}))]
#[case::tool_extra(json!({"id":"c","messages":[],"source":{},"tool_calls":[{"name":"search","arguments":"{}","extra":1}]}))]
#[case::invalid_role(json!({"id":"c","messages":[{"role":"function","content":"q"}],"source":{}}))]
#[case::missing_source(json!({"id":"c","messages":[]}))]
#[case::null_default(json!({"id":"c","messages":[],"source":{},"reply":null}))]
fn records_reject_unknown_or_invalid_fields(#[case] case: Value) {
    assert!(serde_json::from_value::<DatasetCase>(case).is_err());
}
