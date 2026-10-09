use lens_contract::{
    activity::{ActivitySelection, Preview},
    investigations::TraceFindingCount,
    worker::LensSettings,
};
use rstest::rstest;
use serde_json::{Value, json};

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
