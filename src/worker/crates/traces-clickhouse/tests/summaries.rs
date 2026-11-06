use litellm_traces::query::named::{ListTracesParams, ReadAccessParams};
use litellm_traces_cache::{TraceReader, TraceStore};
use litellm_traces_clickhouse::{
    ClickHouseTraces, Config, Connection, InsertTable, insert_rows, summaries,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "queries/support.rs"]
mod fixtures;
mod support;

use fixtures::{
    DATABASE, SeededDatabase, migrated_database, seed_interleaved_agent_runs, seeded_database,
};
use support::TestResult;

fn access(all_teams: bool, user_id: &str, teams: &[&str]) -> ReadAccessParams {
    ReadAccessParams {
        all_teams,
        user_id: user_id.into(),
        team_ids: teams.iter().map(|team| (*team).into()).collect(),
    }
}

fn list_params(access: ReadAccessParams) -> ListTracesParams {
    ListTracesParams {
        access,
        agent: String::new(),
        filter_keys: Vec::new(),
        filter_values: Vec::new(),
        start_ms: 0,
        end_ms: 2_000_000_000_000,
        cursor_ms: 0,
        cursor_trace_id: String::new(),
        limit: 50,
    }
}

fn span(user: &str, id: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("Timestamp".into(), json!(1_790_000_000_000_000_000_i64)),
        ("TraceId".into(), json!("shared-trace")),
        ("SpanId".into(), json!(id)),
        ("SpanName".into(), json!(format!("{user} step"))),
        ("InputPreview".into(), json!(format!("{user} input"))),
        ("ObservationType".into(), json!("llm")),
        ("TeamId".into(), json!("team")),
        ("ApiKeyHash".into(), json!("key")),
        ("UserId".into(), json!(user)),
        ("Duration".into(), json!(1_000_000)),
        ("LiteLLMRequestId".into(), json!(format!("response-{id}"))),
        ("CallEvidence".into(), json!("complete")),
    ])
}

fn cost(user: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("request_id".into(), json!("request-first")),
        ("response_id".into(), json!("response-first")),
        ("team_id".into(), json!("team")),
        ("api_key".into(), json!("key")),
        ("user".into(), json!(user)),
        ("start_time".into(), json!(1_790_000_000_000_i64)),
        ("end_time".into(), json!(1_790_000_000_001_i64)),
        ("spend".into(), json!(0.25)),
    ])
}

#[fixture]
async fn litellm_tagged_database(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult<SeededDatabase> {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let tagged = |id: &str, team: &str, timestamp: i64, attributes: Value| {
        span("owner", id)
            .into_iter()
            .chain([
                ("TraceId".into(), json!(id)),
                ("Timestamp".into(), json!(timestamp)),
                ("TeamId".into(), json!(team)),
                ("SpanAttributes".into(), attributes),
            ])
            .collect()
    };
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![
            tagged("spend-match", "team", 1_790_000_002_000_000_000, json!({"skill":"diagram"})),
            tagged("direct-match", "team", 1_790_000_001_000_000_000, json!({"metadata.tags":"[\"Skill A\",\"department:engineering\",\" spaced tag \" ]", "skill":"diagram"})),
            tagged("newest-unmatched", "team", 1_790_000_003_000_000_000, json!({"metadata.tags":"['Skill A', 'department:engineering']"})),
            tagged("other-key-only", "team", 1_790_000_004_000_000_000, json!({})),
            tagged("foreign", "foreign", 1_790_000_005_000_000_000, json!({"request_tags":"[\"Skill A\",\"department:engineering\"]"})),
        ],
    )
    .await?;
    let spend = |id: &str, key: &str, tags: Value| {
        cost(if key == "other-key" {
            "another-owner"
        } else {
            "owner"
        })
        .into_iter()
        .chain([
            ("request_id".into(), json!(format!("request-{id}"))),
            ("response_id".into(), json!(format!("response-{id}"))),
            ("api_key".into(), json!(key)),
            ("request_tags".into(), tags),
        ])
        .collect()
    };
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![
            spend(
                "spend-match",
                "gateway-key",
                json!(["Skill A", "department:engineering", " spaced tag "]),
            ),
            spend("newest-unmatched", "key", json!(["Skill A"])),
            spend(
                "other-key-only",
                "other-key",
                json!(["Skill A", "department:engineering"]),
            ),
        ],
    )
    .await?;
    Ok(fixture)
}

