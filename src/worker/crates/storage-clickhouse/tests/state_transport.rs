use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, Error,
    state::{ClickHouseState, Head, Snapshot},
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, query_param},
};

struct Service {
    server: MockServer,
    store: ClickHouseState,
}

#[fixture]
async fn service() -> Service {
    let server = MockServer::start().await;
    let store = ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&server.uri()).unwrap(),
    );
    Service { server, store }
}

fn select(table: &'static str) -> impl wiremock::Match {
    move |request: &Request| {
        request.url.query_pairs().any(|(name, query)| {
            name == "query" && query.starts_with("SELECT") && query.contains(table)
        })
    }
}

fn head(key: &str, revision: u64, value: Value) -> Head {
    Head {
        key: key.into(),
        revision,
        digest: format!("{:x}", Sha256::digest(value.to_string().as_bytes())),
    }
}

fn blob(head: &Head, value: Value) -> String {
    json!({"key": head.key, "revision": head.revision, "digest": head.digest, "data": value.to_string()}).to_string()
}

#[rstest]
#[case::application_conflict("Code: 395. LENS_STATE_CONFLICT", Error::StateConflict)]
#[case::other_throw("Code: 395. other error", Error::StateFailed { status: 500, code: Some(395) })]
#[case::keeper_conflict("Code: 999. Bad version", Error::StateConflict)]
#[case::already_exists("Code: 999. (Node exists)", Error::StateExists)]
#[case::keeper_outage("Code: 999. connection lost", Error::StateFailed { status: 500, code: Some(999) })]
#[case::unknown("upstream unavailable", Error::StateFailed { status: 500, code: None })]
#[tokio::test]
async fn failures_keep_conflicts_distinct_from_outages(
    #[future(awt)] service: Service,
    #[case] body: &str,
    #[case] expected: Error,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(body))
        .mount(&service.server)
        .await;
    let error = service.store.read("key").await.unwrap_err();
    assert_eq!(error.to_string(), expected.to_string());
}

#[rstest]
#[case::header("999", "Bad version", Error::StateConflict)]
#[case::unsuccessful_query("241", "Memory limit exceeded", Error::StateFailed { status: 200, code: Some(241) })]
#[tokio::test]
async fn exception_headers_are_not_success_even_after_http_200(
    #[future(awt)] service: Service,
    #[case] code: &str,
    #[case] body: &str,
    #[case] expected: Error,
) {
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("X-ClickHouse-Exception-Code", code)
                .set_body_string(body),
        )
        .mount(&service.server)
        .await;
    assert_eq!(
        service.store.heads(&["x"]).await.unwrap_err().to_string(),
        expected.to_string()
    );
}

#[rstest]
#[case::empty_digest(json!({"key": "x", "revision": 1, "digest": ""}))]
#[case::nonempty_initial(json!({"key": "x", "revision": 0, "digest": "a".repeat(64)}))]
#[case::bad_digit(json!({"key": "x", "revision": 1, "digest": "g".repeat(64)}))]
#[case::uppercase(json!({"key": "x", "revision": 1, "digest": "A".repeat(64)}))]
#[case::short(json!({"key": "x", "revision": 1, "digest": "a".repeat(63)}))]
#[case::unexpected_key(json!({"key": "other", "revision": 0, "digest": ""}))]
#[case::missing_field(json!({"key": "x", "revision": 0}))]
#[tokio::test]
async fn malformed_heads_cannot_be_treated_as_missing_records(
    #[future(awt)] service: Service,
    #[case] row: Value,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row.to_string()))
        .mount(&service.server)
        .await;
    assert!(matches!(
        service.store.heads(&["x"]).await,
        Err(Error::InvalidResponse)
    ));
}

