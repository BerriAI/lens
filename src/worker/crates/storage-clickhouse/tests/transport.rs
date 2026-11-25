use std::collections::BTreeMap;

use litellm_http::Client;
use litellm_storage_clickhouse::{Connection, Error, Query, execute_read, insert_encoded_rows};
use rstest::{fixture, rstest};
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

#[fixture]
async fn clickhouse() -> ContainerAsync<ClickHouse> {
    ClickHouse::default()
        .with_tag(
            "26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e",
        )
        .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
        .start()
        .await
        .expect("ClickHouse")
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct BoundValues {
    text: String,
    strings: Vec<String>,
}

struct Echo;

impl Query for Echo {
    type Params = BoundValues;
    type Row = BoundValues;
    const SQL: &'static str = "SELECT {text:String} AS text, {strings:Array(String)} AS strings";
}

#[rstest]
#[case::escaped_values("雪\tline\nbreak\r\0\\'&=+".into(), vec!["a'b".into(), "雪\\\n&=+".into()])]
#[case::large_scalar("value-".repeat(20_000), Vec::new())]
#[case::large_id_list(
    "profile lookup".into(),
    (0..160).map(|index| format!("response-{index}-{}", "x".repeat(512))).collect()
)]
#[tokio::test]
async fn bound_values_round_trip_without_url_size_limits(
    #[future(awt)] clickhouse: ContainerAsync<ClickHouse>,
    #[case] text: String,
    #[case] strings: Vec<String>,
) {
    let url = format!(
        "http://{}:{}",
        clickhouse.get_host().await.unwrap(),
        clickhouse.get_host_port_ipv4(8123).await.unwrap()
    );
    let values = BoundValues { text, strings };
    let rows = litellm_storage_clickhouse::fetch::<Echo>(
        &Client::no_redirect_for_test(),
        &Connection::parse(&url).unwrap(),
        &values,
    )
    .await
    .expect("bound values reach ClickHouse");

    assert_eq!(rows, vec![values]);
}

#[rstest]
#[case::invalid_database("db; DROP DATABASE default", "spend_logs", true)]
#[case::invalid_table("litellm", "spend_logs; DROP TABLE otel_traces", false)]
#[tokio::test]
async fn insert_rejects_invalid_identifiers(
    #[case] database: &str,
    #[case] table: &str,
    #[case] invalid_database: bool,
) {
    let client = Client::no_redirect_for_test();
    let connection = Connection::writer("http://localhost:8123").expect("valid URL");
    let result = insert_encoded_rows(&client, &connection, database, table, "token", "{}").await;

    assert!(matches!(&result, Err(Error::InvalidSchema)) == invalid_database);
    assert!(matches!(&result, Err(Error::InvalidTable)) != invalid_database);
}

#[rstest]
#[tokio::test]
async fn read_rejects_empty_sql() {
    let client = Client::no_redirect_for_test();
    let connection = Connection::reader("http://localhost:8123", "litellm").expect("valid URL");

    assert!(matches!(
        execute_read(&client, &connection, "  ", &BTreeMap::new()).await,
        Err(Error::EmptySql)
    ));
}

#[derive(serde::Serialize)]
struct QueryParams {
    signed: i64,
    unsigned: u64,
    float: f64,
    text: String,
    strings: Vec<String>,
}

#[derive(Debug, serde::Deserialize, PartialEq)]
struct QueryRow {
    answer: String,
}

struct TypedQuery;

impl litellm_storage_clickhouse::Query for TypedQuery {
    type Params = QueryParams;
    type Row = QueryRow;
    const SQL: &'static str = "SELECT typed_parameters";
}