#[rstest]
#[case::unprojected(false)]
#[case::projected(true)]
#[tokio::test]
async fn litellm_string_tags_filter_before_pagination_without_cross_key_matches(
    #[future(awt)] litellm_tagged_database: TestResult<SeededDatabase>,
    #[case] projected: bool,
) -> TestResult {
    use litellm_traces::request::TraceTag;
    let fixture = litellm_tagged_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    if projected {
        summaries::project(&fixture.database.client, &config, 16).await?;
    }
    let store = ClickHouseTraces::new(
        fixture.database.client.clone(),
        config.storage().reader().clone(),
    );
    let reader = TraceReader::new(usize::MAX);
    let scope = access(false, "", &["team"]);
    let filters = [
        TraceTag {
            key: "tag".into(),
            value: "Skill A".into(),
        },
        TraceTag {
            key: "tag".into(),
            value: "department:engineering".into(),
        },
        TraceTag {
            key: "tag".into(),
            value: " spaced tag ".into(),
        },
    ];
    let first = reader
        .list_tagged_traces(&store, &scope, "", &filters, 0, 2_000_000_000_000, None, 1)
        .await?;
    assert_eq!(first.data.len(), 1);
    assert_eq!(first.data[0].trace_id, "spend-match");
    let second = reader
        .list_tagged_traces(
            &store,
            &scope,
            "",
            &filters,
            0,
            2_000_000_000_000,
            first.next_cursor.as_deref(),
            1,
        )
        .await?;
    assert_eq!(second.data.len(), 1);
    assert_eq!(second.data[0].trace_id, "direct-match");
    let recorded = litellm_traces_clickhouse::tags::read(
        &fixture.database.client,
        &config,
        &scope,
        "spend-match",
        "",
    )
    .await?
    .unwrap();
    assert!(recorded.recorded.contains(&filters[2]));
    assert!(recorded.manual.is_empty());
    Ok(())
}

#[rstest]
#[case::preview(1)]
#[case::sample(0)]
#[tokio::test]
async fn litellm_string_tags_select_the_same_investigation_runs_and_finding_sources(
    #[future(awt)] litellm_tagged_database: TestResult<SeededDatabase>,
    #[case] preview: u8,
) -> TestResult {
    use litellm_traces_clickhouse::query::lens::{
        ExecutionSource, LensAccessParams, LensFindingSource, LensFindingSourceParams, LensSample,
        LensSampleParams,
    };
    let fixture = litellm_tagged_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let params = LensSampleParams {
        profile: String::new(),
        access: LensAccessParams {
            all_teams: false,
            team: "team".into(),
            key_hash: "key".into(),
        },
        source: ExecutionSource::Traces,
        start: 1_789_999_000_000,
        end: 2_000_000_000_000,
        agent_name: String::new(),
        service: String::new(),
        filter_keys: vec!["tag".into(), "tag".into()],
        filter_values: vec!["Skill A".into(), "department:engineering".into()],
        selected_team: String::new(),
        execution_ids: Vec::new(),
        sample_cap: 0,
        sample_percent: 100.0,
        preview,
        after: String::new(),
        limit: 50,
        offset: 0,
    };
    let rows = litellm_storage_clickhouse::fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &params,
    )
    .await?;
    let mut names: Vec<_> = rows.iter().map(|row| row.trace_id.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["direct-match", "spend-match"]);
    let selected = rows
        .iter()
        .find(|row| row.trace_id == "spend-match")
        .unwrap();
    assert!(
        selected
            .attributes
            .contains(&("tag".into(), "Skill A".into()))
    );
    let finding = LensFindingSourceParams {
        profile: String::new(),
        access: params.access,
        trace_id: selected.trace_id.clone(),
        trace_ref: selected.trace_ref.clone(),
        agent_name: String::new(),
        selected_team: String::new(),
        service: String::new(),
        filter_keys: params.filter_keys,
        filter_values: params.filter_values,
    };
    let found = litellm_storage_clickhouse::fetch::<LensFindingSource>(
        &fixture.database.client,
        config.storage().reader(),
        &finding,
    )
    .await?;
    assert_eq!(found.len(), 1);
    Ok(())
}

