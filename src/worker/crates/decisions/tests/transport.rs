use lens_contract::signals::SignalStep;
use lens_decisions::{Deployment, Error, EvaluationModels, Provider, Secret, TransportLimits};
use lens_signals::{DecisionRequest, Question, SignalState};
use litellm_model_catalog::{Catalog, Provenance};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[rstest]
#[case::gateway(Provider::DecisionsCompatible, "", "/v1/decisions", true)]
#[case::different_base(Provider::DecisionsCompatible, "/other", "/v1/decisions", false)]
#[case::native_passthrough(Provider::Typesafe, "", "/v1/systemone", true)]
#[tokio::test]
async fn gateway_marker_only_reaches_the_configured_compatible_transport(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] provider: Provider,
    #[case] base_path: &str,
    #[case] endpoint: &str,
    #[case] marked: bool,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":{}})))
        .expect(1)
        .mount(&server)
        .await;
    let key = "gateway-transport-fixture-key-32-bytes";
    let gateway = lens_inference::GatewayIdentity::new(
        format!("{}{base_path}", server.uri()).parse().unwrap(),
        key,
    )
    .unwrap();
    let client = models(
        vec![Deployment {
            provider,
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        TransportLimits::default(),
    )
    .unwrap()
    .with_gateway(Some(gateway));
    client.evaluate(&request).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer private-key"
    );
    let marker = requests[0].headers.get(lens_inference::GATEWAY_HEADER);
    assert_eq!(marker.is_some(), marked);
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
        assert_eq!(claims["purpose"], "signals");
    }
}

async fn drain(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut input = Vec::new();
    loop {
        let mut bytes = [0; 2048];
        let size = socket.read(&mut bytes).await.unwrap();
        assert_ne!(size, 0);
        input.extend_from_slice(&bytes[..size]);
        if let Some(index) = input.windows(4).position(|window| window == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&input[..index]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                })
                .unwrap();
            if input.len() >= index + 4 + length {
                return;
            }
        }
    }
}

