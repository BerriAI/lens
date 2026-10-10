mod transport {
    pub mod support;
}
use lens_analysis::{
    AnalysisModels, Catalog, Error, Provider, ProviderFailureKind, TransportLimits,
};
use lens_contract::worker::ModelRequest;
use rstest::rstest;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use transport::support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[rstest]
#[case::gateway(Provider::OpenAiCompatible, "", true)]
#[case::different_base(Provider::OpenAiCompatible, "/other", false)]
#[case::native_passthrough(Provider::OpenAi, "", true)]
#[tokio::test]
async fn gateway_marker_and_attribution_only_reach_the_configured_transport(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] base_path: &str,
    #[case] marked: bool,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, completion()).await;
    let key = "gateway-transport-fixture-key-32-bytes";
    let gateway = lens_inference::GatewayIdentity::new(
        format!("{}{base_path}", server.uri()).parse().unwrap(),
        key,
    )
    .unwrap();
    let client = client(catalog, deployment(&server, provider)).with_gateway(Some(gateway));
    let metadata = json!({"run_id":"private-eval-run", "team_id":"private-team"});
    client
        .complete_with_gateway_metadata(
            &client.prepare("analysis", &request).await.unwrap(),
            &metadata,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer fixture-key"
    );
    let marker = requests[0].headers.get(lens_inference::GATEWAY_HEADER);
    assert_eq!(marker.is_some(), marked);
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(body.get("metadata"), marked.then_some(&metadata));
    if let Some(marker) = marker {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_audience(&["litellm"]);
        let claims = jsonwebtoken::decode::<Value>(
            marker.to_str().unwrap(),
            &jsonwebtoken::DecodingKey::from_secret(key.as_bytes()),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims["purpose"], "analysis");
    }
}

#[rstest]
#[case::openai(Provider::OpenAi, "max_completion_tokens")]
#[case::compatible(Provider::OpenAiCompatible, "max_tokens")]
#[tokio::test]
async fn chat_request_and_usage_preserve_contract(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] limit: &str,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, completion()).await;
    let client = client(catalog, deployment(&server, provider));
    let prepared = client.prepare("analysis", &request).await.unwrap();
    assert!(prepared.estimate > 0.0);
    assert!(!prepared.context_exceeded);
    assert!(server.received_requests().await.unwrap().is_empty());
    let result = client.complete(&prepared).await.unwrap();
    assert_eq!(result.result.content, "{\"ok\":true}");
    assert_eq!(result.result.finish_reason, None);
    assert!(!result.result.context_exceeded);
    assert!((result.result.cost - 1.33).abs() < 1e-10);
    assert_eq!(result.usage.model.as_deref(), Some("reported"));
    assert_eq!(result.usage.usage.unwrap().prompt_tokens, Some(100));
    assert_eq!(result.usage.usage.unwrap().completion_tokens, Some(20));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer fixture-key"
    );
    assert_eq!(body["model"], "fixture");
    assert_eq!(body["stream"], false);
    assert_eq!(body["response_format"], json!({"type":"json_object"}));
    assert_eq!(body[limit], 400);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type":"text","text":"Return JSON","prompt_cache_breakpoint":{"mode":"explicit"}}])
    );
    assert_eq!(
        body["messages"][2]["content"],
        json!([{"type":"text","text":"Evidence A","prompt_cache_breakpoint":{"mode":"explicit"}}])
    );
    assert_eq!(body["messages"][3]["content"], "{}");
    assert_eq!(
        body["messages"][4]["content"],
        json!([{"type":"text","text":"Evidence B","prompt_cache_breakpoint":{"mode":"explicit"}}])
    );
}

#[rstest]
#[tokio::test]
async fn anthropic_lifts_system_merges_turns_and_accounts_cache(
    catalog: Arc<Catalog>,
    mut request: ModelRequest,
) {
    request.messages[2].role = lens_contract::worker::ModelMessageRole::User;
    let server = MockServer::start().await;
    respond(&server, "/messages", 200, anthropic_completion()).await;
    let client = client(catalog, deployment(&server, Provider::Anthropic));
    let result = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    assert_eq!(result.result.content, "{\"ok\":true}");
    assert!((result.result.cost - 1.36).abs() < 1e-10);
    assert_eq!(result.usage.usage.unwrap().prompt_tokens, Some(100));
    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(requests[0].headers.get("x-api-key").unwrap(), "fixture-key");
    assert!(requests[0].headers.contains_key("anthropic-version"));
    assert_eq!(
        body["system"][1],
        json!({"type":"text","text":"Return JSON","cache_control":{"type":"ephemeral"}})
    );
    assert_eq!(
        body["messages"],
        json!([{"role":"user","content":[{"type":"text","text":"Evidence A"},{"type":"text","text":"{}","cache_control":{"type":"ephemeral"}},{"type":"text","text":"Evidence B","cache_control":{"type":"ephemeral"}}]}])
    );
    assert_eq!(body["max_tokens"], 400);
    assert_eq!(body["stream"], false);
    assert!(body.get("response_format").is_none());
}