#[rstest]
#[case::malformed("broken")]
#[case::duplicate(
    "{\"key\":\"x\",\"revision\":0,\"digest\":\"\"}\n{\"key\":\"x\",\"revision\":0,\"digest\":\"\"}"
)]
#[tokio::test]
async fn malformed_or_duplicate_rows_are_rejected(
    #[future(awt)] service: Service,
    #[case] body: &str,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&service.server)
        .await;
    assert!(matches!(
        service.store.heads(&["x"]).await,
        Err(Error::InvalidResponse)
    ));
}

#[rstest]
#[case::number(json!(2))]
#[case::clickhouse_quoted(json!("2"))]
#[tokio::test]
async fn heads_keep_requested_order_and_missing_keys(
    #[future(awt)] service: Service,
    #[case] revision: Value,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            json!({"key": "b", "revision": revision, "digest": "a".repeat(64)}).to_string(),
        ))
        .mount(&service.server)
        .await;
    assert_eq!(
        service.store.heads(&["b", "a", "b"]).await.unwrap(),
        vec![
            Head {
                key: "b".into(),
                revision: 2,
                digest: "a".repeat(64)
            },
            Head::empty("a"),
            Head {
                key: "b".into(),
                revision: 2,
                digest: "a".repeat(64)
            }
        ]
    );
}

#[rstest]
#[case::corrupt_data("x", 1, "different")]
#[case::wrong_revision("x", 2, "original")]
#[case::wrong_key("other", 1, "original")]
#[tokio::test]
async fn pinned_reads_validate_the_payload_and_identity(
    #[future(awt)] service: Service,
    #[case] key: &str,
    #[case] revision: u64,
    #[case] value: &str,
) {
    let reference = head("x", 1, json!("original"));
    let row = json!({"key": key, "revision": revision, "digest": reference.digest, "data": json!(value).to_string()});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(row.to_string()))
        .mount(&service.server)
        .await;
    assert!(matches!(
        service.store.resolve(&[reference]).await,
        Err(Error::InvalidResponse)
    ));
}

#[rstest]
#[tokio::test]
async fn multi_read_retries_when_heads_move_during_payload_fetch(#[future(awt)] service: Service) {
    let first = head("a", 1, json!("before"));
    let second = head("a", 2, json!("after"));
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&first))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&second))
        .expect(3)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(blob(&first, json!("before"))))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(blob(&second, json!("after"))))
        .expect(1)
        .mount(&service.server)
        .await;
    assert_eq!(
        service.store.read_many(&["a", "missing"]).await.unwrap(),
        vec![
            Snapshot {
                head: second,
                value: json!("after")
            },
            Snapshot::empty("missing")
        ]
    );
}

#[rstest]
#[tokio::test]
async fn unchanged_reads_reuse_verified_blobs_across_clones(#[future(awt)] service: Service) {
    let value = json!({"settings": {"enabled": true}, "history": [1, 2, 3]});
    let reference = head("x", 1, value.clone());
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&reference))
        .expect(2)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(blob(&reference, value.clone())))
        .expect(1)
        .mount(&service.server)
        .await;

    let first = service.store.read("x").await.unwrap();
    let second = service.store.clone().read("x").await.unwrap();
    assert_eq!(first.value, value);
    assert_eq!(second, first);
}

#[rstest]
#[case::changed(json!("new"))]
#[case::unchanged(json!("old"))]
#[tokio::test]
async fn cached_historical_reads_recheck_persisted_revisions(
    #[future(awt)] service: Service,
    #[case] current_value: Value,
) {
    let original = head("x", 1, json!("old"));
    let current = head("x", 3, current_value.clone());
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&original))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&current))
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(blob(&original, json!("old"))))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(blob(&current, current_value.clone())),
        )
        .with_priority(3)
        .mount(&service.server)
        .await;

    let cached = service.store.read("x").await.unwrap();
    assert_eq!(cached.head, original);
    let resolved = service.store.resolve(&[cached.head]).await;
    if current_value == json!("old") {
        assert_eq!(
            resolved.unwrap(),
            vec![Snapshot {
                head: current,
                value: current_value,
            }]
        );
    } else {
        assert!(matches!(resolved, Err(Error::StateUnavailable)));
    }
}