#[rstest]
#[case::exact(14, true)]
#[case::over(13, false)]
#[tokio::test]
async fn chunked_responses_enforce_the_actual_byte_limit(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] limit: usize,
    #[case] valid: bool,
) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        drain(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n7\r\n{\"answe\r\n7\r\nrs\":{}}\r\n0\r\n\r\n").await.unwrap();
    });
    let api = models(
        vec![Deployment {
            api_base: Some(format!("http://{address}").parse().unwrap()),
            ..deployment
        }],
        TransportLimits {
            max_response_bytes: limit,
            ..Default::default()
        },
    )
    .unwrap();
    let result = api.evaluate(&request).await;
    assert_eq!(result.is_ok(), valid);
    if valid {
        assert_eq!(result.unwrap(), json!({"answers":{}}));
    } else {
        assert!(matches!(result, Err(Error::ResponseLimit)));
    }
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn delayed_response_body_respects_the_whole_call_deadline(
    deployment: Deployment,
    request: DecisionRequest,
) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        drain(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n1\r\n{\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let api = models(
        vec![Deployment {
            api_base: Some(format!("http://{address}").parse().unwrap()),
            ..deployment
        }],
        TransportLimits {
            timeout: Duration::from_millis(30),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(api.evaluate(&request).await, Err(Error::Timeout)));
    server.abort();
}

#[rstest]
#[tokio::test]
async fn duplicate_aliases_round_robin_without_retrying_a_failed_attempt(
    deployment: Deployment,
    request: DecisionRequest,
) {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(2)
        .mount(&first)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":{}})))
        .expect(1)
        .mount(&second)
        .await;
    let api = models(
        vec![
            Deployment {
                api_base: Some(first.uri().parse().unwrap()),
                ..deployment.clone()
            },
            Deployment {
                api_base: Some(second.uri().parse().unwrap()),
                ..deployment
            },
        ],
        Default::default(),
    )
    .unwrap();
    assert!(matches!(
        api.evaluate(&request).await,
        Err(Error::Provider { status: 503 })
    ));
    assert!(second.received_requests().await.unwrap().is_empty());
    assert_eq!(api.evaluate(&request).await.unwrap(), json!({"answers":{}}));
    assert!(matches!(
        api.evaluate(&request).await,
        Err(Error::Provider { status: 503 })
    ));
}

#[rstest]
#[tokio::test]
async fn malformed_provider_json_is_not_a_success(
    deployment: Deployment,
    request: DecisionRequest,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        Default::default(),
    )
    .unwrap();
    assert!(matches!(api.evaluate(&request).await, Err(Error::Json(_))));
}

#[fixture]
fn request() -> DecisionRequest {
    DecisionRequest {
        model: "signals".into(),
        state: SignalState {
            task: "Classify the trace",
            steps: vec![SignalStep {
                kind: "user".into(),
                name: "message".into(),
                content: "I need help".into(),
            }],
        },
        questions: BTreeMap::from([(
            "a".into(),
            Question {
                r#type: "noul",
                instructions: "Is help requested?".into(),
            },
        )]),
        timeout: Duration::from_secs(2),
        tags: vec!["litellm-lens-signals"],
    }
}
#[fixture]
fn deployment() -> Deployment {
    Deployment {
        name: "signals".into(),
        model: "typesafe/test-evaluator".into(),
        provider: Provider::Typesafe,
        api_base: None,
        api_key: Some(Secret::new("private-key")),
    }
}
fn models(
    deployments: Vec<Deployment>,
    limits: TransportLimits,
) -> Result<EvaluationModels, Error> {
    EvaluationModels::new(
        Arc::new(
            Catalog::parse(
                br#"{"evaluation-fixture":{"mode":"evaluation","litellm_provider":"typesafe"}}"#,
                Default::default(),
            )
            .unwrap(),
        ),
        deployments,
        limits,
    )
}

#[rstest]
#[case::typesafe(
    Provider::Typesafe,
    "typesafe/test-evaluator",
    "/v1",
    "/v1/systemone",
    "test-evaluator"
)]
#[case::perplexity(
    Provider::Perplexity,
    "perplexity/test-evaluator",
    "",
    "/v1/decisions",
    "test-evaluator"
)]
#[case::openrouter(
    Provider::OpenRouter,
    "openrouter/typesafe/test-evaluator",
    "/api/v1/",
    "/api/alpha/decisions",
    "typesafe/test-evaluator"
)]
#[case::compatible(
    Provider::DecisionsCompatible,
    "gateway/model-alias",
    "/gateway/v1",
    "/gateway/v1/decisions",
    "gateway/model-alias"
)]
#[case::strands(
    Provider::StrandsDecider,
    "strands_decider/test-evaluator",
    "/base",
    "/base/v1/systemone",
    "test-evaluator"
)]
#[case::cloudflare_root(
    Provider::Cloudflare,
    "cloudflare/clef-test",
    "/account",
    "/account/ai/run/@cf/cloudflare/clef-test",
    "clef-test"
)]
#[case::cloudflare_v1(
    Provider::Cloudflare,
    "cloudflare/@cf/cloudflare/clef-test",
    "/account/ai/v1",
    "/account/ai/run/@cf/cloudflare/clef-test",
    "clef-test"
)]
#[case::cloudflare_run(
    Provider::Cloudflare,
    "@cf/vendor/clef-test",
    "/account/ai/run/",
    "/account/ai/run/@cf/vendor/clef-test",
    "clef-test"
)]
#[tokio::test]
async fn adapters_preserve_provider_paths_model_names_and_decision_body(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] provider: Provider,
    #[case] model: &str,
    #[case] base: &str,
    #[case] endpoint: &str,
    #[case] sent_model: &str,
) {
    let server = MockServer::start().await;
    let response = json!({"answers":{"a":{"type":"noul","noul":0.8}}});
    Mock::given(method("POST"))
        .and(path(endpoint))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            provider,
            model: model.into(),
            api_base: Some(format!("{}{base}", server.uri()).parse().unwrap()),
            ..deployment
        }],
        Default::default(),
    )
    .unwrap();
    assert_eq!(api.evaluate(&request).await.unwrap(), response);
    let requests = server.received_requests().await.unwrap();
    let sent = &requests[0];
    let body: Value = sent.body_json().unwrap();
    assert_eq!(sent.headers["authorization"], "Bearer private-key");
    assert_eq!(body["model"], sent_model);
    assert_eq!(body["state"], serde_json::to_value(&request.state).unwrap());
    assert_eq!(
        body["questions"],
        serde_json::to_value(&request.questions).unwrap()
    );
    assert_eq!(
        body.get("metadata").cloned(),
        if provider == Provider::DecisionsCompatible {
            Some(json!({"tags":["litellm-lens-signals"]}))
        } else {
            None
        }
    );
}