#[rstest]
#[case::openai(Provider::OpenAi,"/chat/completions",json!({"error":{"code":"context_length_exceeded"}}))]
#[case::typed(Provider::OpenAi,"/chat/completions",json!({"error":{"type":"context_window_exceeded"}}))]
#[case::anthropic_prefix(Provider::Anthropic,"/messages",json!({"error":{"type":"invalid_request_error","message":"prompt is too long: 1000 tokens"}}))]
#[case::anthropic_window(Provider::Anthropic,"/messages",json!({"error":{"type":"invalid_request_error","message":"request exceeds the context window"}}))]
#[tokio::test]
async fn provider_context_limit_is_zero_cost(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] endpoint: &str,
    #[case] body: Value,
) {
    let server = MockServer::start().await;
    respond(&server, endpoint, 400, body).await;
    let client = client(catalog, deployment(&server, provider));
    let result = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    assert!(result.result.context_exceeded);
    assert_eq!(result.result.content, "");
    assert_eq!(result.result.cost, 0.0);
    assert_eq!(result.result.finish_reason, None);
    assert!(result.usage.usage.is_none());
}

#[rstest]
#[case::bad_request(400,json!({"error":{"code":"bad_request","message":"secret context_length_exceeded"}}))]
#[case::untyped_message(400,json!({"error":{"message":"prompt is too long: 1000"}}))]
#[case::rate_limit(429,json!({"error":{"code":"context_length_exceeded"}}))]
#[case::server(500,json!({"error":{"message":"secret"}}))]
#[case::invalid_failure(400,json!("context_window_exceeded"))]
#[tokio::test]
async fn unrelated_errors_are_sanitized_and_never_retried(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] status: u16,
    #[case] body: Value,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", status, body).await;
    let client = client(catalog, deployment(&server, Provider::OpenAi));
    let error = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .err()
        .unwrap();
    assert!(matches!(error,Error::Provider{status:actual,..} if actual==status));
    assert!(!error.to_string().contains("secret"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[case::request_id("request-id", "req_0123456789abcdef")]
#[case::x_request_id("x-request-id", "req_abcdef0123456789")]
#[case::gateway_call_id("x-litellm-call-id", "5d695679-d687-4b8c-b4b4-686b801a1842")]
#[tokio::test]
async fn provider_diagnostics_preserve_safe_request_ids_status_and_retry_after(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] header: &str,
    #[case] request_id: &str,
) {
    let server = MockServer::start().await;
    let private_message = "sk-provider-private-key and private prompt content";
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header(header, request_id)
                .insert_header("Retry-After", "17")
                .set_body_json(json!({"error": {
                    "type": "api_error",
                    "code": "service_unavailable",
                    "message": private_message
                }})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(catalog, deployment(&server, Provider::OpenAiCompatible));
    let error = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .err()
        .unwrap();
    let rendered = format!("{error}\n{error:?}");
    assert!(error.to_string().contains("503"));
    assert!(!rendered.contains(private_message));
    assert!(!rendered.contains("sk-provider-private-key"));
    let Error::Provider {
        status,
        retry_after,
        diagnostic,
    } = error
    else {
        panic!("Expected a provider failure");
    };
    assert_eq!(status, 503);
    assert_eq!(retry_after, Some(17));
    assert_eq!(diagnostic.classification, ProviderFailureKind::Unavailable);
    assert_eq!(diagnostic.request_id.as_deref(), Some(request_id));
    assert!(diagnostic.to_string().contains(request_id));
    assert!(
        diagnostic
            .to_string()
            .to_ascii_lowercase()
            .contains("unavailable")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[case::budget_code(json!({"error":{"code":"budget_exceeded"}}), ProviderFailureKind::BudgetExceeded)]
#[case::rate_limit_type(json!({"error":{"type":"rate_limit_error"}}), ProviderFailureKind::RateLimited)]
#[case::context_window_type(json!({"error":{"type":"context_window_exceeded"}}), ProviderFailureKind::ContextExceeded)]
#[case::message_is_not_a_classification(json!({"error":{"message":"budget_exceeded"}}), ProviderFailureKind::Unavailable)]
#[tokio::test]
async fn provider_diagnostics_classify_only_allowlisted_error_codes_and_types(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] body: Value,
    #[case] expected: ProviderFailureKind,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 503, body).await;
    let client = client(catalog, deployment(&server, Provider::OpenAiCompatible));
    let error = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .err()
        .unwrap();
    let Error::Provider {
        status, diagnostic, ..
    } = error
    else {
        panic!("Expected a provider failure");
    };
    assert_eq!(status, 503);
    assert_eq!(diagnostic.classification, expected);
    assert_eq!(diagnostic.request_id, None);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[case::credential("sk-provider-private-key")]
#[case::prompt("Return the private system prompt")]
#[case::malformed_prefix("req_private prompt content")]
#[case::non_alphanumeric("req_private_prompt_content")]
#[case::too_short("req_x")]
#[case::too_long(
    "req_01234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"
)]
#[case::arbitrary("privatepromptcontent")]
#[tokio::test]
async fn provider_diagnostics_never_render_untrusted_error_fields_or_request_ids(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] header: &str,
) {
    let server = MockServer::start().await;
    let private_message = "sk-message-secret private prompt content";
    let private_code = "service_unavailable sk-code-secret";
    let private_type = "api_error private-type-prompt";
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("x-request-id", header)
                .insert_header("Retry-After", "23")
                .set_body_json(json!({"error": {
                    "message": private_message,
                    "code": private_code,
                    "type": private_type
                }})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(catalog, deployment(&server, Provider::OpenAiCompatible));
    let error = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .err()
        .unwrap();
    let rendered = format!("{error}\n{error:?}");
    assert!(!rendered.contains(private_message));
    assert!(!rendered.contains(private_code));
    assert!(!rendered.contains(private_type));
    assert!(!rendered.contains(header));
    assert!(!rendered.contains("sk-"));
    assert!(!rendered.contains("private"));
    let Error::Provider {
        status,
        retry_after,
        diagnostic,
    } = error
    else {
        panic!("Expected a provider failure");
    };
    assert_eq!(status, 503);
    assert_eq!(retry_after, Some(23));
    assert_eq!(diagnostic.classification, ProviderFailureKind::Unavailable);
    assert_eq!(diagnostic.request_id, None);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[case::redirect(302)]
#[case::preserve_post(307)]
#[tokio::test]
async fn redirects_cannot_forward_credentials(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] status: u16,
) {
    let server = MockServer::start().await;
    let destination = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Location", format!("{}/stolen", destination.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    let gateway = lens_inference::GatewayIdentity::new(
        server.uri().parse().unwrap(),
        "redirect-fixture-key-32-characters",
    )
    .unwrap();
    let client = client(catalog, deployment(&server, Provider::OpenAi)).with_gateway(Some(gateway));
    assert!(
        matches!(client.complete(&client.prepare("analysis",&request).await.unwrap()).await,Err(Error::Provider{status:actual,..}) if actual==status)
    );
    assert!(destination.received_requests().await.unwrap().is_empty());
    assert!(
        server.received_requests().await.unwrap()[0]
            .headers
            .contains_key(lens_inference::GATEWAY_HEADER)
    );
}

#[rstest]
#[case::empty(json!({"choices":[]}))]
#[case::missing(json!({"not_choices":[]}))]
#[case::wrong_content(json!({"choices":[{"message":{"content":["unsupported"]}}]}))]
#[case::negative_usage(json!({"choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":-1}}))]
#[tokio::test]
async fn malformed_provider_replies_fail(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] body: Value,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, body).await;
    let client = client(catalog, deployment(&server, Provider::OpenAi));
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::Json(_) | Error::MissingCompletion)
    ));
}