#[rstest]
#[case::updated(json!({"enabled": false}))]
#[case::revoked(Value::Null)]
#[tokio::test]
async fn cached_values_never_hide_a_new_head(
    #[future(awt)] service: Service,
    #[case] updated: Value,
) {
    let first = head("x", 1, json!({"enabled": true}));
    let next = head("x", 2, updated.clone());
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&first))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&next))
        .expect(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(blob(&first, json!({"enabled": true}))),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(200).set_body_string(blob(&next, updated.clone())))
        .expect(1)
        .mount(&service.server)
        .await;

    assert_eq!(service.store.read("x").await.unwrap().head, first);
    assert_eq!(
        service.store.read("x").await.unwrap(),
        Snapshot {
            head: next,
            value: updated
        }
    );
}

#[rstest]
#[tokio::test]
async fn missing_records_do_not_issue_empty_blob_queries(#[future(awt)] service: Service) {
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200))
        .expect(2)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&service.server)
        .await;

    assert_eq!(
        service.store.read("missing").await.unwrap(),
        Snapshot::empty("missing")
    );
    assert_eq!(
        service.store.read("missing").await.unwrap(),
        Snapshot::empty("missing")
    );
}

#[rstest]
#[tokio::test]
async fn corrupt_blobs_do_not_poison_later_reads(#[future(awt)] service: Service) {
    let reference = head("x", 1, json!("correct"));
    Mock::given(select("lens_state_heads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&reference))
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(blob(&reference, json!("corrupt"))),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&service.server)
        .await;
    Mock::given(select("lens_state_blobs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(blob(&reference, json!("correct"))),
        )
        .expect(1)
        .mount(&service.server)
        .await;

    assert!(matches!(
        service.store.read("x").await,
        Err(Error::InvalidResponse)
    ));
    assert_eq!(
        service.store.read("x").await.unwrap().value,
        json!("correct")
    );
    assert_eq!(
        service.store.read("x").await.unwrap().value,
        json!("correct")
    );
}

#[rstest]
#[tokio::test]
async fn unsafe_connection_settings_cannot_relax_state_consistency(
    #[future(awt)] service: Service,
) {
    let url = format!(
        "{}/?database=example&keeper_map_strict_mode=0&insert_keeper_max_retries=10&async_insert=1&wait_end_of_query=0&send_progress_in_http_headers=1&param_keys=wrong",
        service.server.uri()
    );
    let store = ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&url).unwrap(),
    );
    Mock::given(method("POST"))
        .and(query_param("keeper_map_strict_mode", "1"))
        .and(query_param("insert_keeper_max_retries", "0"))
        .and(query_param("async_insert", "0"))
        .and(query_param("wait_end_of_query", "1"))
        .and(query_param("send_progress_in_http_headers", "0"))
        .and(query_param("database", "example"))
        .and(query_param("param_keys", "[\"key\"]"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&service.server)
        .await;
    assert_eq!(
        store.heads(&["key"]).await.unwrap(),
        vec![Head::empty("key")]
    );
    let requests = service.server.received_requests().await.unwrap();
    let settings: Vec<_> = requests[0]
        .url
        .query_pairs()
        .map(|(name, _)| name.into_owned())
        .collect();
    let unique: std::collections::BTreeSet<_> = settings.iter().collect();
    assert_eq!(unique.len(), settings.len());
}

#[rstest]
#[tokio::test]
async fn transport_failure_is_not_a_conflict() {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    drop(socket);
    let store = ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&url).unwrap(),
    );
    assert!(matches!(store.read("x").await, Err(Error::Transport)));
    assert!(store.heads(&[]).await.unwrap().is_empty());
    assert!(store.read_many(&[]).await.unwrap().is_empty());
    assert!(store.resolve(&[]).await.unwrap().is_empty());
}