#[rstest]
#[case::valid(
    r#"{"meta":[],"data":[{"answer":"ok"}],"rows":1,"statistics":{"elapsed":0.1}}"#,
    true
)]
#[case::wrong_type(r#"{"data":[{"answer":1}]}"#, false)]
#[case::missing_column(r#"{"data":[{}]}"#, false)]
#[case::exception(r#"{"data":[],"exception":"failed"}"#, false)]
#[tokio::test]
async fn typed_fetch_encodes_parameters_and_validates_rows(
    #[case] body: &str,
    #[case] valid: bool,
) {
    use litellm_storage_clickhouse::{fetch, fetch_json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, query_param},
    };

    let server = MockServer::start().await;
    Mock::given(body_string_contains(TypedQuery::SQL))
        .and(body_string_contains(format!(
            "name=\"param_signed\"\r\n\r\n{}\r\n",
            i64::MIN
        )))
        .and(body_string_contains(format!(
            "name=\"param_unsigned\"\r\n\r\n{}\r\n",
            u64::MAX
        )))
        .and(body_string_contains("name=\"param_float\"\r\n\r\n12.5\r\n"))
        .and(body_string_contains(
            "name=\"param_text\"\r\n\r\nline\\nbreak\r\n",
        ))
        .and(body_string_contains(
            "name=\"param_strings\"\r\n\r\n['a\\'b','雪']\r\n",
        ))
        .and(query_param("readonly", "1"))
        .and(query_param("max_result_rows", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(2)
        .mount(&server)
        .await;
    let client = Client::no_redirect_for_test();
    let connection = Connection::parse(&server.uri()).unwrap();
    let params = QueryParams {
        signed: i64::MIN,
        unsigned: u64::MAX,
        float: 12.5,
        text: "line\nbreak".into(),
        strings: vec!["a'b".into(), "雪".into()],
    };
    let rows = fetch::<TypedQuery>(&client, &connection, &params).await;
    let envelope = fetch_json::<TypedQuery>(&client, &connection, &params).await;
    if valid {
        assert_eq!(
            rows.unwrap(),
            vec![QueryRow {
                answer: "ok".into()
            }]
        );
        assert_eq!(envelope.unwrap(), body);
    } else {
        assert!(matches!(rows, Err(Error::InvalidResponse)));
        assert!(matches!(envelope, Err(Error::InvalidResponse)));
    }
}

#[rstest]
#[case::result_limit("396", true)]
#[case::memory_limit("241", false)]
#[case::timeout("159", false)]
#[case::unknown("", false)]
#[tokio::test]
async fn server_result_limits_allow_smaller_pages_without_retrying_other_failures(
    #[case] code: &str,
    #[case] result_limit: bool,
) {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).insert_header("X-ClickHouse-Exception-Code", code))
        .expect(1)
        .mount(&server)
        .await;
    let connection = Connection::parse(&server.uri()).unwrap();
    let error = execute_read(
        &Client::no_redirect_for_test(),
        &connection,
        "SELECT 1",
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    if result_limit {
        assert!(matches!(error, Error::ResponseTooLarge));
    } else {
        assert!(matches!(error, Error::QueryFailed(500)));
    }
}

#[rstest]
#[case::within_limit(0, true)]
#[case::over_limit(1, false)]
#[tokio::test]
async fn reads_enforce_response_size_before_json_decoding(
    #[case] excess: usize,
    #[case] allowed: bool,
) {
    use litellm_storage_clickhouse::{ReadLimits, execute_read_with_limits};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    let body = r#"{"data":[{"answer":"ok"}]}"#;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    let result = execute_read_with_limits(
        &Client::no_redirect_for_test(),
        &Connection::parse(&server.uri()).unwrap(),
        "SELECT 'ok' AS answer",
        &BTreeMap::new(),
        ReadLimits {
            response_bytes: body.len() - excess,
            ..litellm_storage_clickhouse::READ_LIMITS
        },
    )
    .await;
    if allowed {
        assert_eq!(result.unwrap(), body);
    } else {
        assert!(matches!(result, Err(Error::ResponseTooLarge)));
    }
}

#[rstest]
#[tokio::test]
async fn default_reads_reject_oversized_responses() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    let server = MockServer::start().await;
    let body = " ".repeat(litellm_storage_clickhouse::READ_LIMITS.response_bytes + 1);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    let result = execute_read(
        &Client::no_redirect_for_test(),
        &Connection::parse(&server.uri()).unwrap(),
        "SELECT payload FROM otel_traces",
        &BTreeMap::new(),
    )
    .await;
    assert!(matches!(result, Err(Error::ResponseTooLarge)));
}

#[test]
fn insert_timeout_environment_controls_transport() {
    for value in ["1", "3", "0", "invalid"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "insert_timeout_environment_child"])
            .env("LITELLM_TEST_INSERT_TIMEOUT", value)
            .env("CLICKHOUSE_INSERT_TIMEOUT_SECONDS", value)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
}

#[tokio::test]
async fn insert_timeout_environment_child() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let Ok(value) = std::env::var("LITELLM_TEST_INSERT_TIMEOUT") else {
        return;
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(1500)))
        .mount(&server)
        .await;
    let result = insert_encoded_rows(
        &Client::no_redirect_for_test(),
        &Connection::parse(&server.uri()).unwrap(),
        "traces",
        "otel_traces",
        "token",
        "{}",
    )
    .await;
    match value.as_str() {
        "1" => assert!(matches!(result, Err(Error::Transport))),
        "3" => assert!(result.is_ok()),
        _ => assert!(matches!(
            result,
            Err(Error::InvalidLimit("CLICKHOUSE_INSERT_TIMEOUT_SECONDS"))
        )),
    }
}
