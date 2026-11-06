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

#[rstest]
#[case::prompt_first((100, 200, None), true, true)]
#[case::prompt_last((200, 100, None), true, true)]
#[case::title_first((150, 200, Some(100)), true, true)]
#[case::title_last((100, 200, Some(250)), true, true)]
#[case::logs_only_title_first((150, 200, Some(100)), true, false)]
#[case::logs_only_title_last((100, 200, Some(250)), true, false)]
#[case::title_only_with_trace((100, 200, Some(150)), false, true)]
#[case::title_only_logs((100, 200, Some(150)), false, false)]
#[tokio::test]
async fn codex_root_keeps_prompt_and_completed_turn_across_export_order(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    span: Span<'_>,
    access: ReadAccessParams,
    turns: TraceConversationTurnsParams,
    #[case] received: (u64, u64, Option<u64>),
    #[case] include_prompt: bool,
    #[case] include_trace: bool,
) -> TestResult {
    use litellm_traces::query::named::{SpanDetailParams, TracePageSpansParams, TraceSpansParams};
    let (prompt_received, trace_received, title_received) = received;
    let fixture = migrated_database?;
    let writer = Connection::writer(&fixture.database.url)?;
    let trace = Span {
        trace: "prior-codex",
        start: 1,
        duration: 8,
        received: trace_received,
        input: "",
        output: "",
        ..span
    };
    let mut trace_row = trace.row();
    trace_row.extend([
        ("SpanName".into(), json!("session_task.turn")),
        ("ObservationType".into(), json!("agent")),
        (
            "SpanAttributes".into(),
            json!({"session.id":"session-a","lens.codex.root":"1"}),
        ),
        ("StatusCode".into(), json!("STATUS_CODE_ERROR")),
        ("InputTokens".into(), json!(17)),
    ]);
    let mut prompt_row = Span {
        start: 2,
        duration: 0,
        received: prompt_received,
        input: "recorded prompt",
        ..trace
    }
    .row();
    prompt_row.extend([
        ("SpanName".into(), json!("codex.user_prompt")),
        ("ObservationType".into(), json!("agent")),
        ("InputPreview".into(), json!("recorded prompt")),
        ("SpanAttributes".into(), json!({"session.id":"session-a","lens.codex.root":"1","event.name":"codex.user_prompt"})),
    ]);
    let mut title_row = prompt_row.clone();
    title_row.insert("Timestamp".into(), json!(START_NS));
    title_row.insert("Input".into(), json!("title prompt"));
    title_row.insert("InputPreview".into(), json!("title prompt"));
    title_row.insert("SpanAttributes".into(), json!({"session.id":"session-a","lens.codex.root":"1","event.name":"codex.user_prompt","lens.codex.prompt_kind":"title"}));
    let mut early_row = trace_row.clone();
    early_row.insert("SpanName".into(), json!("turn/start"));
    early_row.insert("Duration".into(), json!(1));
    early_row.insert("StatusCode".into(), json!("STATUS_CODE_UNSET"));
    early_row.insert("EngineReceivedMs".into(), json!(50));
    let mut rows = Vec::new();
    if include_trace {
        rows.extend([early_row, trace_row]);
    }
    if include_prompt {
        rows.push(prompt_row);
    }
    if let Some(received) = title_received {
        title_row.insert("EngineReceivedMs".into(), json!(received));
        rows.push(title_row);
    }
    let body = encode_rows(rows)?;
    let expected_input = if include_prompt {
        "recorded prompt"
    } else {
        "title prompt"
    };
    let expected_start = START_NS
        + if include_trace {
            1
        } else if include_prompt {
            2
        } else {
            0
        };
    let expected_duration = if include_trace { 8 } else { 0 };
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
    let reader = ClickHouseTraces::new(
        fixture.database.client.clone(),
        Connection::reader(&fixture.database.url, DATABASE)?,
    );
    let anchor = reader
        .conversation_anchor(&TraceConversationAnchorParams {
            access: access.clone(),
            trace_id: trace.trace.into(),
            trace_ref: String::new(),
            snapshot_ms: 300,
        })
        .await?
        .ok_or("missing anchor")?;
    assert_eq!(anchor.start_ns, expected_start);
    assert_eq!(anchor.session_id, "session-a");
    let summary = reader
        .trace_spans(
            &TraceSpansParams {
                access: access.clone(),
                trace_id: trace.trace.into(),
                trace_ref: anchor.trace_ref.clone(),
            },
            300,
        )
        .await?;
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].input_preview, expected_input);
    assert_eq!(summary[0].duration_ns, expected_duration);
    assert_eq!(
        summary[0].status,
        if include_trace {
            litellm_traces::SpanStatus::Error
        } else {
            litellm_traces::SpanStatus::Unset
        }
    );
    assert_eq!(summary[0].input_tokens, if include_trace { 17 } else { 0 });
    let page = reader
        .run_spans(
            &TracePageSpansParams {
                access: access.clone(),
                trace_refs: vec![anchor.trace_ref.clone()],
                trace_ids: vec![trace.trace.into()],
                start_ms: START_NS / 1_000_000,
                end_ms: START_NS / 1_000_000 + 1,
            },
            300,
        )
        .await?;
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].input_preview, expected_input);
    assert_eq!(page[0].duration_ns, expected_duration);
    let detail = reader
        .span_detail(&SpanDetailParams {
            access,
            trace_id: trace.trace.into(),
            trace_ref: anchor.trace_ref,
            span_id: trace.span.into(),
        })
        .await?
        .ok_or("missing root content")?;
    assert_eq!(detail.input, expected_input);
    let previous = reader.conversation_turns(&turns).await?;
    assert_eq!(previous.len(), 1);
    assert_eq!(previous[0].input, expected_input);
    assert_eq!(previous[0].start_ns, expected_start);
    Ok(())
}
