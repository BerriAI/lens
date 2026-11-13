use lens_contract::optimizations::ReplayRequest;
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::text(json!({"messages":[{"role":"user","content":"Question"}],"temperature":0.2}), true)]
#[case::conversation(json!([{"role":"user","content":"Question"}]), true)]
#[case::tool_history_without_definitions(json!([{"role":"tool","content":"Result","tool_call_id":"one"}]), false)]
#[case::credentials(json!({"messages":[{"role":"user","content":"Question"}],"api_key":"example-only"}), false)]
#[case::destination(json!({"messages":[{"role":"user","content":"Question"}],"api_base":"https://example.test"}), false)]
#[case::hosted_tool(json!({"messages":[{"role":"user","content":"Question"}],"tools":[{"type":"web_search"}]}), false)]
#[case::image(json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/image"}}]}]}), false)]
fn capture_accepts_only_supported_replay_payloads(#[case] input: Value, #[case] accepted: bool) {
    assert_eq!(
        ReplayRequest::capture(&input.to_string()).is_some(),
        accepted
    );
}

#[rstest]
fn full_request_preserves_tool_ids_schema_and_sampling() {
    let payload = json!({"model":"original","stream":true,"metadata":{"ignored":"private"},"temperature":0.2,"tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"messages":[
        {"role":"assistant","tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"call-1","content":"The order shipped"},
        {"role":"user","content":"Summarize"}
    ]});
    let captured = ReplayRequest::capture(&payload.to_string()).unwrap();
    assert_eq!(
        captured.messages,
        payload["messages"].as_array().unwrap().clone()
    );
    assert_eq!(captured.parameters["temperature"], 0.2);
    assert_eq!(captured.parameters["tools"], payload["tools"]);
    assert!(!captured.parameters.contains_key("model"));
    assert!(!captured.parameters.contains_key("metadata"));
}
