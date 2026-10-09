use lens_contract::{
    InvalidAgentIo,
    agent_io::{AgentIo, JsonPointer},
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

#[fixture]
fn contract() -> Value {
    json!({
        "version": 1, "connection": "agent",
        "submit": {
            "method": "POST", "path": "/api/runs", "accepted_status": 201,
            "json": {"prompt": "", "client_id": "", "messages": [], "nested": {"value": ""}}
        },
        "input": [
            {"source": "case.input", "target": "/prompt"},
            {"source": "trial.request_id", "target": "/client_id"}
        ],
        "completion": {
            "kind": "poll", "id_pointer": "/id", "path_template": "/api/runs/{id}",
            "status_pointer": "/status", "success": ["completed", "idle"],
            "failure": ["failed", "cancelled", "interrupted"], "error_pointer": "/error",
            "interval_ms": 1000, "timeout_ms": 600000,
            "require_item": {"array_pointer": "/messages", "matches": {"role": "assistant", "status": "completed"}}
        },
        "output": {"pointer": "/summary", "require_nonempty": true},
        "trace": {"source": "accepted", "attribute": "session.id", "pointer": "/id"}
    })
}

#[rstest]
#[case::poll(None)]
#[case::immediate(Some(json!({"kind": "immediate"})))]
fn accepted_contracts_round_trip(mut contract: Value, #[case] completion: Option<Value>) {
    if let Some(completion) = completion {
        contract["completion"] = completion;
    }
    let parsed: AgentIo = serde_json::from_value(contract.clone()).unwrap();
    assert_eq!(parsed.validate(), Ok(()));
    assert_eq!(serde_json::to_value(parsed).unwrap(), contract);
}

#[rstest]
#[case::invalid_source("/input/0/source", json!("env.SECRET"))]
#[case::invalid_method("/submit/method", json!("GET"))]
#[case::body_must_be_object("/submit/json", json!([]))]
#[case::invalid_pointer_prefix("/input/0/target", json!("$.prompt"))]
#[case::invalid_pointer_escape("/output/pointer", json!("/answer~2"))]
#[case::invalid_pointer_suffix("/completion/status_pointer", json!("/status~"))]
#[case::unknown_trace_source("/trace/source", json!("env"))]
#[case::complex_match_value("/completion/require_item/matches/role", json!({"secret":"value"}))]
#[case::immediate_extra_field("/completion", json!({"kind":"immediate","path":"/other"}))]
fn rejects_unsupported_shapes(
    mut contract: Value,
    #[case] pointer: &str,
    #[case] replacement: Value,
) {
    *contract.pointer_mut(pointer).unwrap() = replacement;
    assert!(serde_json::from_value::<AgentIo>(contract).is_err());
}

#[rstest]
#[case::version("/version", json!(2), InvalidAgentIo::Version)]
#[case::connection("/connection", json!("env.SECRET"), InvalidAgentIo::Connection)]
#[case::empty_connection("/connection", json!(""), InvalidAgentIo::Connection)]
#[case::absolute_submit("/submit/path", json!("https://other.example/run"), InvalidAgentIo::RequestPath)]
#[case::network_path("/submit/path", json!("//other.example/run"), InvalidAgentIo::RequestPath)]
#[case::backslash_path("/submit/path", json!("/\\other.example/run"), InvalidAgentIo::RequestPath)]
#[case::query_path("/submit/path", json!("/run?secret=value"), InvalidAgentIo::RequestPath)]
#[case::fragment_path("/submit/path", json!("/run#fragment"), InvalidAgentIo::RequestPath)]
#[case::control_path("/submit/path", json!("/run\n"), InvalidAgentIo::RequestPath)]
#[case::template_submit("/submit/path", json!("/run/{id}"), InvalidAgentIo::RequestPath)]
#[case::rejected_status("/submit/accepted_status", json!(302), InvalidAgentIo::AcceptedStatus)]
#[case::no_input("/input", json!([]), InvalidAgentIo::MissingInput)]
#[case::input_not_mapped("/input/0/source", json!("trial.request_id"), InvalidAgentIo::MissingInput)]
#[case::missing_target("/input/0/target", json!("/missing"), InvalidAgentIo::InputTarget)]
#[case::root_target("/input/0/target", json!(""), InvalidAgentIo::OverlappingInputTargets)]
#[case::object_target("/input/0/target", json!("/nested"), InvalidAgentIo::InputTarget)]
#[case::nonempty_array_target("/submit/json/prompt", json!(["fixed"]), InvalidAgentIo::InputTarget)]
#[case::duplicate_target("/input/1/target", json!("/prompt"), InvalidAgentIo::OverlappingInputTargets)]
#[case::ancestor_targets("/input", json!([{"source":"case.input","target":"/nested"},{"source":"trial.request_id","target":"/nested/value"}]), InvalidAgentIo::OverlappingInputTargets)]
#[case::poll_without_id("/completion/path_template", json!("/api/runs"), InvalidAgentIo::PollPath)]
#[case::multiple_ids("/completion/path_template", json!("/api/{id}/{id}"), InvalidAgentIo::PollPath)]
#[case::unknown_placeholder("/completion/path_template", json!("/api/{id}/{secret}"), InvalidAgentIo::PollPath)]
#[case::external_poll("/completion/path_template", json!("//other.example/{id}"), InvalidAgentIo::PollPath)]
#[case::overlapping_states("/completion/failure", json!(["idle"]), InvalidAgentIo::PollStates)]
#[case::duplicate_states("/completion/success", json!(["idle", "idle"]), InvalidAgentIo::PollStates)]
#[case::empty_states("/completion/failure", json!([]), InvalidAgentIo::PollStates)]
#[case::blank_state("/completion/success", json!([" "]), InvalidAgentIo::PollStates)]
#[case::fast_poll("/completion/interval_ms", json!(249), InvalidAgentIo::PollTiming)]
#[case::slow_poll("/completion/interval_ms", json!(60001), InvalidAgentIo::PollTiming)]
#[case::short_timeout("/completion/timeout_ms", json!(999), InvalidAgentIo::PollTiming)]
#[case::long_timeout("/completion/timeout_ms", json!(3600001), InvalidAgentIo::PollTiming)]
#[case::empty_match("/completion/require_item/matches", json!({}), InvalidAgentIo::EmptyMatch)]
fn rejects_invalid_execution_contracts(
    mut contract: Value,
    #[case] pointer: &str,
    #[case] replacement: Value,
    #[case] expected: InvalidAgentIo,
) {
    *contract.pointer_mut(pointer).unwrap() = replacement;
    let parsed: AgentIo = serde_json::from_value(contract).unwrap();
    assert_eq!(parsed.validate(), Err(expected));
}

#[rstest]
fn maps_followups_into_an_existing_empty_array(mut contract: Value) {
    contract["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":"case.followups","target":"/messages"}));
    let parsed: AgentIo = serde_json::from_value(contract).unwrap();
    assert_eq!(parsed.validate(), Ok(()));
    let mut body = Value::Object(parsed.submit.json);
    *parsed.input[2].target.resolve_mut(&mut body).unwrap() = json!(["Follow up"]);
    assert_eq!(body["messages"], json!(["Follow up"]));
    assert_eq!(body["nested"], json!({"value":""}));
}

#[rstest]
#[case::escaped_key("/a~1b/~0key", Some(json!("value")))]
#[case::array_index("/items/0", Some(json!("first")))]
#[case::missing_key("/missing", None)]
#[case::missing_index("/items/1", None)]
#[case::leading_zero_index("/items/00", None)]
fn pointers_resolve_exact_tokens(#[case] pointer: &str, #[case] expected: Option<Value>) {
    let pointer = JsonPointer::try_from(pointer.to_owned()).unwrap();
    let body = json!({"a/b":{"~key":"value"},"items":["first"]});
    assert_eq!(pointer.resolve(&body).cloned(), expected);
}

#[rstest]
fn poll_interval_cannot_exceed_its_deadline(mut contract: Value) {
    contract["completion"]["interval_ms"] = json!(2000);
    contract["completion"]["timeout_ms"] = json!(1000);
    let parsed: AgentIo = serde_json::from_value(contract).unwrap();
    assert_eq!(parsed.validate(), Err(InvalidAgentIo::PollTiming));
}