#[rstest]
#[case::provider_response("provider_response:response-first", "", "", "")]
#[case::provider_request("provider_request:request-first", "", "request-first", "")]
#[case::gateway_call("litellm_request:gateway-first", "gateway-first", "", "")]
#[case::transport("transport:", "", "", "model-span")]
#[tokio::test]
async fn litellm_tag_lookup_uses_recorded_call_aliases_and_prioritizes_upstream_tags(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] call_key: &str,
    #[case] gateway_call: &str,
    #[case] provider_request: &str,
    #[case] transport_span: &str,
) -> TestResult {
    use litellm_traces::request::TraceTag;
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let attributes: BTreeMap<_, _> = (0..70)
        .map(|index| (format!("label-{index}"), format!("value-{index}")))
        .collect();
    let trace = span("owner", "model-span")
        .into_iter()
        .chain([
            ("CallKeys".into(), json!([call_key])),
            ("SpanAttributes".into(), json!(attributes)),
        ])
        .collect();
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![trace],
    )
    .await?;
    let request = cost("owner")
        .into_iter()
        .chain([
            ("litellm_call_id".into(), json!(gateway_call)),
            ("provider_request_id".into(), json!(provider_request)),
            ("trace_id".into(), json!("shared-trace")),
            ("span_id".into(), json!(transport_span)),
            (
                "request_tags".into(),
                json!(["Skill A", "department:engineering"]),
            ),
        ])
        .collect();
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![request],
    )
    .await?;
    let result = litellm_traces_clickhouse::tags::read(
        &fixture.database.client,
        &config,
        &access(false, "owner", &[]),
        "shared-trace",
        "",
    )
    .await?
    .unwrap();
    assert_eq!(result.recorded.len(), 64);
    assert_eq!(
        result.recorded[0],
        TraceTag {
            key: "tag".into(),
            value: "Skill A".into()
        }
    );
    assert_eq!(
        result.recorded[1],
        TraceTag {
            key: "tag".into(),
            value: "department:engineering".into()
        }
    );
    Ok(())
}