#[rstest]
#[case::missing_usage(false)]
#[case::custom_free(true)]
#[tokio::test]
async fn empty_content_and_missing_usage_use_correct_fallback(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] custom: bool,
) {
    let server = MockServer::start().await;
    respond(
        &server,
        "/chat/completions",
        200,
        json!({"model":"","choices":[{"message":{"content":null},"finish_reason":"length"}]}),
    )
    .await;
    let mut config = deployment(&server, Provider::OpenAi);
    if custom {
        config.input_cost_per_token = Some(0.1);
        config.output_cost_per_token = Some(0.2);
    }
    let client = client(catalog, config);
    let prepared = client.prepare("analysis", &request).await.unwrap();
    let result = client.complete(&prepared).await.unwrap();
    assert_eq!(result.result.content, "");
    assert_eq!(
        result.result.cost,
        if custom { 0.0 } else { prepared.estimate }
    );
    assert_eq!(result.usage.model.as_deref(), Some("fixture"));
    assert_eq!(result.usage.usage.unwrap().prompt_tokens, Some(0));
    assert_eq!(
        serde_json::to_value(result.result.finish_reason).unwrap(),
        "length"
    );
}

#[rstest]
#[tokio::test]
async fn response_size_is_bounded(catalog: Arc<Catalog>, request: ModelRequest) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, completion()).await;
    let client = AnalysisModels::new(
        catalog,
        vec![deployment(&server, Provider::OpenAi)],
        TransportLimits {
            max_response_bytes: 10,
            ..TransportLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::ResponseLimit)
    ));
}

