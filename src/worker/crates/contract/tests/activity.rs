use lens_contract::{
    activity::{ActivitySelection, Preview, is_decisions_mode},
    investigations::TraceFindingCount,
    worker::LensSettings,
};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::decisions("decisions", true)]
#[case::legacy_evaluation("evaluation", true)]
#[case::chat("chat", false)]
fn recognizes_both_decisions_model_modes(#[case] mode: &str, #[case] expected: bool) {
    assert_eq!(is_decisions_mode(mode), expected);
}

#[rstest]
fn public_selection_preserves_every_activity_setting() {
    let settings:LensSettings=serde_json::from_value(json!({
        "source":"both","service":"svc","agent_name":"agent","filters":[{"key":"team","value":"red"}],
        "sample_size":7,"sample_percent":12.5,"team_id":"team","execution_ids":["execution"],
        "name":"investigation","model":"model"
    })).unwrap();
    assert_eq!(
        serde_json::to_value(ActivitySelection::from(&settings)).unwrap(),
        json!({
            "source":"both","service":"svc","agent_name":"agent","filters":[{"key":"team","value":"red"}],
            "sample_size":7,"sample_percent":12.5,"team_id":"team","execution_ids":["execution"]
        })
    );
}

#[rstest]
fn preview_defaults_match_the_existing_selection_contract() {
    let preview: Preview = serde_json::from_value(json!({"selection":{}})).unwrap();
    assert_eq!(
        serde_json::to_value(preview).unwrap(),
        json!({"as_of":null,"offset":0,"lookback_hours":24,"selection":{
            "source":"traces","service":"","agent_name":"","filters":[],"sample_size":null,"sample_percent":100.0,"team_id":"","execution_ids":[]
        }})
    );
}

#[rstest]
#[case::missing(json!({"trace_id":"trace","trace_ref":""}),false)]
#[case::unassessed(json!({"trace_id":"trace","trace_ref":"","finding_count":null}),true)]
#[case::assessed_clean(json!({"trace_id":"trace","trace_ref":"","finding_count":0}),true)]
fn finding_counts_require_a_nullable_count(#[case] value: Value, #[case] valid: bool) {
    let parsed = serde_json::from_value::<TraceFindingCount>(value.clone());
    assert_eq!(parsed.is_ok(), valid);
    if let Ok(parsed) = parsed {
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }
}

#[rstest]
#[case::coding("profile", json!("Coding"))]
#[case::unclassified("profile", json!("Unclassified"))]
#[case::unknown("profile", json!("Not recorded"))]
#[case::all_profiles("profile", json!(null))]
#[case::saved_signal("signal_id", json!("task_not_completed"))]
#[case::all_signals("signal_id", json!(null))]
fn selected_filter_survives_saved_settings_and_activity_conversion(
    #[case] field: &str,
    #[case] value: Value,
) {
    let settings: LensSettings =
        serde_json::from_value(json!({"name":"Review","model":"analysis",field:value})).unwrap();
    assert_eq!(
        serde_json::to_value(&settings).unwrap().get(field),
        if value.is_null() { None } else { Some(&value) }
    );
    let selection = ActivitySelection::from(&settings);
    assert_eq!(
        serde_json::to_value(selection).unwrap().get(field),
        if value.is_null() { None } else { Some(&value) }
    );
    let public = serde_json::to_value(lens_contract::investigations::Public(&settings)).unwrap();
    assert_eq!(
        public.get(field),
        if value.is_null() { None } else { Some(&value) }
    );
}

#[rstest]
#[case::empty(json!(""))]
#[case::number(json!(42))]
fn invalid_filter_does_not_become_an_unfiltered_selection(
    #[values("profile", "signal_id")] field: &str,
    #[case] value: Value,
) {
    assert!(serde_json::from_value::<ActivitySelection>(json!({field:value})).is_err());
    assert!(
        serde_json::from_value::<LensSettings>(
            json!({"name":"Review","model":"analysis",field:value})
        )
        .is_err()
    );
}