#[rstest]
#[case::same_owner_different_key("gateway-key", "owner", false, true)]
#[case::different_owner_different_key("gateway-key", "someone-else", false, false)]
#[case::same_key_different_owner("key", "someone-else", false, true)]
#[case::ambiguous_owned_calls("gateway-key", "owner", true, false)]
#[tokio::test]
async fn litellm_tag_lookup_preserves_spend_ownership_and_rejects_ambiguous_calls(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] request_key: &str,
    #[case] request_user: &str,
    #[case] ambiguous: bool,
    #[case] matches: bool,
) -> TestResult {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    let request = |request_id: &str, key: &str| {
        cost(request_user)
            .into_iter()
            .chain([
                ("request_id".into(), json!(request_id)),
                ("api_key".into(), json!(key)),
                ("request_tags".into(), json!(["Skill A"])),
            ])
            .collect()
    };
    let requests = if ambiguous {
        vec![
            request("request-first", request_key),
            request("request-second", "second-owned-key"),
        ]
    } else {
        vec![request("request-first", request_key)]
    };
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        requests,
    )
    .await?;
    let result = litellm_traces_clickhouse::tags::read(
        &fixture.database.client,
        &config,
        &access(false, "owner", &[]),
        "shared-trace",
        "",
    )
    .await?
    .unwrap();
    let upstream: Vec<_> = result
        .recorded
        .into_iter()
        .filter(|tag| tag.key == "tag")
        .map(|tag| tag.value)
        .collect();
    assert_eq!(
        upstream,
        if matches {
            vec!["Skill A".to_owned()]
        } else {
            Vec::new()
        }
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn corrected_spend_aliases_do_not_attach_new_tags_to_an_old_response(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let client = &fixture.database.client;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    let original = cost("owner")
        .into_iter()
        .chain([("request_tags".into(), json!(["original"]))])
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![original],
    )
    .await?;
    let scope = access(false, "owner", &[]);
    let before = litellm_traces_clickhouse::tags::read(client, &config, &scope, "shared-trace", "")
        .await?
        .unwrap();
    assert!(
        before
            .recorded
            .iter()
            .any(|tag| tag.key == "tag" && tag.value == "original")
    );
    let correction = cost("owner")
        .into_iter()
        .chain([
            ("response_id".into(), json!("corrected-response")),
            ("end_time".into(), json!(1_790_000_000_002_i64)),
            ("request_tags".into(), json!(["corrected"])),
        ])
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![correction],
    )
    .await?;
    let after = litellm_traces_clickhouse::tags::read(client, &config, &scope, "shared-trace", "")
        .await?
        .unwrap();
    assert!(
        after
            .recorded
            .iter()
            .filter(|tag| tag.key == "tag")
            .collect::<Vec<_>>()
            .is_empty()
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_late_evaluation_marker_excludes_an_existing_summary(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    let params = list_params(access(true, "", &[]));
    assert_eq!(store.summary_page(&params).await?.unwrap().data.len(), 1);

    let marker = span("owner", "evaluation")
        .into_iter()
        .chain([
            (
                "Timestamp".into(),
                json!(time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64),
            ),
            (
                "SpanAttributes".into(),
                json!({"deployment.environment": "lens-eval"}),
            ),
        ])
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![marker],
    )
    .await?;
    assert!(store.summary_page(&params).await?.unwrap().data.is_empty());
    assert_eq!(summaries::project(client, &config, 16).await?, 0);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn summary_pages_filter_agents_before_pagination(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let (start_ms, end_ms) = seed_interleaved_agent_runs(&fixture).await?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    summaries::project(&fixture.database.client, &config, 16).await?;
    let store = ClickHouseTraces::new(
        fixture.database.client.clone(),
        config.storage().reader().clone(),
    );
    let reader = TraceReader::new(usize::MAX);
    let access = access(true, "", &[]);
    let params = ListTracesParams {
        access: access.clone(),
        agent: "A".into(),
        filter_keys: Vec::new(),
        filter_values: Vec::new(),
        start_ms,
        end_ms,
        cursor_ms: 0,
        cursor_trace_id: String::new(),
        limit: 2,
    };
    assert!(store.summary_page(&params).await?.is_some());
    let first = reader
        .list_traces(&store, &access, "A", start_ms, end_ms, None, 2)
        .await?;
    assert_eq!(
        first
            .data
            .iter()
            .map(|run| run.trace_id.as_str())
            .collect::<Vec<_>>(),
        ["a-new", "a-middle"]
    );
    let next = reader
        .list_traces(
            &store,
            &access,
            "A",
            start_ms,
            end_ms,
            first.next_cursor.as_deref(),
            2,
        )
        .await?;
    assert_eq!(
        next.data
            .iter()
            .map(|run| run.trace_id.as_str())
            .collect::<Vec<_>>(),
        ["a-old"]
    );
    let unfiltered = reader
        .list_traces(&store, &access, "", start_ms, end_ms, None, 10)
        .await?;
    assert_eq!(unfiltered.data.len(), 6);
    assert_eq!(unfiltered.data[0].trace_id, "b-new");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn oversized_summary_does_not_block_projection_of_other_traces(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let oversized = span("owner", "first")
        .into_iter()
        .chain([("InputPreview".into(), json!("x".repeat(2 * 1024 * 1024)))])
        .collect();
    let smaller = span("owner", "second")
        .into_iter()
        .chain([
            ("TraceId".into(), json!("smaller-trace")),
            ("Timestamp".into(), json!(1_789_999_999_000_000_000_i64)),
        ])
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![oversized, smaller],
    )
    .await?;
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    let scope = access(true, "", &[]);
    let rows = store.list_runs(&list_params(scope.clone())).await?;

    assert_eq!(summaries::project(client, &config, 1).await?, 1);
    assert!(store.summaries(&scope, &rows).await?.is_empty());
    assert_eq!(summaries::project(client, &config, 1).await?, 1);
    let saved = store.summaries(&scope, &rows).await?;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].trace_id, "smaller-trace");
    assert_eq!(saved[0].span_count, 1);
    assert_eq!(summaries::project(client, &config, 16).await?, 0);
    Ok(())
}

