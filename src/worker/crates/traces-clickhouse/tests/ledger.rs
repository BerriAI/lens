use std::{collections::BTreeMap, sync::Arc};

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region, retry::RetryConfig};
use lens_payloads::{Field, ObjectKey, Store, sha256_hex};
use litellm_storage_clickhouse::fetch;
use litellm_traces::query::named as contracts;
use litellm_traces_clickhouse::{
    Connection, InsertTable, PayloadRefRow, Payloads, Unavailable, insert_ledger,
    insert_payload_ref, insert_rows, ledger_row, query::lens, query::named::SpendByResponseIds,
    request_page, request_text,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[path = "queries/support.rs"]
mod fixtures;
mod support;

use fixtures::{DATABASE, SeededDatabase, migrated_database};
use support::TestResult;

const START_MS: i64 = 1_790_000_000_000;
const BUCKET: &str = "lens";

#[fixture]
fn record() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("request_id".into(), json!("billing-record")),
        ("litellm_call_id".into(), json!("gateway-call")),
        ("response_id".into(), json!("provider-response")),
        ("team_id".into(), json!("team-a")),
        ("api_key".into(), json!("key-a")),
        ("user".into(), json!("user-a")),
        ("model".into(), json!("model-a")),
        ("start_time".into(), json!(START_MS)),
        ("end_time".into(), json!(START_MS + 1)),
        ("spend".into(), json!(0.125)),
    ])
}

fn access(team: &str) -> contracts::ReadAccessParams {
    contracts::ReadAccessParams {
        all_teams: false,
        user_id: String::new(),
        team_ids: vec![team.into()],
    }
}

async fn deliver(
    fixture: &SeededDatabase,
    events: &[(&str, &BTreeMap<String, Value>)],
) -> TestResult {
    let rows = events
        .iter()
        .map(|(event_id, record)| ledger_row(event_id, record, 1024 * 1024))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|rejection| format!("{rejection:?}"))?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_ledger(&fixture.database.client, &writer, DATABASE, &rows).await?;
    Ok(())
}

async fn lookup(
    fixture: &SeededDatabase,
    team: &str,
    id: &str,
) -> TestResult<Vec<contracts::SpendByResponseIdsRow>> {
    let params = contracts::SpendByResponseIdsParams {
        access: access(team),
        response_ids: vec![id.into()],
        provider_request_ids: vec![id.into()],
        request_ids: vec![id.into()],
        trace_ids: vec![id.into()],
        start_ms: START_MS - 1,
        end_ms: START_MS + 2,
    };
    let reader = Connection::writer(&fixture.database.url)?;
    let mut rows: Vec<_> =
        fetch::<SpendByResponseIds>(&fixture.database.client, &reader, &params.into())
            .await?
            .into_iter()
            .map(|row| row.0)
            .collect();
    rows.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    Ok(rows)
}

async fn scalar(fixture: &SeededDatabase, sql: &str) -> TestResult<String> {
    Ok(fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(sql.to_owned())
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?
        .trim()
        .to_owned())
}