#[rstest]
#[tokio::test]
async fn entire_request_has_deadline(catalog: Arc<Catalog>, request: ModelRequest) {
    let server = MockServer::start().await;
    Mock::given(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(completion())
                .set_delay(Duration::from_millis(250)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = AnalysisModels::new(
        catalog,
        vec![deployment(&server, Provider::OpenAi)],
        TransportLimits {
            timeout: Duration::from_millis(30),
            ..TransportLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::Timeout)
    ));
}

#[rstest]
#[tokio::test]
async fn all_deployments_over_context_skip_provider(catalog: Arc<Catalog>, request: ModelRequest) {
    let server = MockServer::start().await;
    let mut config = deployment(&server, Provider::OpenAi);
    config.capacity.max_input_tokens = std::num::NonZeroU64::new(1);
    let client = client(catalog, config);
    let prepared = client.prepare("analysis", &request).await.unwrap();
    assert!(prepared.context_exceeded);
    assert_eq!(prepared.estimate, 0.0);
    let result = client.complete(&prepared).await.unwrap();
    assert!(result.result.context_exceeded);
    assert_eq!(result.result.cost, 0.0);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn grouped_deployments_round_robin_and_reserve_maximum_price(
    catalog: Arc<Catalog>,
    request: ModelRequest,
) {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    respond(&first, "/chat/completions", 200, completion()).await;
    respond(&second, "/messages", 200, anthropic_completion()).await;
    let a = deployment(&first, Provider::OpenAi);
    let mut b = deployment(&second, Provider::Anthropic);
    b.input_cost_per_token = Some(0.2);
    b.output_cost_per_token = Some(0.3);
    b.output_limits.max_tokens = std::num::NonZeroU64::new(100);
    let solo = client(catalog.clone(), b.clone());
    let expected = solo.prepare("analysis", &request).await.unwrap().estimate;
    let group = AnalysisModels::new(catalog, vec![a, b], TransportLimits::default()).unwrap();
    assert_eq!(group.models(), ["analysis"]);
    let p = group.prepare("analysis", &request).await.unwrap();
    assert_eq!(p.estimate, expected);
    group.complete(&p).await.unwrap();
    let p = group.prepare("analysis", &request).await.unwrap();
    assert_eq!(p.estimate, expected);
    group.complete(&p).await.unwrap();
    assert_eq!(
        first.received_requests().await.unwrap()[0]
            .body_json::<Value>()
            .unwrap()["max_completion_tokens"],
        100
    );
    assert_eq!(
        second.received_requests().await.unwrap()[0]
            .body_json::<Value>()
            .unwrap()["max_tokens"],
        100
    );
}

#[rstest]
#[case::duration(Duration::ZERO, 100)]
#[case::bytes(Duration::from_secs(1), 0)]
fn invalid_transport_limits_fail(
    catalog: Arc<Catalog>,
    #[case] timeout: Duration,
    #[case] max_response_bytes: usize,
) {
    assert!(matches!(
        AnalysisModels::new(
            catalog,
            vec![],
            TransportLimits {
                timeout,
                max_response_bytes
            }
        ),
        Err(Error::TransportLimits)
    ));
}

#[rstest]
#[tokio::test]
async fn unconfigured_aliases_are_not_exposed(catalog: Arc<Catalog>, request: ModelRequest) {
    let client = AnalysisModels::new(catalog, vec![], TransportLimits::default()).unwrap();
    assert!(client.models().is_empty());
    assert!(
        matches!(client.prepare("analysis",&request).await,Err(Error::UnknownModel{model}) if model=="analysis")
    );
}

#[rstest]
#[case::empty_name("", "fixture", "key")]
#[case::blank_model("analysis", "  ", "key")]
#[case::blank_key("analysis", "fixture", " \n")]
#[tokio::test]
async fn invalid_deployments_fail_before_requests(
    catalog: Arc<Catalog>,
    #[case] name: &str,
    #[case] model: &str,
    #[case] key: &str,
) {
    let server = MockServer::start().await;
    let mut config = deployment(&server, Provider::OpenAi);
    config.name = name.into();
    config.model = model.into();
    config.api_key = lens_analysis::Secret::new(key);
    assert!(matches!(
        AnalysisModels::new(catalog, vec![config], TransportLimits::default()),
        Err(Error::MissingModel | Error::MissingCredential { .. })
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::credentials("https://user:password@example.com/v1")]
#[case::username("https://user@example.com/v1")]
#[case::query("https://example.com/v1?key=secret")]
#[case::fragment("https://example.com/v1#key")]
#[case::file("file:///tmp/model")]
#[case::ftp("ftp://example.com/model")]
#[tokio::test]
async fn unsafe_api_bases_are_rejected(catalog: Arc<Catalog>, #[case] endpoint: &str) {
    let server = MockServer::start().await;
    let mut config = deployment(&server, Provider::OpenAi);
    config.api_base = Some(endpoint.parse().unwrap());
    assert!(matches!(
        AnalysisModels::new(catalog, vec![config], TransportLimits::default()),
        Err(Error::Endpoint)
    ));
}

#[rstest]
#[case::base("/v1/", Provider::OpenAi, "/v1/chat/completions")]
#[case::full(
    "/v1/chat/completions",
    Provider::OpenAiCompatible,
    "/v1/chat/completions"
)]
#[case::anthropic("/v1/messages", Provider::Anthropic, "/v1/messages")]
#[tokio::test]
async fn explicit_api_base_path_is_preserved(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] base: &str,
    #[case] provider: Provider,
    #[case] endpoint: &str,
) {
    let server = MockServer::start().await;
    respond(
        &server,
        endpoint,
        200,
        if provider == Provider::Anthropic {
            anthropic_completion()
        } else {
            completion()
        },
    )
    .await;
    let mut config = deployment(&server, provider);
    config.api_base = Some(format!("{}{base}", server.uri()).parse().unwrap());
    config.model = match provider {
        Provider::OpenAi => "openai/fixture",
        Provider::Anthropic => "anthropic/fixture",
        Provider::OpenAiCompatible => "gateway/fixture",
    }
    .into();
    let client = client(catalog, config);
    client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    let body = server.received_requests().await.unwrap()[0]
        .body_json::<Value>()
        .unwrap();
    assert_eq!(
        body["model"],
        if provider == Provider::OpenAiCompatible {
            "gateway/fixture"
        } else {
            "fixture"
        }
    );
}

#[rstest]
#[case::embedding("embedding")]
#[case::image("image_generation")]
#[tokio::test]
async fn non_chat_models_are_declined(#[case] mode: &str) {
    let server = MockServer::start().await;
    let catalog = catalog_with(
        json!({"mode":mode,"input_cost_per_token":0.01,"output_cost_per_token":0.02,"max_output_tokens":400}),
    );
    assert!(matches!(
        AnalysisModels::new(
            catalog,
            vec![deployment(&server, Provider::OpenAi)],
            TransportLimits::default()
        ),
        Err(Error::ModelSurface { .. })
    ));
}

#[rstest]
#[case::missing_price(json!({"max_output_tokens":100}))]
#[case::missing_capacity(json!({"input_cost_per_token":0.01,"output_cost_per_token":0.02}))]
#[tokio::test]
async fn incomplete_catalog_requires_explicit_config(#[case] fields: Value) {
    let server = MockServer::start().await;
    let catalog = catalog_with(fields);
    assert!(matches!(
        AnalysisModels::new(
            catalog,
            vec![deployment(&server, Provider::OpenAi)],
            TransportLimits::default()
        ),
        Err(Error::Policy(_))
    ));
}

#[rstest]
#[case::disabled(false)]
#[tokio::test]
async fn unsupported_cache_breakpoints_stay_plain(request: ModelRequest, #[case] supports: bool) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, completion()).await;
    let catalog = catalog_with(
        json!({"input_cost_per_token":0.01,"output_cost_per_token":0.02,"max_output_tokens":100,"supports_prompt_cache_breakpoint":supports}),
    );
    let client = client(catalog, deployment(&server, Provider::OpenAi));
    client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    let body = server.received_requests().await.unwrap()[0]
        .body_json::<Value>()
        .unwrap();
    assert_eq!(body["messages"][1]["content"], "Return JSON");
    assert_eq!(body["messages"][4]["content"], "Evidence B");
}

#[rstest]
#[tokio::test]
async fn custom_rates_use_reported_total_usage(catalog: Arc<Catalog>, request: ModelRequest) {
    let server = MockServer::start().await;
    respond(&server, "/messages", 200, anthropic_completion()).await;
    let mut config = deployment(&server, Provider::Anthropic);
    config.input_cost_per_token = Some(0.1);
    config.output_cost_per_token = Some(0.2);
    let client = client(catalog, config);
    let result = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    assert_eq!(result.result.cost, 14.0);
}

#[rstest]
fn secret_debug_never_discloses_value() {
    assert_eq!(
        format!("{:?}", lens_analysis::Secret::new("hidden-key")),
        "[REDACTED]"
    );
}

#[rstest]
fn bundled_catalog_loads_offline_with_provenance() {
    use sha2::{Digest, Sha256};
    let catalog = lens_analysis::bundled_catalog().unwrap();
    let provenance: Value =
        serde_json::from_slice(include_bytes!("../data/provenance.json")).unwrap();
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/model_catalog.json"))
        ),
        provenance["files"]["model_catalog.json"]["sha256"]
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../data/anthropic_tokenizer.json"))
        ),
        provenance["files"]["anthropic_tokenizer.json"]["sha256"]
    );
    assert_eq!(
        catalog.provenance().revision.as_deref(),
        provenance["revision"].as_str()
    );
    assert!(catalog.model_count() > 0);
}

