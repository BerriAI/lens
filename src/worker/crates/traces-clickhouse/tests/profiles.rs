use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::{
    Config, InsertTable, insert_rows,
    query::lens::{
        ExecutionSource, LensAccessParams, LensFindingSource, LensFindingSourceParams, LensSample,
        LensSampleParams,
    },
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "queries/support.rs"]
mod fixtures;
mod support;
use fixtures::{DATABASE, SeededDatabase, migrated_database};
use support::TestResult;

fn row(
    trace: &str,
    span: &str,
    parent: &str,
    time: i64,
    resource: Value,
    attributes: Value,
    tenant: (&str, &str),
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "Timestamp".into(),
            json!(1_790_000_000_000_000_000_i64 + time),
        ),
        ("TraceId".into(), json!(trace)),
        ("SpanId".into(), json!(span)),
        ("ParentSpanId".into(), json!(parent)),
        ("SpanName".into(), json!("agent")),
        ("ObservationType".into(), json!("agent")),
        ("AgentName".into(), json!("assistant")),
        ("TeamId".into(), json!(tenant.0)),
        ("ApiKeyHash".into(), json!(tenant.1)),
        ("Duration".into(), json!(1_000_000)),
        ("ResourceAttributes".into(), resource),
        ("SpanAttributes".into(), attributes),
    ])
}

#[fixture]
async fn profiled_database(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult<SeededDatabase> {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![
            row(
                "coding",
                "root",
                "",
                10,
                json!({"task_category":"Workflows","scenario":"Research"}),
                json!({"task_category":" Coding "}),
                ("team", "key"),
            ),
            row(
                "coding",
                "child",
                "root",
                0,
                json!({}),
                json!({"task_category":"Workflows"}),
                ("team", "key"),
            ),
            row(
                "workflows",
                "root",
                "",
                20,
                json!({"scenario":" Workflows "}),
                json!({}),
                ("team", "key"),
            ),
            row(
                "workflows",
                "child",
                "root",
                0,
                json!({}),
                json!({"task_category":"Coding"}),
                ("team", "key"),
            ),
            row(
                "earliest",
                "root",
                "",
                0,
                json!({"task_category":" ","scenario":""}),
                json!({}),
                ("team", "key"),
            ),
            row(
                "earliest",
                "first-child",
                "root",
                1,
                json!({"scenario":"Coding"}),
                json!({}),
                ("team", "key"),
            ),
            row(
                "earliest",
                "second-child",
                "root",
                2,
                json!({}),
                json!({"task_category":"Workflows"}),
                ("team", "key"),
            ),
            row(
                "resource-priority",
                "root",
                "",
                0,
                json!({"task_category":"Coding"}),
                json!({"scenario":"Workflows"}),
                ("team", "key"),
            ),
            row(
                "unclassified",
                "root",
                "",
                0,
                json!({"scenario":" "}),
                json!({}),
                ("team", "key"),
            ),
            row(
                "coding",
                "foreign-root",
                "",
                0,
                json!({}),
                json!({"task_category":"Workflows"}),
                ("foreign", "key"),
            ),
            row(
                "coding",
                "other-key-root",
                "",
                0,
                json!({}),
                json!({"task_category":"Workflows"}),
                ("team", "other-key"),
            ),
        ],
    )
    .await?;
    Ok(fixture)
}

fn access() -> LensAccessParams {
    LensAccessParams {
        all_teams: false,
        team: "team".into(),
        key_hash: "key".into(),
    }
}
fn parameters(profile: &str, preview: u8) -> LensSampleParams {
    LensSampleParams {
        profile: profile.into(),
        access: access(),
        source: ExecutionSource::Traces,
        start: 1_789_999_000_000,
        end: 2_000_000_000_000,
        agent_name: "assistant".into(),
        service: String::new(),
        filter_keys: Vec::new(),
        filter_values: Vec::new(),
        selected_team: String::new(),
        execution_ids: Vec::new(),
        sample_cap: 0,
        sample_percent: 100.0,
        preview,
        after: String::new(),
        limit: 100,
        offset: 0,
    }
}

#[rstest]
#[case::all("",vec!["coding","earliest","resource-priority","unclassified","workflows"])]
#[case::coding("Coding",vec!["coding","earliest","resource-priority"])]
#[case::workflows("Workflows",vec!["workflows"])]
#[case::unclassified("Unclassified",vec!["unclassified"])]
#[case::unknown("Not recorded",vec![])]
#[case::recorded_id("recorded:Coding",vec!["coding","earliest","resource-priority"])]
#[case::unclassified_id("unclassified",vec!["unclassified"])]
#[tokio::test]
async fn preview_and_actual_sampling_use_profile_precedence_and_tenant_identity(
    #[future(awt)] profiled_database: TestResult<SeededDatabase>,
    #[case] profile: &str,
    #[case] expected: Vec<&str>,
    #[values(0, 1)] preview: u8,
) -> TestResult {
    let fixture = profiled_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let rows = fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &parameters(profile, preview),
    )
    .await?;
    let actual: std::collections::BTreeSet<_> =
        rows.iter().map(|row| row.trace_id.as_str()).collect();
    let expected: std::collections::BTreeSet<_> = expected.into_iter().collect();
    assert_eq!(rows.len(), expected.len());
    assert_eq!(actual, expected);
    Ok(())
}