#[rstest]
#[case::wrapped(json!({"result":{"answers":{"a":{"type":"noul","noul":0.2}}}}),json!({"answers":{"a":{"type":"noul","noul":0.2}}}))]
#[case::top_level_wins(json!({"answers":{},"result":{"answers":{"wrong":1}}}),json!({"answers":{},"result":{"answers":{"wrong":1}}}))]
#[case::nonobject_result(json!({"result":[]}),json!({"result":[]}))]
#[tokio::test]
async fn cloudflare_response_unwrap_matches_the_provider_contract(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] body: Value,
    #[case] expected: Value,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            provider: Provider::Cloudflare,
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        Default::default(),
    )
    .unwrap();
    assert_eq!(api.evaluate(&request).await.unwrap(), expected);
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn private_strands_endpoint_can_omit_authentication(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] key: Option<&str>,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":{}})))
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            provider: Provider::StrandsDecider,
            api_base: Some(server.uri().parse().unwrap()),
            api_key: key.map(Secret::new),
            ..deployment
        }],
        Default::default(),
    )
    .unwrap();
    api.evaluate(&request).await.unwrap();
    assert!(
        !server.received_requests().await.unwrap()[0]
            .headers
            .contains_key("authorization")
    );
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::rate_limited(429)]
#[case::server_failure(500)]
#[case::redirect(302)]
#[tokio::test]
async fn failures_are_sanitized_and_never_retried_or_redirected(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] status: u16,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Location", format!("{}/leaked", server.uri()))
                .set_body_string("secret-provider-detail"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        Default::default(),
    )
    .unwrap();
    let error = api.evaluate(&request).await.unwrap_err();
    assert!(matches!(error,Error::Provider{status:actual}if actual==status));
    assert!(!error.to_string().contains("secret"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[case::exact(14, true)]
#[case::over(13, false)]
#[tokio::test]
async fn response_size_is_bounded(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] limit: usize,
    #[case] valid: bool,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"answers":{}}"#))
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        TransportLimits {
            max_response_bytes: limit,
            ..Default::default()
        },
    )
    .unwrap();
    let result = api.evaluate(&request).await;
    assert_eq!(result.is_ok(), valid);
    if !valid {
        assert!(matches!(result, Err(Error::ResponseLimit)));
    }
}

#[rstest]
#[tokio::test]
async fn timeout_covers_the_provider_call(deployment: Deployment, request: DecisionRequest) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(100))
                .set_body_json(json!({"answers":{}})),
        )
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        TransportLimits {
            timeout: Duration::from_millis(10),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(api.evaluate(&request).await, Err(Error::Timeout)));
}

#[rstest]
#[case::ftp("ftp://example.test")]
#[case::userinfo("https://name:password@example.test")]
#[case::username("https://name@example.test")]
#[case::password("https://:password@example.test")]
#[case::query("https://example.test?token=secret")]
#[case::fragment("https://example.test#secret")]
fn invalid_api_bases_fail_before_any_request(deployment: Deployment, #[case] base: &str) {
    assert!(matches!(
        models(
            vec![Deployment {
                api_base: Some(base.parse().unwrap()),
                ..deployment
            }],
            Default::default()
        ),
        Err(Error::Endpoint)
    ));
}

#[rstest]
#[case::cloudflare(Provider::Cloudflare)]
#[case::strands(Provider::StrandsDecider)]
#[case::compatible(Provider::DecisionsCompatible)]
fn explicit_connections_require_an_api_base(deployment: Deployment, #[case] provider: Provider) {
    assert!(matches!(
        models(
            vec![Deployment {
                provider,
                ..deployment
            }],
            Default::default()
        ),
        Err(Error::Endpoint)
    ));
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
#[case::blank(Some(" "))]
fn authenticated_providers_require_a_key(deployment: Deployment, #[case] key: Option<&str>) {
    assert!(matches!(
        models(
            vec![Deployment {
                api_key: key.map(Secret::new),
                ..deployment
            }],
            Default::default()
        ),
        Err(Error::MissingCredential { .. })
    ));
}

#[rstest]
#[case::token("private-token")]
#[case::multiline("private\ncredential")]
fn credentials_are_redacted_in_debug_output(deployment: Deployment, #[case] key: &str) {
    let secret = Secret::new(key);
    assert_eq!(format!("{secret:?}"), "[REDACTED]");
    let rendered = format!(
        "{:?}",
        Deployment {
            api_key: Some(secret),
            ..deployment
        }
    );
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains(key));
}