#[rstest]
#[case::cooling_down(0, 0)]
#[case::retry_due(120_000, 1)]
#[tokio::test]
async fn deferred_projections_retry_only_after_the_cooldown(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] age_ms: u64,
    #[case] expected: usize,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    let scope = access(true, "", &[]);
    let rows = store.list_runs(&list_params(scope.clone())).await?;
    let computed_at =
        (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64 - age_ms;
    let marker = json!({
        "TeamId": "team", "ApiKeyHash": "key", "TraceId": "shared-trace",
        "TraceRef": rows[0].trace_ref, "ScopeUserId": "", "SourceSpanCount": 0,
        "StartTs": "2026-09-21 14:13:20.000", "ComputedAt": computed_at,
        "ResolutionVersion": 0, "SpendKeys": [], "Summary": ""
    });
    client
        .post(config.storage().writer().url().clone())
        .body(format!(
            "INSERT INTO {DATABASE}.lens_trace_summaries FORMAT JSONEachRow\n{marker}"
        ))
        .send()
        .await?
        .error_for_status()?;

    assert_eq!(summaries::project(client, &config, 16).await?, expected);
    assert_eq!(store.summaries(&scope, &rows).await?.len(), expected);
    Ok(())
}

#[rstest]
#[case::administrator(access(true, "", &[]))]
#[case::team(access(false, "", &["team"]))]
#[case::owner(access(false, "owner", &[]))]
#[case::other_user(access(false, "other", &[]))]
#[tokio::test]
async fn summaries_preserve_user_visibility_within_a_shared_trace(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] scope: ReadAccessParams,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("other", "second")],
    )
    .await?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![cost("other")],
    )
    .await?;
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    let before = TraceReader::new(usize::MAX)
        .list_traces(&store, &scope, "", 0, 2_000_000_000_000, None, 50)
        .await?;
    assert_eq!(before.data.len(), 1);

    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let rows = store.list_runs(&list_params(scope.clone())).await?;
    assert_eq!(store.summaries(&scope, &rows).await?, before.data);
    assert_eq!(
        store.summary_page(&list_params(scope)).await?.unwrap().data,
        before.data
    );
    Ok(())
}

#[rstest]
#[case::alias("response_id", "different-response")]
#[case::owner("user", "other")]
#[tokio::test]
async fn cost_corrections_invalidate_previous_aliases_and_owners(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] field: &str,
    #[case] replacement: &str,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let scope = access(false, "owner", &[]);
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![cost("owner")],
    )
    .await?;
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let rows = store.list_runs(&list_params(scope.clone())).await?;
    assert_eq!(store.summaries(&scope, &rows).await?[0].spend, Some(0.25));
    let corrected = cost("owner")
        .into_iter()
        .chain([
            (field.into(), json!(replacement)),
            ("end_time".into(), json!(1_790_000_000_002_i64)),
        ])
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![corrected],
    )
    .await?;
    assert!(store.summaries(&scope, &rows).await?.is_empty());
    assert!(
        store
            .summary_page(&list_params(scope.clone()))
            .await?
            .is_none()
    );
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let updated = store.summaries(&scope, &rows).await?;
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].spend, None);
    assert_eq!(updated[0].priced_calls, 0);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn late_spans_and_costs_refresh_persisted_totals(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let client = &fixture.database.client;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let scope = access(true, "", &[]);
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "first")],
    )
    .await?;
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let rows = store.list_runs(&list_params(scope.clone())).await?;
    assert_eq!(store.summaries(&scope, &rows).await?[0].spend, None);
    assert_eq!(summaries::project(client, &config, 16).await?, 0);

    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::SpendLogs,
        vec![cost("owner")],
    )
    .await?;
    assert!(store.summaries(&scope, &rows).await?.is_empty());
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let priced = store.summaries(&scope, &rows).await?;
    assert_eq!(priced[0].spend, Some(0.25));
    assert_eq!(priced[0].priced_calls, 1);

    insert_rows(
        client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "late")],
    )
    .await?;
    let rows = store.list_runs(&list_params(scope.clone())).await?;
    assert!(store.summaries(&scope, &rows).await?.is_empty());
    assert_eq!(summaries::project(client, &config, 16).await?, 1);
    let updated = store.summaries(&scope, &rows).await?;
    assert_eq!(updated[0].span_count, 2);
    assert_eq!(updated[0].spend, Some(0.25));
    assert_eq!(updated[0].llm_calls, 2);
    assert_eq!(summaries::project(client, &config, 16).await?, 0);
    Ok(())
}

