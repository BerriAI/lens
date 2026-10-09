mod evals {
    pub mod support;
}
use evals::support::{Database, database, span};
use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::query::evals::{EvalTrace, EvalTraceAttribute, EvalTraceParams};
use rstest::rstest;
use serde_json::json;
use sha2::{Digest, Sha256};

#[rstest]
#[case::trace_id(EvalTraceAttribute::TraceId, "trace-a", 2)]
#[case::session(EvalTraceAttribute::SessionId, "session", 3)]
#[case::missing(EvalTraceAttribute::SessionId, "absent", 0)]
#[tokio::test]
async fn eval_reads_resolve_complete_traces_within_the_dataset_team(
    #[future(awt)] database: Database,
    #[case] attribute: EvalTraceAttribute,
    #[case] value: &str,
    #[case] count: usize,
) {
    database
        .insert(vec![
            span("team-a", "trace-a", "root", "session", 1),
            span("team-a", "trace-a", "child", "", 1),
            span("team-a", "trace-b", "root", "session", 1),
            span("team-b", "trace-a", "foreign", "session", 1),
        ])
        .await;
    let rows = fetch::<EvalTrace>(
        &database.client,
        &database.reader,
        &EvalTraceParams {
            team: "team-a".into(),
            attribute,
            value: value.into(),
            cursor: String::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), count);
    assert!(!rows.iter().any(|row| row.span_id == "foreign"));
    if count > 0 {
        let root = rows
            .iter()
            .find(|row| row.trace_id == "trace-a" && row.span_id == "root")
            .unwrap();
        assert_eq!(root.input, "question");
        assert_eq!(root.output, "answer");
        assert_eq!(root.version, "build");
        assert_eq!(root.status, litellm_traces::SpanStatus::Ok);
        assert_eq!(root.end_ns - root.start_ns, 1_000_000);
        assert_eq!(
            root.trace_ref,
            format!("{:X}", Sha256::digest(b"team-a\0key\0trace-a"))
        );
    }
}

#[rstest]
#[tokio::test]
async fn eval_reads_page_without_duplicates_and_use_the_latest_receipt(
    #[future(awt)] database: Database,
) {
    let mut rows: Vec<_> = (0..1001)
        .map(|index| span("team-a", "trace", &format!("{index:04}"), "session", 1))
        .collect();
    let mut replacement = span("team-a", "trace", "0000", "session", 2);
    replacement["Output"] = json!("updated answer");
    rows.push(replacement);
    database.insert(rows).await;
    let mut params = EvalTraceParams {
        team: "team-a".into(),
        attribute: EvalTraceAttribute::TraceId,
        value: "trace".into(),
        cursor: String::new(),
    };
    let first = fetch::<EvalTrace>(&database.client, &database.reader, &params)
        .await
        .unwrap();
    assert_eq!(first.len(), 1000);
    assert_eq!(first[0].output, "updated answer");
    params.cursor = first.last().unwrap().key.clone();
    let second = fetch::<EvalTrace>(&database.client, &database.reader, &params)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].span_id, "1000");
    assert!(second[0].key > params.cursor);
}

#[rstest]
#[tokio::test]
async fn resource_session_and_version_attributes_resolve_without_span_attributes(
    #[future(awt)] database: Database,
) {
    let mut row = span("team", "trace", "root", "", 1);
    row["SpanAttributes"] = json!({});
    row["ResourceAttributes"] =
        json!({"session.id":"resource-session","agent.version":"resource-build"});
    database.insert(vec![row]).await;
    let rows = fetch::<EvalTrace>(
        &database.client,
        &database.reader,
        &EvalTraceParams {
            team: "team".into(),
            attribute: EvalTraceAttribute::SessionId,
            value: "resource-session".into(),
            cursor: String::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version, "resource-build");
}