#[rstest]
#[case::name(" ", "model")]
#[case::model("name", "")]
fn deployment_identifiers_must_be_nonempty(
    deployment: Deployment,
    #[case] name: &str,
    #[case] model: &str,
) {
    assert!(matches!(
        models(
            vec![Deployment {
                name: name.into(),
                model: model.into(),
                ..deployment
            }],
            Default::default()
        ),
        Err(Error::MissingModel)
    ));
}

#[rstest]
#[case::timeout(Duration::ZERO, 1)]
#[case::bytes(Duration::from_secs(1), 0)]
fn transport_limits_must_be_positive(#[case] timeout: Duration, #[case] bytes: usize) {
    assert!(matches!(
        models(
            vec![],
            TransportLimits {
                timeout,
                max_response_bytes: bytes
            }
        ),
        Err(Error::Limits)
    ));
}

#[rstest]
#[tokio::test]
async fn unknown_alias_is_rejected(request: DecisionRequest) {
    assert!(matches!(
        models(vec![], Default::default())
            .unwrap()
            .evaluate(&request)
            .await,
        Err(Error::UnknownModel { .. })
    ));
}

#[rstest]
fn known_chat_models_cannot_be_configured_for_evaluation(deployment: Deployment) {
    let catalog = Catalog::parse(
        br#"{"test":{"mode":"chat","litellm_provider":"typesafe"}}"#,
        Provenance::default(),
    )
    .unwrap();
    assert!(matches!(
        EvaluationModels::new(
            Arc::new(catalog),
            vec![Deployment {
                model: "test".into(),
                ..deployment
            }],
            Default::default()
        ),
        Err(Error::ModelSurface { .. })
    ));
}

#[rstest]
fn model_groups_are_sorted_configured_and_deduplicated(deployment: Deployment) {
    let api = models(
        vec![
            deployment.clone(),
            deployment.clone(),
            Deployment {
                name: "a".into(),
                provider: Provider::Perplexity,
                ..deployment.clone()
            },
            Deployment {
                provider: Provider::OpenRouter,
                ..deployment
            },
        ],
        Default::default(),
    )
    .unwrap();
    assert_eq!(api.models(), vec!["a", "signals"]);
    let groups = api.model_groups();
    assert_eq!(groups[0].providers, vec!["perplexity"]);
    assert_eq!(groups[1].providers, vec!["openrouter", "typesafe"]);
    assert_eq!(groups[1].mode, "evaluation");
    assert!(groups[1].supported_openai_params.is_none());
}

#[rstest]
#[case::success(200, false, 65536, None)]
#[case::provider(429, false, 65536, Some("Lens signal provider returned HTTP 429"))]
#[case::limit(
    200,
    false,
    1,
    Some("Lens signal provider response exceeds the configured limit")
)]
#[case::timeout(200, true, 65536, Some("Lens signal model request timed out"))]
#[tokio::test]
async fn decisions_trait_preserves_safe_failure_categories(
    deployment: Deployment,
    request: DecisionRequest,
    #[case] status: u16,
    #[case] delayed: bool,
    #[case] bytes: usize,
    #[case] expected: Option<&str>,
) {
    use lens_signals::Decisions;
    let server = MockServer::start().await;
    let response = ResponseTemplate::new(status)
        .set_body_json(json!({"answers":{}}))
        .set_delay(if delayed {
            Duration::from_millis(100)
        } else {
            Duration::ZERO
        });
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    let api = models(
        vec![Deployment {
            api_base: Some(server.uri().parse().unwrap()),
            ..deployment
        }],
        TransportLimits {
            timeout: if delayed {
                Duration::from_millis(20)
            } else {
                Duration::from_secs(2)
            },
            max_response_bytes: bytes,
        },
    )
    .unwrap();
    let result = api.complete(&request).await;
    assert_eq!(
        result.as_ref().err().map(ToString::to_string).as_deref(),
        expected
    );
    if expected.is_none() {
        assert_eq!(result.unwrap(), json!({"answers":{}}));
    }
}

#[rstest]
#[tokio::test]
async fn configuration_failure_is_sanitized_by_the_decisions_trait(request: DecisionRequest) {
    use lens_signals::Decisions;
    let result = models(vec![], Default::default())
        .unwrap()
        .complete(&request)
        .await
        .unwrap_err();
    assert!(matches!(
        result,
        lens_signals::DecisionsError::Unavailable(_)
    ));
    assert_eq!(result.to_string(), "Lens signal model request failed");
}