#[rstest]
#[case::declared_length(false)]
#[case::chunked(true)]
#[tokio::test]
async fn response_at_byte_limit_succeeds(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] chunked: bool,
) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = serde_json::to_vec(&completion()).unwrap();
    let limit = body.len();
    let response = if chunked {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            String::from_utf8(body).unwrap()
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8(body).unwrap()
        )
    };
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        drain_request(&mut stream);
        stream.write_all(response.as_bytes()).unwrap();
    });
    let placeholder = MockServer::start().await;
    let mut config = deployment(&placeholder, Provider::OpenAi);
    config.api_base = Some(format!("http://{address}").parse().unwrap());
    let client = AnalysisModels::new(
        catalog,
        vec![config],
        TransportLimits {
            max_response_bytes: limit,
            ..TransportLimits::default()
        },
    )
    .unwrap();
    let result = client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    assert_eq!(result.result.content, "{\"ok\":true}");
    handle.join().unwrap();
}

#[rstest]
#[tokio::test]
async fn chunked_response_cannot_bypass_limit(catalog: Arc<Catalog>, request: ModelRequest) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        drain_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n6\r\nabcdef\r\n6\r\nghijkl\r\n0\r\n\r\n").unwrap();
    });
    let placeholder = MockServer::start().await;
    let mut config = deployment(&placeholder, Provider::OpenAi);
    config.api_base = Some(format!("http://{address}").parse().unwrap());
    let client = AnalysisModels::new(
        catalog,
        vec![config],
        TransportLimits {
            max_response_bytes: 10,
            ..TransportLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::ResponseLimit)
    ));
    handle.join().unwrap();
}