#[rstest]
#[case::administrator(true, "")]
#[case::team_a(false, "team-a")]
#[case::team_b(false, "team-b")]
#[case::unauthorized(false, "team-none")]
#[tokio::test]
async fn persistent_summaries_match_resolved_lists_after_a_reader_restart(
    #[future(awt)] seeded_database: TestResult<SeededDatabase>,
    #[case] all_teams: bool,
    #[case] team_id: &str,
    #[values(1, 50)] limit: u32,
) -> TestResult {
    let fixture = seeded_database?;
    let config = Config::new(DATABASE.to_owned(), &fixture.database.url, 7)?;
    let access = ReadAccessParams {
        all_teams,
        user_id: String::new(),
        team_ids: vec![team_id.to_owned()],
    };
    let store = ClickHouseTraces::new(
        fixture.database.client.clone(),
        Connection::reader(&fixture.database.url, DATABASE)?,
    );
    let before = TraceReader::new(usize::MAX)
        .list_traces(&store, &access, "", 0, 2_000_000_000_000, None, limit)
        .await?;
    let before_next = if let Some(cursor) = before.next_cursor.as_deref() {
        Some(
            TraceReader::new(usize::MAX)
                .list_traces(
                    &store,
                    &access,
                    "",
                    0,
                    2_000_000_000_000,
                    Some(cursor),
                    limit,
                )
                .await?,
        )
    } else {
        None
    };

    assert!(summaries::project(&fixture.database.client, &config, 16).await? > 0);

    let rows = store
        .list_runs(&ListTracesParams {
            access: access.clone(),
            agent: String::new(),
            filter_keys: Vec::new(),
            filter_values: Vec::new(),
            start_ms: 0,
            end_ms: 2_000_000_000_000,
            cursor_ms: 0,
            cursor_trace_id: String::new(),
            limit,
        })
        .await?;
    let persisted = store.summaries(&access, &rows).await?;
    assert_eq!(persisted.len(), before.data.len());
    let params = ListTracesParams {
        limit,
        ..list_params(access.clone())
    };
    assert_eq!(
        store.summary_page(&params).await?.unwrap().data,
        before.data
    );
    assert_eq!(
        summaries::project(&fixture.database.client, &config, 16).await?,
        0
    );

    let after = TraceReader::new(usize::MAX)
        .list_traces(&store, &access, "", 0, 2_000_000_000_000, None, limit)
        .await?;
    assert_eq!(after, before);
    if let Some(cursor) = after.next_cursor.as_deref() {
        let after_next = TraceReader::new(usize::MAX)
            .list_traces(
                &store,
                &access,
                "",
                0,
                2_000_000_000_000,
                Some(cursor),
                limit,
            )
            .await?;
        assert_eq!(Some(after_next), before_next);
    }
    Ok(())
}

