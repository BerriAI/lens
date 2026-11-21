#[allow(
    dead_code,
    reason = "The shared HTTP fixture also supports activity-specific cases"
)]
mod activity {
    pub mod support;
}

use activity::support::{ADMIN, Database, SECRET, database, delegated};
use lens_auth::{Authentication, Settings};
use lens_contract::{
    auth::{Identity, Role},
    signals::{SignalAttempt, SignalAttemptStatus, SignalConfig, SignalEvidence, SignalOverview},
    worker::{Execution, ExecutionSource},
};
use lens_signals::SignalRepository;
use litellm_storage_clickhouse::{sessions::Sessions, signals::Signals};
use rstest::rstest;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

struct Server {
    url: url::Url,
    client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(database: &Database) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let authentication = Arc::new(Authentication {
        settings: Settings::new(ADMIN, Some(SECRET.into()), url.as_str()).unwrap(),
        sessions: Sessions(database.sessions.clone()),
    });
    let router = lens_server::signals::with_catalog(
        authentication,
        Signals(database.sessions.clone()),
        [
            ("evaluation", "evaluation"),
            ("chat-judge", "chat"),
            ("embeddings", "embedding"),
        ]
        .map(
            |(model_group, mode)| lens_contract::activity::AnalysisModelInfo {
                model_group: model_group.into(),
                providers: Vec::new(),
                mode: mode.into(),
                supported_openai_params: None,
            },
        )
        .to_vec(),
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server {
        url,
        client: reqwest::Client::new(),
        task,
    }
}

fn execution(id: &str, reference: &str) -> Execution {
    Execution {
        id: id.into(),
        source: ExecutionSource::Traces,
        name: id.into(),
        trace_id: id.into(),
        trace_ref: reference.into(),
        team_id: "alpha".into(),
        start_time: String::new(),
        span_count: 2,
        root_seen: true,
        service: String::new(),
        metadata: Vec::new(),
    }
}

async fn overview(server: &Server) -> SignalOverview {
    server
        .client
        .get(server.url.join("/lens/signals/overview").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn store_result(
    repository: &Signals,
    config: &SignalConfig,
    reference: &str,
    at: chrono::DateTime<chrono::Utc>,
    attempt: &SignalAttempt,
) {
    let trace = execution("same", reference);
    let until = at + chrono::TimeDelta::minutes(5);
    assert!(repository.claim(&trace, config, until, at).await.unwrap());
    repository
        .store(&trace, config, until, at, attempt)
        .await
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn overview_filters_current_results_and_never_returns_trace_quotes(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    let config: SignalConfig = serde_json::from_value(json!({
        "model": "evaluation", "threshold": 0.7, "signals": [
            {"id": "example", "name": "Example", "question": "Did an incident occur?"}
        ]
    }))
    .unwrap();
    let repository = Signals(database.sessions.clone());
    repository.save_config(&config).await.unwrap();
    let now = chrono::Utc::now();
    let matches = SignalAttempt {
        status: SignalAttemptStatus::Classified,
        partial: true,
        model: config.model.clone(),
        error: String::new(),
        scores: BTreeMap::from([("example".into(), 0.8)]),
        evidence: BTreeMap::from([(
            "example".into(),
            SignalEvidence {
                span_id: "sample-span".into(),
                quote: "Synthetic trace evidence".into(),
            },
        )]),
        failure: None,
    };
    let clear = SignalAttempt {
        scores: BTreeMap::from([("example".into(), 0.2)]),
        partial: false,
        evidence: BTreeMap::new(),
        ..matches.clone()
    };
    let failed = SignalAttempt {
        status: SignalAttemptStatus::Failed,
        scores: BTreeMap::new(),
        ..clear.clone()
    };

    store_result(&repository, &config, "match", now, &matches).await;
    store_result(&repository, &config, "clear", now, &clear).await;
    store_result(&repository, &config, "failed", now, &failed).await;
    store_result(
        &repository,
        &config,
        "old",
        now - chrono::TimeDelta::hours(25),
        &matches,
    )
    .await;
    let other_config = SignalConfig {
        model: "chat-judge".into(),
        ..config.clone()
    };
    store_result(&repository, &other_config, "other-model", now, &matches).await;
    store_result(&repository, &config, "superseded", now, &matches).await;
    store_result(&repository, &other_config, "superseded", now, &clear).await;

    let result = overview(&server).await;
    assert_eq!(
        (
            result.evaluated,
            result.failed,
            result.partial,
            result.truncated
        ),
        (2, 1, 1, false)
    );
    let [matched] = result.matches.as_slice() else {
        panic!("expected one matched trace")
    };
    assert_eq!(
        (matched.trace_id.as_str(), matched.trace_ref.as_str()),
        ("same", "match")
    );
    assert_eq!(matched.flags[0].score, 0.8);
    assert_eq!(matched.flags[0].evidence, None);

    repository
        .save_config(&SignalConfig {
            threshold: 0.9,
            ..config
        })
        .await
        .unwrap();
    let stricter = overview(&server).await;
    assert_eq!(stricter.evaluated, 2);
    assert!(stricter.matches.is_empty());
}

#[rstest]
#[tokio::test]
async fn recorded_signals_contracts_replay_against_real_http_and_clickhouse(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parity/fixtures/signals");
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, 19);
}

#[rstest]
#[case::read("GET", "/lens/signals", Role::ProxyAdminViewer, json!(null), 200)]
#[case::viewer_write("PUT", "/lens/signals", Role::ProxyAdminViewer, json!({}), 403)]
#[case::team_read("GET", "/lens/signals", Role::InternalUser, json!(null), 403)]
#[case::team_traces("POST", "/lens/traces/signals", Role::InternalUser, json!({"traces":[{"trace_id":"trace"}]}), 403)]
#[case::viewer_overview("GET", "/lens/signals/overview", Role::ProxyAdminViewer, json!(null), 200)]
#[case::team_overview("GET", "/lens/signals/overview", Role::InternalUser, json!(null), 403)]
#[case::administrator_write("PUT", "/lens/signals", Role::ProxyAdmin, json!({}), 200)]
#[tokio::test]
async fn signal_routes_preserve_read_and_write_permissions(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] role: Role,
    #[case] body: Value,
    #[case] expected: u16,
) {
    let server = serve(&database).await;
    let token = delegated(Identity {
        user_role: role,
        user_id: Some("caller".into()),
        team_id: Some("alpha".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
}

#[rstest]
#[tokio::test]
async fn stored_classifications_keep_trace_identity_order_and_live_display_settings(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    let config = json!({"model":"evaluation","threshold":0.5,"signals":[
        {"id":"first","name":"First","question":"First question?"},
        {"id":"second","name":"Second","question":"Second question?"}
    ]});
    let response = server
        .client
        .put(server.url.join("/lens/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&config)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let config: SignalConfig = response.json().await.unwrap();
    let repository = Signals(database.sessions.clone());
    let now = chrono::Utc::now();
    let until = now + chrono::TimeDelta::minutes(5);
    let classified = execution("same", "classified");
    assert!(
        repository
            .claim(&classified, &config, until, now)
            .await
            .unwrap()
    );
    repository
        .store(
            &classified,
            &config,
            until,
            now,
            &SignalAttempt {
                partial: false,
                status: SignalAttemptStatus::Classified,
                model: "actual-model".into(),
                error: String::new(),
                scores: BTreeMap::from([("first".into(), 0.5), ("second".into(), 0.8)]),
                evidence: BTreeMap::new(),
                failure: None,
            },
        )
        .await
        .unwrap();
    assert!(
        repository
            .claim(&execution("same", "pending"), &config, until, now)
            .await
            .unwrap()
    );
    let traces = json!({"traces":[{"trace_id":"same","trace_ref":"classified"},{"trace_id":"same","trace_ref":"pending"},{"trace_id":"same"},{"trace_id":"same","trace_ref":"classified"}]});
    let response = server
        .client
        .post(server.url.join("/lens/traces/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&traces)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let rows: Value = response.json().await.unwrap();
    assert_eq!(
        rows[0]["flags"],
        json!([{"signal_id":"second","name":"Second","score":0.8},{"signal_id":"first","name":"First","score":0.5}])
    );
    assert_eq!(rows[0]["model"], "actual-model");
    assert_eq!(rows[1]["status"], "pending");
    assert_eq!(rows[2]["status"], "unclassified");
    assert_eq!(rows[3], rows[0]);
    let updated = SignalConfig {
        threshold: 0.7,
        signals: config
            .signals
            .iter()
            .map(|signal| lens_contract::signals::Signal {
                name: format!("New {}", signal.name),
                ..signal.clone()
            })
            .collect(),
        ..config.clone()
    };
    repository.save_config(&updated).await.unwrap();
    drop(server);
    let server = serve(&database).await;
    let rows: Value = server
        .client
        .post(server.url.join("/lens/traces/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&traces)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rows[0]["status"], "classified");
    assert_eq!(
        rows[0]["flags"],
        json!([{"signal_id":"second","name":"New Second","score":0.8}])
    );
    repository
        .save_config(&SignalConfig {
            model: String::new(),
            ..updated
        })
        .await
        .unwrap();
    let rows: Value = server
        .client
        .post(server.url.join("/lens/traces/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&traces)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rows[0]["status"], "unclassified");
    assert_eq!(rows[0]["flags"], json!([]));
}

#[rstest]
#[case::unknown("not-configured")]
#[case::not_a_judge("embeddings")]
#[tokio::test]
async fn a_model_that_cannot_judge_cannot_be_saved_with_empty_signals(
    #[future(awt)] database: Database,
    #[case] model: &str,
) {
    let server = serve(&database).await;
    let response = server
        .client
        .put(server.url.join("/lens/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"model":model,"signals":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        Signals(database.sessions.clone())
            .get_config()
            .await
            .unwrap(),
        SignalConfig::default()
    );
}

#[rstest]
#[tokio::test]
async fn a_chat_model_can_be_saved_as_the_signal_judge(#[future(awt)] database: Database) {
    let server = serve(&database).await;
    let response = server
        .client
        .put(server.url.join("/lens/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"model":"chat-judge","signals":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        Signals(database.sessions.clone())
            .get_config()
            .await
            .unwrap()
            .model,
        "chat-judge"
    );
}

#[rstest]
#[tokio::test]
async fn invalid_stored_scores_do_not_become_unclassified_successes(
    #[future(awt)] database: Database,
) {
    let key = "trace-signal/%5B%22broken%22%2C%22%22%5D";
    let config = SignalConfig::default();
    database.sessions.update(key, |_| json!({
        "trace_id":"broken","trace_ref":"","config_key":lens_signals::config_key(&config),"span_count":1,
        "claimed_until":null,"classified_at":null,"data":{"status":"classified","scores":{"user_frustration":2}}
    }), 4).await.unwrap();
    let server = serve(&database).await;
    let response = server
        .client
        .post(server.url.join("/lens/traces/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"traces":[{"trace_id":"broken"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Lens signal storage is unavailable"})
    );
}

#[rstest::fixture]
fn large_config() -> Value {
    let signals:Vec<_> = (0..21).map(|id|json!({"id":format!("s{id:063}"),"name":"n".repeat(61),"question":"q".repeat(501)})).collect();
    json!({"model":"evaluation","threshold":0.95,"signals":signals})
}

#[rstest]
#[tokio::test]
async fn large_signal_configuration_survives_a_restart(
    #[future(awt)] database: Database,
    large_config: Value,
) {
    let server = serve(&database).await;
    let response = server
        .client
        .put(server.url.join("/lens/signals").unwrap())
        .bearer_auth(ADMIN)
        .json(&large_config)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), large_config);
    drop(server);
    let restarted = serve(&database).await;
    let response = restarted
        .client
        .get(restarted.url.join("/lens/signals").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), large_config);
}