#[rstest]
#[case::openai(Provider::OpenAi,"/chat/completions",json!({"choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":9223372036854775808u64}}))]
#[case::anthropic_sum(Provider::Anthropic,"/messages",json!({"model":"fixture","content":[],"usage":{"input_tokens":18446744073709551615u64,"output_tokens":0,"cache_read_input_tokens":1}}))]
#[case::completion_tokens(Provider::OpenAi,"/chat/completions",json!({"choices":[{"message":{"content":"{}"}}],"usage":{"completion_tokens":9223372036854775808u64}}))]
#[tokio::test]
async fn reported_usage_cannot_overflow(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] endpoint: &str,
    #[case] body: Value,
) {
    let server = MockServer::start().await;
    respond(&server, endpoint, 200, body).await;
    let client = client(catalog, deployment(&server, provider));
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::TokenCount)
    ));
}

#[rstest]
#[case::tool_use(json!({"type":"tool_use","id":"tool","name":"unexpected","input":{}}))]
#[case::unknown(json!({"type":"future_block","text":"not a completion"}))]
#[tokio::test]
async fn anthropic_unsupported_results_are_declined(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] block: Value,
) {
    let server = MockServer::start().await;
    let mut body = anthropic_completion();
    body["content"] = json!([block]);
    respond(&server, "/messages", 200, body).await;
    let client = client(catalog, deployment(&server, Provider::Anthropic));
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::ResponseContent)
    ));
}

