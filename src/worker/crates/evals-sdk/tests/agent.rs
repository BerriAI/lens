use lens_contract::agent_io::AgentIo;
use lens_evals_sdk::{
    agent::{Connection, Executor, ResolvedConnection},
    model::Case,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

#[fixture]
fn case() -> Case {
    Case {
        id: "case-1".into(),
        input: "fix the bug".into(),
        followups: vec![],
        meta: BTreeMap::new(),
        expected: "never send the expected answer".into(),
    }
}
#[fixture]
fn immediate() -> AgentIo {
    serde_json::from_value(json!({"version":1,"connection":"test","submit":{"method":"POST","path":"/run","accepted_status":200,"json":{"prompt":"","request_id":"","fixed":{"mode":"real"}}},"input":[{"source":"case.input","target":"/prompt"},{"source":"trial.request_id","target":"/request_id"}],"completion":{"kind":"immediate"},"output":{"pointer":"/output","require_nonempty":true}})).unwrap()
}
#[fixture]
fn polled(immediate: AgentIo) -> AgentIo {
    let mut value = serde_json::to_value(immediate).unwrap();
    value["completion"] = json!({"kind":"poll","id_pointer":"/id","path_template":"/jobs/{id}","status_pointer":"/status","error_pointer":"/error","success":["completed","idle"],"failure":["failed","cancelled","interrupted"],"interval_ms":250,"timeout_ms":1000,"require_item":{"array_pointer":"/messages","matches":{"role":"assistant","status":"completed"}}});
    value["output"]["pointer"] = json!("/summary");
    value["trace"] = json!({"source":"accepted","attribute":"session.id","pointer":"/id"});
    serde_json::from_value(value).unwrap()
}
async fn executor(server: &MockServer, io: AgentIo) -> Executor {
    let profile: Connection =
        serde_json::from_value(json!({"auth":"bearer","base_url_env":"URL","token_env":"TOKEN"}))
            .unwrap();
    let connection = profile
        .resolve(|name| match name {
            "URL" => Some(server.uri()),
            "TOKEN" => Some("agent-secret-token".into()),
            _ => None,
        })
        .unwrap();
    Executor::connect(io, connection).await.unwrap()
}
#[rstest]
#[tokio::test]
async fn maps_only_declared_input_and_returns_output(immediate: AgentIo, case: Case) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/run"))
        .and(header("Authorization", "Bearer agent-secret-token"))
        .and(body_json(
            json!({"prompt":"fix the bug","request_id":"stable-id","fixed":{"mode":"real"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output":"fixed"})))
        .expect(1)
        .mount(&server)
        .await;
    let result = executor(&server, immediate)
        .await
        .run(case, "stable-id".into(), Duration::from_secs(1))
        .await;
    assert_eq!(result.output.as_deref(), Some("fixed"));
    assert!(result.trace.is_none() && result.error.is_none());
}
#[rstest]
#[case::redirect(302)]
#[case::server_error(503)]
#[case::unauthorized(401)]
#[tokio::test]
async fn does_not_repeat_an_ambiguous_submission(
    immediate: AgentIo,
    case: Case,
    #[case] status: u16,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/run"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Location", "/other")
                .set_body_json(json!({"detail":"agent-secret-token"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = executor(&server, immediate)
        .await
        .run(case, "stable-id".into(), Duration::from_secs(1))
        .await;
    assert_eq!(
        result.error.unwrap().message,
        format!("Agent HTTP {status}")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
#[rstest]
#[case::missing(json!({}))]
#[case::wrong_type(json!({"output":42}))]
#[case::empty(json!({"output":" "}))]
#[tokio::test]
async fn invalid_output_is_a_trial_error(immediate: AgentIo, case: Case, #[case] response: Value) {
    let server = MockServer::start().await;
    Mock::given(path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .mount(&server)
        .await;
    let result = executor(&server, immediate)
        .await
        .run(case, "id".into(), Duration::from_secs(1))
        .await;
    assert!(result.error.is_some() && result.output.is_none());
}
#[rstest]
#[tokio::test]
async fn unmapped_followups_are_rejected_before_submit(immediate: AgentIo, case: Case) {
    let server = MockServer::start().await;
    let result = executor(&server, immediate)
        .await
        .run(
            Case {
                followups: vec!["then check".into()],
                ..case
            },
            "id".into(),
            Duration::from_secs(1),
        )
        .await;
    assert!(result.error.unwrap().message.contains("followups"));
    assert!(server.received_requests().await.unwrap().is_empty());
}
#[rstest]
#[tokio::test]
async fn polls_encoded_id_until_success_and_completed_assistant(polled: AgentIo, case: Case) {
    let server = MockServer::start().await;
    Mock::given(path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"job/one?x=2"})))
        .expect(1)
        .mount(&server)
        .await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    Mock::given(method("GET")).and(path("/jobs/job%2Fone%3Fx=2")).respond_with(move |_: &wiremock::Request| {
        let number = counted.fetch_add(1, Ordering::SeqCst);
        let response = if number == 0 { json!({"status":"queued"}) } else { json!({"status":"idle","summary":if number == 1 {"premature"} else {"settled output"},"messages":if number == 1 {json!([])} else {json!([{"role":"assistant","status":"completed"}])}}) };
        ResponseTemplate::new(200).set_body_json(response)
    }).expect(3).mount(&server).await;
    let result = executor(&server, polled)
        .await
        .run(case, "id".into(), Duration::from_secs(2))
        .await;
    assert_eq!(result.output.as_deref(), Some("settled output"));
    assert_eq!(result.trace.unwrap().value, "job/one?x=2");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}
#[rstest]
#[tokio::test]
async fn acceptance_and_polling_share_one_deadline(polled: AgentIo, case: Case) {
    let server = MockServer::start().await;
    Mock::given(path("/run"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(80))
                .set_body_json(json!({"id":"job"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/jobs/job")).respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(80)).set_body_json(json!({"status":"completed","summary":"answer","messages":[{"role":"assistant","status":"completed"}]}))).expect(1).mount(&server).await;
    let result = executor(&server, polled)
        .await
        .run(case, "id".into(), Duration::from_millis(120))
        .await;
    assert_eq!(result.error.unwrap().r#type, "AgentTimeoutError");
    assert!(result.output.is_none());
}
#[rstest]
#[case::failed("failed")]
#[case::cancelled("cancelled")]
#[case::interrupted("interrupted")]
#[tokio::test]
async fn terminal_failure_never_reads_summary(polled: AgentIo, case: Case, #[case] status: &str) {
    let server = MockServer::start().await;
    Mock::given(path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"job"})))
        .mount(&server)
        .await;
    Mock::given(path("/jobs/job")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":status,"summary":"looks successful","error":"upstream rejected agent-secret-token"}))).mount(&server).await;
    let result = executor(&server, polled)
        .await
        .run(case, "id".into(), Duration::from_secs(1))
        .await;
    assert!(result.error.is_some() && result.output.is_none());
    assert_eq!(
        result.error.unwrap().message,
        "Agent execution failed: upstream rejected [redacted]"
    );
}
#[fixture]
fn session_io() -> AgentIo {
    serde_json::from_value(json!({"version":1,"connection":"moyai","submit":{"method":"POST","path":"/api/runs","accepted_status":201,"json":{"prompt":"","client_id":"","mode":"modal","chat_enabled":true}},"input":[{"source":"case.input","target":"/prompt"},{"source":"trial.request_id","target":"/client_id"}],"completion":{"kind":"immediate"},"output":{"pointer":"/output","require_nonempty":true}})).unwrap()
}
async fn session_server(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/login"))
        .and(header("Origin", server.uri().as_str()))
        .and(body_json(json!({"password":"private-password"})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "Set-Cookie",
                    "workspace_session=private-cookie; Path=/; HttpOnly",
                )
                .set_body_json(json!({"ok":true})),
        )
        .expect(1)
        .mount(server)
        .await;
    Mock::given(path("/api/session"))
        .and(header("Cookie", "workspace_session=private-cookie"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"authenticated":true,"csrf":"private-csrf"})),
        )
        .expect(1)
        .mount(server)
        .await;
}
fn session_connection(server: &MockServer) -> ResolvedConnection {
    let profile: Connection = serde_json::from_value(
        json!({"auth":"moyai_session","base_url_env":"URL","password_env":"PASSWORD"}),
    )
    .unwrap();
    profile
        .resolve(|name| match name {
            "URL" => Some(server.uri()),
            "PASSWORD" => Some("private-password".into()),
            _ => None,
        })
        .unwrap()
}
#[rstest]
#[tokio::test]
async fn session_auth_is_reused_for_concurrent_trials(session_io: AgentIo, case: Case) {
    let server = MockServer::start().await;
    session_server(&server).await;
    Mock::given(path("/api/runs"))
        .and(header("Origin", server.uri().as_str()))
        .and(header("Cookie", "workspace_session=private-cookie"))
        .and(header("X-CSRF-Token", "private-csrf"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"output":"answer"})))
        .expect(2)
        .mount(&server)
        .await;
    let executor = Executor::connect(session_io, session_connection(&server))
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        executor.run(case.clone(), "one".into(), Duration::from_secs(1)),
        executor.run(case, "two".into(), Duration::from_secs(1))
    );
    assert!(first.error.is_none() && second.error.is_none());
}
#[rstest]
#[case::not_authenticated(false, "csrf", true)]
#[case::no_csrf(true, "", true)]
#[case::no_cookie(true, "csrf", false)]
#[tokio::test]
async fn invalid_session_refuses_before_agent_post(
    session_io: AgentIo,
    #[case] authenticated: bool,
    #[case] csrf: &str,
    #[case] cookie: bool,
) {
    let server = MockServer::start().await;
    let response = ResponseTemplate::new(200).set_body_json(json!({"ok":true}));
    let response = if cookie {
        response.insert_header("Set-Cookie", "workspace_session=private-cookie; Path=/")
    } else {
        response
    };
    Mock::given(path("/api/login"))
        .respond_with(response)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/session"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"authenticated":authenticated,"csrf":csrf})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        Executor::connect(session_io, session_connection(&server))
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[rstest]
#[tokio::test]
async fn explicitly_optional_empty_output_is_preserved(immediate: AgentIo, case: Case) {
    let server = MockServer::start().await;
    Mock::given(path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output":""})))
        .mount(&server)
        .await;
    let mut io = immediate;
    io.output.require_nonempty = false;
    let result = executor(&server, io)
        .await
        .run(case, "id".into(), Duration::from_secs(1))
        .await;
    assert_eq!(result.output.as_deref(), Some(""));
    assert!(result.validate().is_ok());
}

#[rstest]
#[case::absolute("https://untrusted.example/run")]
#[case::network_path("//untrusted.example/run")]
#[tokio::test]
async fn off_origin_contract_rejected_before_requests(immediate: AgentIo, #[case] target: &str) {
    let server = MockServer::start().await;
    let mut io = immediate;
    io.submit.path = target.into();
    assert!(
        Executor::connect(io, session_connection(&server))
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::mode("/mode")]
#[case::chat("/chat_enabled")]
#[tokio::test]
async fn session_bindings_cannot_override_execution_safety(
    session_io: AgentIo,
    #[case] target: &str,
) {
    let server = MockServer::start().await;
    let mut value = serde_json::to_value(session_io).unwrap();
    value["input"][0]["target"] = json!(target);
    let io = serde_json::from_value(value).unwrap();
    assert!(
        Executor::connect(io, session_connection(&server))
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::cancelled(200, "cancellation was requested")]
#[case::cancel_failed(503, "cancellation failed")]
#[tokio::test]
async fn session_timeout_cancels_only_its_accepted_job(
    session_io: AgentIo,
    polled: AgentIo,
    case: Case,
    #[case] status: u16,
    #[case] message: &str,
) {
    let server = MockServer::start().await;
    session_server(&server).await;
    let io = AgentIo {
        completion: polled.completion,
        output: polled.output,
        ..session_io
    };
    Mock::given(method("POST"))
        .and(path("/api/runs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"accepted-job"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jobs/accepted-job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"running"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/runs/accepted-job/cancel"))
        .and(header("Origin", server.uri().as_str()))
        .and(header("Cookie", "workspace_session=private-cookie"))
        .and(header("X-CSRF-Token", "private-csrf"))
        .respond_with(ResponseTemplate::new(status).set_body_json(json!({"status":"cancelled"})))
        .expect(1)
        .mount(&server)
        .await;
    let executor = Executor::connect(io, session_connection(&server))
        .await
        .unwrap();
    let result = executor
        .run(case, "id".into(), Duration::from_millis(80))
        .await;
    let error = result.error.unwrap();
    assert_eq!(error.r#type, "AgentTimeoutError");
    assert!(error.message.contains(message));
    assert!(result.output.is_none());
}