#[rstest]
#[case::distinct_deliveries(&["evt-a", "evt-b"], &["evt-a", "evt-b"])]
#[case::replayed_delivery(&["evt-a", "evt-a"], &["evt-a"])]
#[case::replay_of_one_of_two(&["evt-a", "evt-b", "evt-b"], &["evt-a", "evt-b"])]
#[tokio::test]
async fn each_delivery_is_read_once_before_any_merge(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    record: BTreeMap<String, Value>,
    #[case] delivered: &[&str],
    #[case] expected: &[&str],
) -> TestResult {
    let fixture = migrated_database?;
    for event_id in delivered {
        deliver(&fixture, &[(event_id, &record)]).await?;
    }

    let rows = lookup(&fixture, "team-a", "provider-response").await?;

    assert_eq!(
        rows.iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(rows.iter().all(|row| row.spend == Some(0.125)));
    assert!(rows.iter().all(|row| row.request_id == "billing-record"));
    Ok(())
}

#[rstest]
#[case::own_team("team-a", 1)]
#[case::other_team("team-b", 0)]
#[tokio::test]
async fn ledger_costs_are_visible_only_inside_their_tenant(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    record: BTreeMap<String, Value>,
    #[case] team: &str,
    #[case] visible: usize,
) -> TestResult {
    let fixture = migrated_database?;
    deliver(&fixture, &[("evt-a", &record)]).await?;

    assert_eq!(lookup(&fixture, team, "gateway-call").await?.len(), visible);
    Ok(())
}

#[rstest]
#[case::same_call_prefers_ledger("gateway-call", &["evt-a"])]
#[case::other_call_keeps_history("older-call", &["", "evt-a"])]
#[tokio::test]
async fn legacy_rows_are_read_only_when_no_ledger_event_owns_the_call(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    record: BTreeMap<String, Value>,
    #[case] legacy_call: &str,
    #[case] expected: &[&str],
) -> TestResult {
    let fixture = migrated_database?;
    let legacy = record
        .clone()
        .into_iter()
        .chain([
            ("litellm_call_id".to_owned(), json!(legacy_call)),
            ("request_id".to_owned(), json!("legacy-record")),
        ])
        .collect();
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        DATABASE,
        InsertTable::SpendLogs,
        vec![legacy],
    )
    .await?;
    deliver(&fixture, &[("evt-a", &record)]).await?;

    let rows = lookup(&fixture, "team-a", "provider-response").await?;

    assert_eq!(
        rows.iter()
            .map(|row| row.event_id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    Ok(())
}

#[rstest]
#[case::costs("lens_ledger_costs")]
#[case::aliases("lens_ledger_aliases")]
#[tokio::test]
async fn ledger_rows_are_partitioned_by_their_event_month(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    record: BTreeMap<String, Value>,
    #[case] table: &str,
) -> TestResult {
    let fixture = migrated_database?;
    deliver(&fixture, &[("evt-a", &record)]).await?;

    let expected = scalar(
        &fixture,
        &format!("SELECT toYYYYMM(fromUnixTimestamp64Milli(toInt64({START_MS})))"),
    )
    .await?;
    let partitions = scalar(
        &fixture,
        &format!(
            "SELECT groupUniqArray(partition) FROM system.parts \
             WHERE database = '{DATABASE}' AND table = '{table}' AND active FORMAT TSV"
        ),
    )
    .await?;
    assert_eq!(partitions, format!("['{expected}']"));
    Ok(())
}

fn store(server: &MockServer) -> Payloads {
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("AKID", "SECRET", None, None, "test"))
        .endpoint_url(server.uri())
        .force_path_style(true)
        .retry_config(RetryConfig::disabled())
        .build();
    Payloads::new(Arc::new(Store::new(
        aws_sdk_s3::Client::from_conf(config),
        BUCKET.into(),
        String::new(),
    )))
}

fn full_body() -> String {
    json!([
        {"role": "user", "content": format!("{} the needle 雪 is here", "hay ".repeat(5000))}
    ])
    .to_string()
}

async fn offload(fixture: &SeededDatabase, server: &MockServer, key: &ObjectKey) -> TestResult {
    offload_serving(
        fixture,
        server,
        key,
        ResponseTemplate::new(200).set_body_string(full_body()),
    )
    .await
}

async fn offload_serving(
    fixture: &SeededDatabase,
    server: &MockServer,
    key: &ObjectKey,
    served: ResponseTemplate,
) -> TestResult {
    let body = full_body();
    let record: BTreeMap<String, Value> = record()
        .into_iter()
        .chain([("messages".to_owned(), json!(body))])
        .collect();
    let row = ledger_row("evt-a", &record, 4096).map_err(|rejection| format!("{rejection:?}"))?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_ledger(&fixture.database.client, &writer, DATABASE, &[row]).await?;
    insert_payload_ref(
        &fixture.database.client,
        &writer,
        DATABASE,
        &PayloadRefRow {
            team_id: "team-a".into(),
            api_key: "key-a".into(),
            event_id: "evt-a".into(),
            field: Field::Messages.name(),
            event_time_ms: START_MS,
            object_key: key.as_str().to_owned(),
            bytes: body.len() as u64,
            sha256: sha256_hex(body.as_bytes()),
        },
    )
    .await?;
    Mock::given(method("GET"))
        .and(path(format!("/{BUCKET}/{}", key.as_str())))
        .respond_with(served)
        .mount(server)
        .await;
    Ok(())
}

fn tenant_key(team: &str) -> ObjectKey {
    ObjectKey::at_millis("", START_MS, team, "key-a", "evt-a", Field::Messages).unwrap()
}

async fn request_source(fixture: &SeededDatabase) -> TestResult<lens::LensRequestSourceRow> {
    let reader = Connection::writer(&fixture.database.url)?;
    let rows = fetch::<lens::LensRequestSource>(
        &fixture.database.client,
        &reader,
        &lens::LensRequestSourceParams {
            access: lens::LensAccessParams {
                all_teams: false,
                team: "team-a".into(),
                key_hash: String::new(),
            },
            source: lens::ContentSource::Requests,
            id: "billing-record".into(),
            record_team: "team-a".into(),
            start_time: "2026-01-01 00:00:00".into(),
        },
    )
    .await?;
    Ok(rows.into_iter().next().ok_or("request source missing")?)
}

#[rstest]
#[tokio::test]
async fn offloaded_bodies_expand_and_search_beyond_the_stored_preview(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let server = MockServer::start().await;
    offload(&fixture, &server, &tenant_key("team-a")).await?;
    let source = request_source(&fixture).await?;

    assert!(source.messages.len() <= 4096);
    assert!(!source.messages.contains("the needle"));
    let bodies = store(&server)
        .resolve(
            &fixture.database.client,
            &Connection::writer(&fixture.database.url)?,
            &source.team_id,
            &source.api_key,
            &source.event_id,
        )
        .await;
    assert_eq!(bodies.unavailable, None);
    let messages = bodies.messages.ok_or("body was not resolved")?;

    assert_eq!(messages, full_body());
    assert!(request_text(&messages, &source.response, &source.error_str).contains("the needle 雪"));
    let (page, more) = request_page(&messages, &source.response, &source.error_str, 16_001);
    assert!(page.contains("the needle 雪"));
    assert!(!more);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn references_outside_the_rows_tenant_are_never_fetched(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let server = MockServer::start().await;
    offload(&fixture, &server, &tenant_key("team-b")).await?;
    let source = request_source(&fixture).await?;

    let bodies = store(&server)
        .resolve(
            &fixture.database.client,
            &Connection::writer(&fixture.database.url)?,
            &source.team_id,
            &source.api_key,
            &source.event_id,
        )
        .await;

    assert_eq!(bodies.messages, None);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
    Ok(())
}

#[rstest]
#[case::deleted(
    ResponseTemplate::new(404).set_body_string(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>NoSuchKey</Code><Message>missing</Message></Error>",
    ),
    Unavailable::Expired
)]
#[case::changed(
    ResponseTemplate::new(200).set_body_string("[]"),
    Unavailable::Unreadable
)]
#[case::store_down(ResponseTemplate::new(503), Unavailable::Unreadable)]
#[tokio::test]
async fn unresolved_bodies_report_why_only_the_preview_is_available(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] served: ResponseTemplate,
    #[case] reason: Unavailable,
) -> TestResult {
    let fixture = migrated_database?;
    let server = MockServer::start().await;
    offload_serving(&fixture, &server, &tenant_key("team-a"), served).await?;
    let source = request_source(&fixture).await?;

    let bodies = store(&server)
        .resolve(
            &fixture.database.client,
            &Connection::writer(&fixture.database.url)?,
            &source.team_id,
            &source.api_key,
            &source.event_id,
        )
        .await;

    assert_eq!((bodies.messages, bodies.unavailable), (None, Some(reason)));
    assert!(lens_payloads::is_preview(&source.messages));
    Ok(())
}