#[rstest]
#[tokio::test]
async fn valid_deployment_is_selected_when_other_is_over_context(
    catalog: Arc<Catalog>,
    request: ModelRequest,
) {
    let tiny = MockServer::start().await;
    let large = MockServer::start().await;
    respond(&large, "/chat/completions", 200, completion()).await;
    let mut first = deployment(&tiny, Provider::OpenAi);
    first.capacity.max_input_tokens = std::num::NonZeroU64::new(1);
    let second = deployment(&large, Provider::OpenAi);
    let client =
        AnalysisModels::new(catalog, vec![first, second], TransportLimits::default()).unwrap();
    let prepared = client.prepare("analysis", &request).await.unwrap();
    assert!(!prepared.context_exceeded);
    assert_eq!(
        client.complete(&prepared).await.unwrap().result.content,
        "{\"ok\":true}"
    );
    assert!(tiny.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn model_groups_describe_configured_aliases_only(catalog: Arc<Catalog>) {
    let server = MockServer::start().await;
    let first = deployment(&server, Provider::OpenAi);
    let second = deployment(&server, Provider::Anthropic);
    let mut third = deployment(&server, Provider::OpenAiCompatible);
    third.name = "other".into();
    let models = AnalysisModels::new(
        catalog,
        vec![first.clone(), second, first, third],
        TransportLimits::default(),
    )
    .unwrap();
    assert_eq!(models.models(), ["analysis", "other"]);
    assert_eq!(
        serde_json::to_value(models.model_groups()).unwrap(),
        json!([
            {"model_group":"analysis","providers":["anthropic","openai"],"mode":"chat","supported_openai_params":null},
            {"model_group":"other","providers":["openai_compatible"],"mode":"chat","supported_openai_params":null},
        ])
    );
}

#[rstest]
#[case::capacity_only(None, 73)]
#[case::explicit_output(Some(31), 31)]
#[tokio::test]
async fn configured_output_capacity_reaches_provider(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] explicit: Option<u64>,
    #[case] expected: u64,
) {
    let server = MockServer::start().await;
    respond(&server, "/chat/completions", 200, completion()).await;
    let mut config = deployment(&server, Provider::OpenAi);
    config.capacity.max_output_tokens = std::num::NonZeroU64::new(73);
    config.output_limits.max_output_tokens = explicit.and_then(std::num::NonZeroU64::new);
    let client = client(catalog, config);
    client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        server.received_requests().await.unwrap()[0]
            .body_json::<Value>()
            .unwrap()["max_completion_tokens"],
        expected
    );
}

#[rstest]
#[case::anthropic_only(Provider::Anthropic, true, false, true)]
#[case::anthropic_disabled(Provider::Anthropic, false, true, false)]
#[case::openai_only(Provider::OpenAi, false, true, true)]
#[case::openai_disabled(Provider::OpenAi, true, false, false)]
#[tokio::test]
async fn cache_capability_uses_selected_protocol(
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] prompt_caching: bool,
    #[case] prompt_breakpoint: bool,
    #[case] cached: bool,
) {
    let catalog = catalog_with(
        json!({"input_cost_per_token":0.01,"output_cost_per_token":0.02,"max_output_tokens":100,"supports_prompt_caching":prompt_caching,"supports_prompt_cache_breakpoint":prompt_breakpoint}),
    );
    let server = MockServer::start().await;
    respond(
        &server,
        if provider == Provider::Anthropic {
            "/messages"
        } else {
            "/chat/completions"
        },
        200,
        if provider == Provider::Anthropic {
            anthropic_completion()
        } else {
            completion()
        },
    )
    .await;
    let client = client(catalog, deployment(&server, provider));
    client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    let body = server.received_requests().await.unwrap()[0]
        .body_json::<Value>()
        .unwrap();
    let actual = if provider == Provider::Anthropic {
        body["system"][1].get("cache_control").is_some()
    } else {
        body["messages"][1]["content"].is_array()
    };
    assert_eq!(actual, cached);
}

