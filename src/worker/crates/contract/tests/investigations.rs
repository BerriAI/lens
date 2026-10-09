use lens_contract::{investigations::*, worker};
use rstest::{fixture, rstest};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[fixture]
fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/investigations_public.json")).unwrap()
}

fn roundtrip<T: DeserializeOwned + Serialize>(value: &Value) -> Value {
    serde_json::to_value(serde_json::from_value::<T>(value.clone()).unwrap()).unwrap()
}

#[rstest]
#[case::lens("lens",roundtrip::<Lens>)]
#[case::filled_lens("filled_lens",roundtrip::<Lens>)]
#[case::worker("worker",roundtrip::<Worker>)]
#[case::settings("settings",roundtrip::<Public<worker::LensSettings>>)]
#[case::job("job",roundtrip::<Public<worker::Job>>)]
#[case::review("review",roundtrip::<Public<worker::Review>>)]
#[case::finding("finding",roundtrip::<Public<worker::Finding>>)]
#[case::draft("draft",roundtrip::<Public<worker::FindingDraft>>)]
#[case::sample("sample",roundtrip::<Public<worker::Sample>>)]
#[case::activity("activity",roundtrip::<Public<worker::Activity>>)]
#[case::extraction("extraction",roundtrip::<Public<worker::Extraction>>)]
#[case::content("content",roundtrip::<Public<worker::ExecutionContent>>)]
#[case::claim("claim",roundtrip::<Public<worker::Claim>>)]
#[case::progress("progress",roundtrip::<Public<worker::Progress>>)]
#[case::result("result",roundtrip::<Public<worker::Result>>)]
#[case::run_request("run_request",roundtrip::<RunRequest>)]
#[case::worker_created("worker_created",roundtrip::<WorkerCreated>)]
#[case::lens_list("lens_list",roundtrip::<LensList>)]
#[case::review_page("review_page",roundtrip::<ReviewPage>)]
#[case::watch_all("watch_all",roundtrip::<WatchAllResult>)]
#[case::finding_update("finding_update",roundtrip::<FindingUpdate>)]
#[case::reservation("reservation",roundtrip::<BudgetReservation>)]
fn public_records_match_retained_python_defaults(
    recorded: Value,
    #[case] name: &str,
    #[case] encode: fn(&Value) -> Value,
) {
    assert_eq!(encode(&recorded[name]), recorded[name]);
}

#[rstest]
fn absent_lens_fields_receive_existing_defaults(recorded: Value) {
    let minimal = json!({"id":"lens","scope":{"team_id":"alpha"},"settings":{"name":"Research","model":"analysis","checks":[{"id":"retries","instruction":"Find unrecovered retries"}]},"created_at":"2026-01-15T00:00:00Z","next_run_at":"2026-01-15T00:00:00Z","budget_month":"2026-01"});
    assert_eq!(roundtrip::<Lens>(&minimal), recorded["lens"]);
}

#[rstest]
fn public_view_materializes_fields_without_changing_worker_protocol(recorded: Value) {
    let settings: worker::LensSettings =
        serde_json::from_value(recorded["settings"].clone()).unwrap();
    let wire = serde_json::to_value(&settings).unwrap();
    assert!(wire.get("execution_ids").is_none());
    assert!(wire.get("sample_size").is_none());
    assert_eq!(json!(Public(&settings)), recorded["settings"]);
}

#[rstest]
fn null_option_and_nested_empty_defaults_are_preserved() {
    assert_eq!(
        roundtrip::<RunRequest>(&json!({})),
        json!({"settings":null,"lookback_hours":null,"start":null,"end":null,"agent_name":null})
    );
    assert_eq!(
        json!(Scope::default()),
        json!({"team_id":"","api_key_hash":"","all_teams":false})
    );
    assert_eq!(
        roundtrip::<WatchAllResult>(&json!({"watching":[]})),
        json!({"watching":[],"skipped":[]})
    );
}

#[rstest]
fn unknown_aggregate_fields_are_rejected(recorded: Value) {
    let extra = Value::Object(
        recorded["lens"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .chain([("unexpected".into(), json!(true))])
            .collect(),
    );
    assert!(serde_json::from_value::<Lens>(extra).is_err());
}

#[rstest]
#[case::missing(None, true)]
#[case::null(Some(json!(null)), true)]
#[case::hexadecimal(Some(json!("abcdef0123456789".repeat(4))), true)]
#[case::short(Some(json!("a".repeat(63))), false)]
#[case::long(Some(json!("a".repeat(65))), false)]
#[case::uppercase(Some(json!("A".repeat(64))), false)]
#[case::outside_hex(Some(json!("g".repeat(64))), false)]
#[case::below_digit(Some(json!("/".repeat(64))), false)]
#[case::above_digit(Some(json!(":".repeat(64))), false)]
#[case::before_lowercase(Some(json!("`".repeat(64))), false)]
#[case::unicode(Some(json!("é".repeat(32))), false)]
#[case::number(Some(json!(123)), false)]
fn worker_analysis_key_matches_retained_validation(
    recorded: Value,
    #[case] key: Option<Value>,
    #[case] valid: bool,
) {
    let record = Value::Object(
        recorded["worker"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(field, _)| field.as_str() != "analysis_key_id")
            .map(|(field, value)| (field.clone(), value.clone()))
            .chain(key.clone().map(|key| ("analysis_key_id".into(), key)))
            .collect(),
    );
    let result = serde_json::from_value::<Worker>(record);
    assert_eq!(result.is_ok(), valid);
    if valid {
        assert_eq!(
            json!(result.unwrap().analysis_key_id),
            key.unwrap_or(Value::Null)
        );
    }
}
