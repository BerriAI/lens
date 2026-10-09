use std::collections::BTreeMap;

use litellm_traces::query::named::{
    ReadAccessParams, TraceConversationAnchorParams, TraceConversationTurnsParams,
};
use litellm_traces_cache::TraceStore;
use litellm_traces_clickhouse::{ClickHouseTraces, Connection, encode_rows};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

#[path = "queries/support.rs"]
mod fixtures;
mod support;

use fixtures::{DATABASE, SeededDatabase, migrated_database};
use support::TestResult;

const START_NS: i64 = 1_790_000_000_000_000_000;

#[derive(Clone, Copy)]
struct Span<'a> {
    trace: &'a str,
    span: &'a str,
    parent: &'a str,
    start: i64,
    duration: u64,
    received: u64,
    team: &'a str,
    key: &'a str,
    user: &'a str,
    session: &'a str,
    resource_session: &'a str,
    input: &'a str,
    output: &'a str,
}

impl Span<'_> {
    fn row(self) -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("Timestamp".into(), json!(START_NS + self.start)),
            ("TraceId".into(), json!(self.trace)),
            ("SpanId".into(), json!(self.span)),
            ("ParentSpanId".into(), json!(self.parent)),
            ("Duration".into(), json!(self.duration)),
            ("EngineReceivedMs".into(), json!(self.received)),
            ("TeamId".into(), json!(self.team)),
            ("ApiKeyHash".into(), json!(self.key)),
            ("UserId".into(), json!(self.user)),
            (
                "SpanAttributes".into(),
                json!({"session.id": self.session, "agent.source.url": "https://example.test/thread"}),
            ),
            (
                "ResourceAttributes".into(),
                json!({"session.id": self.resource_session}),
            ),
            ("Input".into(), json!(self.input)),
            ("Output".into(), json!(self.output)),
        ])
    }
}

#[fixture]
fn span() -> Span<'static> {
    Span {
        trace: "current",
        span: "root",
        parent: "",
        start: 10,
        duration: 1,
        received: 100,
        team: "team-a",
        key: "key-a",
        user: "user-a",
        session: "session-a",
        resource_session: "",
        input: "original request",
        output: "original reply",
    }
}

#[fixture]
fn access() -> ReadAccessParams {
    ReadAccessParams {
        all_teams: true,
        user_id: String::new(),
        team_ids: Vec::new(),
    }
}