#[rstest]
#[case::legacy("gpt-3.5-turbo-0301", 4, 0.0)]
#[case::standard("fixture", 3, 0.0)]
#[case::cache_write_reservation("fixture", 3, 2.0)]
#[tokio::test]
async fn prompt_estimate_preserves_message_accounting(
    request: ModelRequest,
    #[case] model: &str,
    #[case] message_tokens: u64,
    #[case] cache_price: f64,
) {
    let catalog=Arc::new(Catalog::parse(&serde_json::to_vec(&json!({model:{"litellm_provider":"openai","mode":"chat","input_cost_per_token":1.0,"output_cost_per_token":0.0,"cache_creation_input_token_cost":cache_price,"max_output_tokens":100}})).unwrap(),Default::default()).unwrap());
    let server = MockServer::start().await;
    let mut config = deployment(&server, Provider::OpenAi);
    config.model = model.into();
    let client = client(catalog, config);
    let actual = client.prepare("analysis", &request).await.unwrap().estimate;
    let counter = litellm_token_counter::TokenCounter::from_tiktoken("cl100k_base").unwrap();
    let messages = lens_inference::request_messages(&request).unwrap();
    let count = messages
        .iter()
        .map(|message| {
            counter.count_text(&message.content).unwrap() as u64
                + counter.count_text(&message.role.to_string()).unwrap() as u64
                + message_tokens
        })
        .sum::<u64>()
        + 3;
    assert_eq!(actual, count as f64 * 1.0_f64.max(cache_price));
}

#[rstest]
#[case::unrelated_anthropic(Provider::Anthropic,"/messages",json!({"error":{"type":"invalid_request_error","message":"invalid max_tokens"}}))]
#[case::wrong_anthropic_type(Provider::Anthropic,"/messages",json!({"error":{"type":"authentication_error","message":"prompt is too long: this is not a context error"}}))]
#[case::wrong_provider(Provider::OpenAi,"/chat/completions",json!({"error":{"type":"invalid_request_error","message":"prompt is too long: this is not an Anthropic error"}}))]
#[tokio::test]
async fn context_message_requires_matching_provider_and_error_type(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] provider: Provider,
    #[case] endpoint: &str,
    #[case] body: Value,
) {
    let server = MockServer::start().await;
    respond(&server, endpoint, 400, body).await;
    let client = client(catalog, deployment(&server, provider));
    assert!(matches!(
        client
            .complete(&client.prepare("analysis", &request).await.unwrap())
            .await,
        Err(Error::Provider { status: 400, .. })
    ));
}

#[rstest]
#[case::interleaved(json!([
    {"role":"user","content":"First"},
    {"role":"system","content":"Later instruction"},
    {"role":"assistant","content":"{}"},
    {"role":"user","content":"Second"},
    {"role":"user","content":"Third"}
]))]
#[case::empty_system(json!([
    {"role":"system","content":""},
    {"role":"system","content":"Later instruction"},
    {"role":"user","content":"First"},
    {"role":"user","content":"Second"}
]))]
#[tokio::test]
async fn lifted_system_cache_marks_follow_original_message(
    catalog: Arc<Catalog>,
    #[case] messages: Value,
) {
    let request: ModelRequest =
        serde_json::from_value(json!({"prompt":"unused","purpose":"extract","messages":messages}))
            .unwrap();
    let server = MockServer::start().await;
    respond(&server, "/messages", 200, anthropic_completion()).await;
    let client = client(catalog, deployment(&server, Provider::Anthropic));
    client
        .complete(&client.prepare("analysis", &request).await.unwrap())
        .await
        .unwrap();
    let body = server.received_requests().await.unwrap()[0]
        .body_json::<Value>()
        .unwrap();
    assert_eq!(body["system"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["system"][1],
        json!({"type":"text","text":"Later instruction"})
    );
}

#[rstest]
#[case::integer("37", Some(37))]
#[case::zero("0", Some(0))]
#[case::invalid("later", None)]
#[case::negative("-1", None)]
#[case::too_large("18446744073709551616", None)]
#[tokio::test]
async fn retry_after_is_preserved_without_retrying(
    catalog: Arc<Catalog>,
    request: ModelRequest,
    #[case] value: &str,
    #[case] expected: Option<u64>,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", value))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(catalog, deployment(&server, Provider::OpenAi));
    assert!(
        matches!(client.complete(&client.prepare("analysis",&request).await.unwrap()).await,Err(Error::Provider{status:429,retry_after,..}) if retry_after==expected)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