#[rstest]
#[case::unprojected(false)]
#[case::projected(true)]
#[tokio::test]
async fn tags_filter_before_pagination_and_preserve_tenant_scope(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] projected: bool,
) -> TestResult {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let tagged = |id: &str, team: &str, skill: &str, timestamp: i64| {
        BTreeMap::from([
            ("Timestamp".into(), json!(timestamp)),
            ("TraceId".into(), json!(id)),
            ("SpanId".into(), json!(id)),
            ("SpanName".into(), json!("agent")),
            ("ObservationType".into(), json!("agent")),
            ("TeamId".into(), json!(team)),
            ("ApiKeyHash".into(), json!("key")),
            ("UserId".into(), json!("owner")),
            ("ResourceAttributes".into(), json!({"skill":skill})),
            ("SpanAttributes".into(), json!({"department":"engineering"})),
            ("Duration".into(), json!(1_000_000)),
        ])
    };
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![
            tagged(
                "newest-unmatched",
                "team",
                "other",
                1_790_000_003_000_000_000,
            ),
            tagged(
                "foreign-match",
                "foreign",
                "diagram",
                1_790_000_002_000_000_000,
            ),
            tagged("first-match", "team", "diagram", 1_790_000_001_000_000_000),
            tagged("second-match", "team", "diagram", 1_790_000_000_000_000_000),
        ],
    )
    .await?;
    if projected {
        summaries::project(&fixture.database.client, &config, 16).await?;
    }
    let store = ClickHouseTraces::new(
        fixture.database.client.clone(),
        config.storage().reader().clone(),
    );
    let reader = TraceReader::new(usize::MAX);
    let scope = access(false, "", &["team"]);
    let tags = vec![
        litellm_traces::request::TraceTag {
            key: "skill".into(),
            value: "diagram".into(),
        },
        litellm_traces::request::TraceTag {
            key: "department".into(),
            value: "engineering".into(),
        },
    ];
    let first = reader
        .list_tagged_traces(&store, &scope, "", &tags, 0, 2_000_000_000_000, None, 1)
        .await?;
    assert_eq!(first.data[0].trace_id, "first-match");
    let second = reader
        .list_tagged_traces(
            &store,
            &scope,
            "",
            &tags,
            0,
            2_000_000_000_000,
            first.next_cursor.as_deref(),
            1,
        )
        .await?;
    assert_eq!(second.data[0].trace_id, "second-match");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn manual_tags_persist_filter_and_select_investigation_activity(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    use litellm_traces::request::TraceTag;
    use litellm_traces_clickhouse::{
        query::lens::{ExecutionSource, LensAccessParams, LensSample, LensSampleParams},
        tags,
    };
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![span("owner", "manual-step")],
    )
    .await?;
    let scope = access(false, "owner", &[]);
    let tag = TraceTag {
        key: "skill".into(),
        value: "diagram".into(),
    };
    let saved = tags::write(
        &fixture.database.client,
        &config,
        &scope,
        "shared-trace",
        "",
        std::slice::from_ref(&tag),
    )
    .await?
    .unwrap();
    assert_eq!(saved.manual, vec![tag.clone()]);
    let reloaded = tags::read(
        &fixture.database.client,
        &config,
        &scope,
        "shared-trace",
        "",
    )
    .await?
    .unwrap();
    assert_eq!(reloaded.manual, vec![tag.clone()]);
    let other = access(false, "stranger", &["foreign"]);
    assert!(
        tags::read(
            &fixture.database.client,
            &config,
            &other,
            "shared-trace",
            ""
        )
        .await?
        .is_none()
    );
    assert!(
        tags::write(
            &fixture.database.client,
            &config,
            &other,
            "shared-trace",
            "",
            &[]
        )
        .await?
        .is_none()
    );
    let store = ClickHouseTraces::new(
        fixture.database.client.clone(),
        config.storage().reader().clone(),
    );
    let reader = TraceReader::new(usize::MAX);
    let page = reader
        .list_tagged_traces(&store, &scope, "", &[tag], 0, 2_000_000_000_000, None, 50)
        .await?;
    assert_eq!(page.data.len(), 1);
    let selected = litellm_storage_clickhouse::fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &LensSampleParams {
            profile: String::new(),
            access: LensAccessParams {
                all_teams: true,
                team: String::new(),
                key_hash: String::new(),
            },
            source: ExecutionSource::Traces,
            start: 1_789_999_000_000,
            end: 2_000_000_000_000,
            agent_name: String::new(),
            service: String::new(),
            filter_keys: vec!["skill".into()],
            filter_values: vec!["diagram".into()],
            selected_team: String::new(),
            execution_ids: Vec::new(),
            sample_cap: 0,
            sample_percent: 100.0,
            preview: 1,
            after: String::new(),
            limit: 50,
            offset: 0,
        },
    )
    .await?;
    assert_eq!(selected.len(), 1);
    assert!(
        selected[0]
            .attributes
            .contains(&("skill".into(), "diagram".into()))
    );
    tags::write(
        &fixture.database.client,
        &config,
        &scope,
        "shared-trace",
        "",
        &[],
    )
    .await?
    .unwrap();
    assert!(
        tags::read(
            &fixture.database.client,
            &config,
            &scope,
            "shared-trace",
            ""
        )
        .await?
        .unwrap()
        .manual
        .is_empty()
    );
    let empty = reader
        .list_tagged_traces(
            &store,
            &scope,
            "",
            &[TraceTag {
                key: "skill".into(),
                value: "diagram".into(),
            }],
            0,
            2_000_000_000_000,
            None,
            50,
        )
        .await?;
    assert!(empty.data.is_empty());
    Ok(())
}
