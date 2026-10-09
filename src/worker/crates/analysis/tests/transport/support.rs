use lens_analysis::{AnalysisModels, Catalog, Deployment, Provider, Secret, TransportLimits};
use lens_contract::worker::ModelRequest;
use lens_inference::{ModelCapacity, OutputLimits};
use litellm_model_catalog::Provenance;
use rstest::fixture;
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[fixture]
pub fn catalog() -> Arc<Catalog> {
    catalog_with(
        json!({"mode":"chat","litellm_provider":"openai","input_cost_per_token":0.01,"output_cost_per_token":0.02,"max_input_tokens":10000,"max_output_tokens":400,"supports_prompt_caching":true,"supports_prompt_cache_breakpoint":true,"cache_read_input_token_cost":0.001,"cache_creation_input_token_cost":0.03,"cache_creation_input_token_cost_above_1hr":0.04}),
    )
}

pub fn catalog_with(fields: Value) -> Arc<Catalog> {
    Arc::new(
        Catalog::parse(
            serde_json::to_vec(&json!({"fixture":fields}))
                .unwrap()
                .as_slice(),
            Provenance::default(),
        )
        .unwrap(),
    )
}

#[fixture]
pub fn request() -> ModelRequest {
    serde_json::from_value(json!({"prompt":"unused","purpose":"extract","messages":[{"role":"system","content":"Return JSON"},{"role":"user","content":"Evidence A"},{"role":"assistant","content":"{}"},{"role":"user","content":"Evidence B"}]})).unwrap()
}

pub fn deployment(server: &MockServer, provider: Provider) -> Deployment {
    Deployment {
        name: "analysis".into(),
        model: "fixture".into(),
        provider,
        api_base: Some(server.uri().parse().unwrap()),
        api_key: Secret::new("fixture-key"),
        input_cost_per_token: None,
        output_cost_per_token: None,
        capacity: ModelCapacity::default(),
        output_limits: OutputLimits::default(),
    }
}

pub fn client(catalog: Arc<Catalog>, deployment: Deployment) -> AnalysisModels {
    AnalysisModels::new(catalog, vec![deployment], TransportLimits::default()).unwrap()
}

pub fn completion() -> Value {
    json!({"model":"reported","choices":[{"message":{"content":"{\"ok\":true}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":30,"cache_creation_tokens":10}}})
}

pub fn anthropic_completion() -> Value {
    json!({"model":"reported","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"{\"ok\":"},{"type":"redacted_thinking","data":"hidden"},{"type":"text","text":"true}"}],"stop_reason":"end_turn","usage":{"input_tokens":60,"output_tokens":20,"cache_read_input_tokens":30,"cache_creation_input_tokens":10,"cache_creation":{"ephemeral_5m_input_tokens":7,"ephemeral_1h_input_tokens":3}}})
}

pub async fn respond(server: &MockServer, endpoint: &str, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

pub fn drain_request(stream: &mut std::net::TcpStream) {
    use std::io::Read;
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        headers.push(byte[0]);
    }
    let headers = String::from_utf8(headers).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    stream.read_exact(&mut vec![0; length]).unwrap();
}