#[rstest]
#[case::matches("Coding", true)]
#[case::different("Workflows", false)]
#[case::unknown("Not recorded", false)]
#[case::recorded_id("recorded:Coding", true)]
#[case::other_id("recorded:Workflows", false)]
#[tokio::test]
async fn imported_finding_sources_must_belong_to_the_selected_profile(
    #[future(awt)] profiled_database: TestResult<SeededDatabase>,
    #[case] profile: &str,
    #[case] expected: bool,
) -> TestResult {
    let fixture = profiled_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let rows = fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &parameters("Coding", 1),
    )
    .await?;
    let coding = rows.iter().find(|row| row.trace_id == "coding").unwrap();
    let found = fetch::<LensFindingSource>(
        &fixture.database.client,
        config.storage().reader(),
        &LensFindingSourceParams {
            profile: profile.into(),
            access: access(),
            trace_id: "coding".into(),
            trace_ref: coding.trace_ref.clone(),
            agent_name: "assistant".into(),
            selected_team: String::new(),
            service: String::new(),
            filter_keys: Vec::new(),
            filter_values: Vec::new(),
        },
    )
    .await?;
    assert_eq!(!found.is_empty(), expected);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn inferred_profiles_filter_runs_without_overriding_recorded_labels_or_other_keys(
    #[future(awt)] profiled_database: TestResult<SeededDatabase>,
    #[values(0, 1)] preview: u8,
) -> TestResult {
    let fixture = profiled_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let body = concat!(
        "{\"TeamId\":\"team\",\"ApiKeyHash\":\"key\",\"TraceId\":\"unclassified\",\"Category\":\"Inferred coding\",\"StartTs\":\"2026-09-21 00:00:00\",\"ComputedAt\":1,\"ResolutionVersion\":1}\n",
        "{\"TeamId\":\"team\",\"ApiKeyHash\":\"key\",\"TraceId\":\"coding\",\"Category\":\"Inferred coding\",\"StartTs\":\"2026-09-21 00:00:00\",\"ComputedAt\":1,\"ResolutionVersion\":1}\n",
        "{\"TeamId\":\"team\",\"ApiKeyHash\":\"other-key\",\"TraceId\":\"workflows\",\"Category\":\"Inferred coding\",\"StartTs\":\"2026-09-21 00:00:00\",\"ComputedAt\":1,\"ResolutionVersion\":1}"
    );
    litellm_storage_clickhouse::insert_encoded_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        "lens_task_profiles",
        "inferred-profile-fixture",
        body,
    )
    .await?;
    let rows = fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &parameters("Inferred coding", preview),
    )
    .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trace_id, "unclassified");
    let found = fetch::<LensFindingSource>(
        &fixture.database.client,
        config.storage().reader(),
        &LensFindingSourceParams {
            profile: "Inferred coding".into(),
            access: access(),
            trace_id: "unclassified".into(),
            trace_ref: rows[0].trace_ref.clone(),
            agent_name: "assistant".into(),
            selected_team: String::new(),
            service: String::new(),
            filter_keys: Vec::new(),
            filter_values: Vec::new(),
        },
    )
    .await?;
    assert_eq!(found.len(), 1);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn background_projection_caches_recorded_profiles_without_loading_a_model(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![row(
            "recorded",
            "root",
            "",
            0,
            json!({}),
            json!({"task_category":"Workflow diagrams"}),
            ("team", "key"),
        )],
    )
    .await?;
    let cache = std::env::temp_dir().join(format!(
        "lens-profile-no-model-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut classifier = lens_profiles::Classifier::new(cache.clone());
    assert!(
        litellm_traces_clickhouse::profiles::project(
            &fixture.database.client,
            &config,
            &mut classifier,
        )
        .await?
    );
    assert!(
        !litellm_traces_clickhouse::profiles::project(
            &fixture.database.client,
            &config,
            &mut classifier,
        )
        .await?
    );
    assert!(!cache.exists());
    let rows = fetch::<LensSample>(
        &fixture.database.client,
        config.storage().reader(),
        &parameters("Workflow diagrams", 1),
    )
    .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trace_id, "recorded");
    Ok(())
}