#[rstest]
#[case::duplicate(true)]
#[case::invalid_json(false)]
#[tokio::test]
async fn duplicate_or_undecodable_payloads_fail_closed(
    #[future(awt)] service: Service,
    #[case] duplicate: bool,
) {
    let reference = head("x", 1, json!("original"));
    let body = if duplicate {
        let row = blob(&reference, json!("original"));
        format!("{row}\n{row}")
    } else {
        let data = "not json";
        json!({"key": "x", "revision": 1, "digest": format!("{:x}", Sha256::digest(data.as_bytes())), "data": data}).to_string()
    };
    let reference = if duplicate {
        reference
    } else {
        serde_json::from_str::<Head>(&body).unwrap()
    };
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&service.server)
        .await;
    assert!(matches!(
        service.store.resolve(&[reference]).await,
        Err(Error::InvalidResponse)
    ));
}

#[rstest]
#[case::missing_digest(Head { key: "x".into(), revision: 1, digest: String::new() })]
#[case::initial_has_digest(Head { key: "x".into(), revision: 0, digest: "a".repeat(64) })]
#[tokio::test]
async fn invalid_references_are_rejected_without_storage(
    #[future(awt)] service: Service,
    #[case] reference: Head,
) {
    assert!(matches!(
        service
            .store
            .resolve(std::slice::from_ref(&reference))
            .await,
        Err(Error::InvalidState)
    ));
    let previous = Snapshot {
        head: reference,
        value: Value::Null,
    };
    assert!(matches!(
        service
            .store
            .prepare(vec![litellm_storage_clickhouse::state::Change {
                previous,
                value: json!(true)
            }])
            .await,
        Err(Error::InvalidState)
    ));
}

#[rstest]
#[case::head_insert("INSERT INTO lens_state_heads FORMAT JSONEachRow")]
#[case::payload_insert("INSERT INTO lens_state_blobs FORMAT JSONEachRow")]
#[tokio::test]
async fn failed_preparation_cannot_publish(
    #[future(awt)] service: Service,
    #[case] failed_query: &str,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&service.server)
        .await;
    Mock::given(query_param("query", failed_query))
        .respond_with(ResponseTemplate::new(503))
        .with_priority(1)
        .expect(1)
        .mount(&service.server)
        .await;
    let change = litellm_storage_clickhouse::state::Change {
        previous: Snapshot::empty("x"),
        value: json!(true),
    };
    assert!(matches!(
        service.store.commit(vec![change]).await,
        Err(Error::StateFailed { status: 503, .. })
    ));
    let requests = service.server.received_requests().await.unwrap();
    let publications: Vec<_> = requests
        .iter()
        .filter(|request| {
            request
                .url
                .query_pairs()
                .any(|(key, value)| key == "query" && value.starts_with("ALTER"))
        })
        .collect();
    assert!(publications.is_empty());
}

#[rstest]
#[tokio::test]
async fn large_state_responses_reach_json_decoding(#[future(awt)] service: Service) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(" ".repeat(64 * 1024 * 1024 + 1)))
        .mount(&service.server)
        .await;
    let result = service.store.heads(&["x"]).await;
    assert!(matches!(result, Err(Error::InvalidResponse)));
}

#[rstest]
#[tokio::test]
async fn large_state_commits_reach_storage(#[future(awt)] service: Service) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&service.server)
        .await;
    let overhead = json!({"key": "x", "revision": 1, "digest": "a".repeat(64), "data": "\"\""})
        .to_string()
        .len();
    let change = litellm_storage_clickhouse::state::Change {
        previous: Snapshot::empty("x"),
        value: json!("x".repeat(64 * 1024 * 1024 + 1 - overhead)),
    };
    let result = service.store.prepare(vec![change]).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(!service.server.received_requests().await.unwrap().is_empty());
}