async fn store(fixture: &SeededDatabase, spans: &[Span<'_>]) -> TestResult<ClickHouseTraces> {
    let writer = Connection::writer(&fixture.database.url)?;
    let body = encode_rows(spans.iter().copied().map(Span::row).collect())?;
    fixture
        .database
        .client
        .post(writer.url().clone())
        .body(format!(
            "INSERT INTO {DATABASE}.otel_traces FORMAT JSONEachRow\n{body}"
        ))
        .send()
        .await?
        .error_for_status()?;
    Ok(ClickHouseTraces::new(
        fixture.database.client.clone(),
        Connection::reader(&fixture.database.url, DATABASE)?,
    ))
}

#[rstest]
#[case::span_session("session-a", "", "", "session-a")]
#[case::resource_session("", "session-a", "", "session-a")]
#[case::conflicting_attributes("session-a", "another-session", "", "")]
#[case::matching_attributes("session-a", "session-a", "", "session-a")]
#[case::zero_parent("session-a", "", "0000000000000000", "session-a")]
#[case::no_session("", "", "", "")]
#[case::no_root("session-a", "", "missing-parent", "")]
#[tokio::test]
async fn anchor_requires_a_true_root_and_explicit_session(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    access: ReadAccessParams,
    #[case] session: &str,
    #[case] resource_session: &str,
    #[case] parent: &str,
    #[case] expected: &str,
) -> TestResult {
    let fixture = migrated_database?;
    let store = store(
        &fixture,
        &[Span {
            session,
            resource_session,
            parent,
            ..span
        }],
    )
    .await?;
    let anchor = store
        .conversation_anchor(&TraceConversationAnchorParams {
            access,
            trace_id: "current".into(),
            trace_ref: String::new(),
            snapshot_ms: 300,
        })
        .await?
        .ok_or("missing current trace")?;
    assert_eq!(anchor.session_id, expected);
    assert_eq!(anchor.team_id, span.team);
    assert_eq!(anchor.api_key_hash, span.key);
    if !expected.is_empty() {
        assert_eq!(anchor.start_ns, START_NS + span.start);
    }
    Ok(())
}

#[rstest]
#[case::conflicting_session("team-a", "key-a", "another-session")]
#[case::conflicting_key("team-a", "another-key", "session-a")]
#[case::conflicting_team("another-team", "key-a", "session-a")]
#[tokio::test]
async fn anchor_does_not_choose_between_sessions_or_owners(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    access: ReadAccessParams,
    #[case] team: &str,
    #[case] key: &str,
    #[case] session: &str,
) -> TestResult {
    let fixture = migrated_database?;
    let store = store(
        &fixture,
        &[
            span,
            Span {
                span: "child",
                parent: "root",
                team,
                key,
                session,
                ..span
            },
        ],
    )
    .await?;
    let anchor = store
        .conversation_anchor(&TraceConversationAnchorParams {
            access,
            trace_id: "current".into(),
            trace_ref: String::new(),
            snapshot_ms: 300,
        })
        .await?
        .ok_or("missing current trace")?;
    assert!(anchor.session_id.is_empty());
    Ok(())
}

#[rstest]
#[tokio::test]
async fn anchor_uses_visible_revisions_and_the_earliest_true_root(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    access: ReadAccessParams,
) -> TestResult {
    let fixture = migrated_database?;
    let store = store(
        &fixture,
        &[
            Span {
                session: "old-session",
                ..span
            },
            Span {
                received: 200,
                ..span
            },
            Span {
                received: 400,
                session: "future-session",
                ..span
            },
            Span {
                span: "child",
                parent: "root",
                start: 0,
                session: "",
                ..span
            },
            Span {
                span: "earlier-root",
                start: 5,
                session: "",
                ..span
            },
        ],
    )
    .await?;
    let params = TraceConversationAnchorParams {
        access,
        trace_id: "current".into(),
        trace_ref: String::new(),
        snapshot_ms: 300,
    };
    let anchor = store
        .conversation_anchor(&params)
        .await?
        .ok_or("missing trace")?;
    assert_eq!(anchor.session_id, "session-a");
    assert_eq!(anchor.start_ns, START_NS + 5);
    let scoped = store
        .conversation_anchor(&TraceConversationAnchorParams {
            trace_ref: anchor.trace_ref,
            ..params.clone()
        })
        .await?
        .ok_or("missing scoped trace")?;
    assert_eq!(scoped.session_id, "session-a");
    assert_eq!(scoped.start_ns, START_NS + 5);
    let absent = store
        .conversation_anchor(&TraceConversationAnchorParams {
            trace_ref: "not-this-owner".into(),
            ..params
        })
        .await?;
    assert!(absent.is_none());
    Ok(())
}

#[fixture]
fn turns(access: ReadAccessParams) -> TraceConversationTurnsParams {
    TraceConversationTurnsParams {
        access,
        team_id: "team-a".into(),
        api_key_hash: "key-a".into(),
        session_id: "session-a".into(),
        current_trace_id: "current".into(),
        before_ns: START_NS + 10,
        snapshot_ms: 300,
        has_cursor: 0,
        after_start_ns: 0,
        after_trace_ref: String::new(),
        after_span_id: String::new(),
        limit: 50,
    }
}

#[rstest]
#[tokio::test]
async fn prior_turns_preserve_payloads_and_page_chronologically_within_one_owner_and_session(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    turns: TraceConversationTurnsParams,
) -> TestResult {
    let fixture = migrated_database?;
    let prior = Span {
        trace: "prior-a",
        start: 1,
        ..span
    };
    let store = store(
        &fixture,
        &[
            span,
            prior,
            Span {
                received: 200,
                input: "updated request",
                output: "updated reply",
                ..prior
            },
            Span {
                received: 200,
                input: "another request",
                output: "another reply",
                ..prior
            },
            Span {
                received: 400,
                input: "future request",
                output: "future reply",
                ..prior
            },
            Span {
                span: "second-root",
                ..prior
            },
            Span {
                trace: "prior-b",
                parent: "0000000000000000",
                ..prior
            },
            Span {
                trace: "resource-session",
                start: 2,
                session: "",
                resource_session: "session-a",
                ..prior
            },
            Span {
                trace: "overlapping",
                start: 3,
                duration: 20,
                ..prior
            },
            Span {
                trace: "different-key",
                key: "key-b",
                ..prior
            },
            Span {
                trace: "different-team",
                team: "team-b",
                ..prior
            },
            Span {
                trace: "different-session",
                session: "session-b",
                ..prior
            },
            Span {
                trace: "conflicting-attributes",
                resource_session: "session-b",
                ..prior
            },
            Span {
                trace: "no-session",
                session: "",
                ..prior
            },
            Span {
                trace: "child",
                parent: "another-root",
                ..prior
            },
            Span {
                trace: "future",
                start: 11,
                ..prior
            },
            Span {
                trace: "same-time-as-current",
                start: 10,
                ..prior
            },
            Span {
                trace: "late-arrival",
                received: 400,
                ..prior
            },
            Span {
                trace: "changed-session",
                ..prior
            },
            Span {
                trace: "changed-session",
                received: 200,
                session: "session-b",
                ..prior
            },
            Span {
                trace: "changed-parent",
                ..prior
            },
            Span {
                trace: "changed-parent",
                received: 200,
                parent: "actual-parent",
                ..prior
            },
            Span {
                trace: "changed-time",
                ..prior
            },
            Span {
                trace: "changed-time",
                received: 200,
                start: 11,
                ..prior
            },
        ],
    )
    .await?;
    let rows = store.conversation_turns(&turns).await?;
    assert_eq!(rows.len(), 5);
    let updated = rows
        .iter()
        .find(|row| row.trace_id == "prior-a" && row.span_id == "root")
        .ok_or("missing first turn")?;
    assert_eq!(updated.input, "updated request");
    assert_eq!(updated.output, "updated reply");
    assert_eq!(rows[3].trace_id, "resource-session");
    assert_eq!(rows[4].trace_id, "overlapping");
    assert_eq!(rows[4].input, span.input);
    assert_eq!(rows[4].output, "");
    let order = rows
        .iter()
        .map(|row| (row.start_ns, row.trace_ref.clone(), row.span_id.clone()))
        .collect::<Vec<_>>();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
    let first_page = store
        .conversation_turns(&TraceConversationTurnsParams {
            limit: 1,
            ..turns.clone()
        })
        .await?;
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].trace_ref, rows[0].trace_ref);
    assert_eq!(first_page[0].span_id, rows[0].span_id);
    let remaining = store
        .conversation_turns(&TraceConversationTurnsParams {
            has_cursor: 1,
            after_start_ns: first_page[0].start_ns,
            after_trace_ref: first_page[0].trace_ref.clone(),
            after_span_id: first_page[0].span_id.clone(),
            ..turns
        })
        .await?;
    assert_eq!(
        remaining
            .iter()
            .map(|row| (row.trace_ref.as_str(), row.span_id.as_str()))
            .collect::<Vec<_>>(),
        rows[1..]
            .iter()
            .map(|row| (row.trace_ref.as_str(), row.span_id.as_str()))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[rstest]
#[case::user_allowed("user-a", vec![], true)]
#[case::team_allowed("", vec!["team-a"], true)]
#[case::other_user("user-b", vec![], false)]
#[case::other_team("", vec!["team-b"], false)]
#[case::no_access("", vec![], false)]
#[tokio::test]
async fn both_queries_apply_normal_read_access(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    turns: TraceConversationTurnsParams,
    #[case] user: &str,
    #[case] teams: Vec<&str>,
    #[case] allowed: bool,
) -> TestResult {
    let fixture = migrated_database?;
    let store = store(
        &fixture,
        &[
            span,
            Span {
                trace: "prior",
                start: 1,
                ..span
            },
        ],
    )
    .await?;
    let access = ReadAccessParams {
        all_teams: false,
        user_id: user.into(),
        team_ids: teams.into_iter().map(String::from).collect(),
    };
    let anchor = store
        .conversation_anchor(&TraceConversationAnchorParams {
            access: access.clone(),
            trace_id: "current".into(),
            trace_ref: String::new(),
            snapshot_ms: 300,
        })
        .await?;
    assert_eq!(anchor.is_some(), allowed);
    let rows = store
        .conversation_turns(&TraceConversationTurnsParams { access, ..turns })
        .await?;
    assert_eq!(rows.len(), usize::from(allowed));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn prior_turn_limit_is_bounded_in_storage(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    turns: TraceConversationTurnsParams,
) -> TestResult {
    let fixture = migrated_database?;
    let ids = (0..60)
        .map(|index| format!("prior-{index:02}"))
        .collect::<Vec<_>>();
    let spans = ids
        .iter()
        .map(|trace| Span {
            trace,
            start: 1,
            ..span
        })
        .collect::<Vec<_>>();
    let store = store(&fixture, &spans).await?;
    let rows = store
        .conversation_turns(&TraceConversationTurnsParams {
            limit: u32::MAX,
            ..turns
        })
        .await?;
    assert_eq!(rows.len(), 51);
    Ok(())
}
