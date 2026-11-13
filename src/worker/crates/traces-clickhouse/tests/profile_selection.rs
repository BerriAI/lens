use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::{
    Config, InsertTable, insert_rows,
    profiles::{
        AssignmentOutcome, BucketingParams, TaskAssignment, bucketing_candidates,
        discovery_eligible, discovery_samples, record_assignment,
    },
    query::profiles::{ProfileRuns, ProfileRunsParams},
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "queries/support.rs"]
mod fixtures;
mod support;
use fixtures::{DATABASE, SeededDatabase, migrated_database};
use support::TestResult;

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

/// Assignments expire with the run's start, so fixture runs must stay inside the 7-day retention
/// whenever the suite runs.
static START_MS: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64;
    now / MINUTE_MS * MINUTE_MS - 6 * 60 * MINUTE_MS
});

fn span(
    trace: &str,
    minute: i64,
    input: &str,
    attributes: Value,
    tenant: (&str, &str),
    agent: &str,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "Timestamp".into(),
            json!((*START_MS + minute * MINUTE_MS) * 1_000_000),
        ),
        ("TraceId".into(), json!(trace)),
        ("SpanId".into(), json!(format!("{trace}-root"))),
        ("ParentSpanId".into(), json!("")),
        ("SpanName".into(), json!("agent")),
        ("ObservationType".into(), json!("agent")),
        ("AgentName".into(), json!(agent)),
        ("TeamId".into(), json!(tenant.0)),
        ("ApiKeyHash".into(), json!(tenant.1)),
        ("Duration".into(), json!(1_000_000)),
        ("Input".into(), json!(input)),
        ("ResourceAttributes".into(), json!({})),
        ("SpanAttributes".into(), attributes),
    ])
}

#[fixture]
async fn runs(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult<SeededDatabase> {
    let fixture = migrated_database?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let team = ("team", "key");
    insert_rows(
        &fixture.database.client,
        config.storage().writer(),
        DATABASE,
        InsertTable::OtelTraces,
        vec![
            span(
                "first",
                0,
                "Fix the login bug",
                json!({}),
                team,
                "assistant",
            ),
            span(
                "second",
                10,
                "Write release notes",
                json!({}),
                team,
                "assistant",
            ),
            span(
                "third",
                20,
                "Summarize this PR",
                json!({}),
                team,
                "assistant",
            ),
            span(
                "first",
                30,
                "Same trace id, other team",
                json!({}),
                ("other", "key"),
                "assistant",
            ),
            span(
                "recorded",
                5,
                "Billing question",
                json!({"task_category": "Billing"}),
                team,
                "assistant",
            ),
            span("silent", 15, "", json!({}), team, "assistant"),
            span(
                "elsewhere",
                25,
                "Not this agent",
                json!({}),
                team,
                "planner",
            ),
            span(
                "expiring",
                -(7 * 24 * 60) + 6 * 60 + 30,
                "Older than the retention allows",
                json!({}),
                team,
                "assistant",
            ),
        ],
    )
    .await?;
    Ok(fixture)
}

fn whole_hour() -> (i64, i64) {
    (*START_MS, *START_MS + 60 * MINUTE_MS)
}

fn traces(rows: &[litellm_traces_clickhouse::profiles::BucketingCandidate]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{}/{}", row.team_id, row.trace_id))
        .collect()
}

#[rstest]
#[case::capped(2, vec!["other/first", "team/third"])]
#[case::everything(500, vec!["other/first", "team/third", "team/second", "team/first"])]
#[tokio::test]
async fn the_preview_count_and_the_discovery_sample_select_the_same_runs(
    #[future(awt)] runs: TestResult<SeededDatabase>,
    #[case] limit: u32,
    #[case] expected: Vec<&str>,
) -> TestResult {
    let fixture = runs?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let params = BucketingParams::windows("assistant", &[whole_hour()], limit);

    let eligible = discovery_eligible(&fixture.database.client, &config, &params).await?;
    let sample = discovery_samples(&fixture.database.client, &config, &params).await?;

    assert_eq!(eligible, 4);
    assert_eq!(traces(&sample), expected);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn windows_select_runs_by_when_they_started(
    #[future(awt)] runs: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = runs?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let windows = [
        (*START_MS, *START_MS + 5 * MINUTE_MS),
        (*START_MS + 15 * MINUTE_MS, *START_MS + 25 * MINUTE_MS),
    ];

    let sample = discovery_samples(
        &fixture.database.client,
        &config,
        &BucketingParams::windows("assistant", &windows, 500),
    )
    .await?;

    assert_eq!(traces(&sample), vec!["team/third", "team/first"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn classification_skips_runs_its_version_settled_while_rediscovery_still_samples_them(
    #[future(awt)] runs: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = runs?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let classify = |version: u32| BucketingParams {
        taxonomy_id: "tax".into(),
        taxonomy_version: version,
        require_input: 0,
        max_attempts: 3,
        ..BucketingParams::windows("assistant", &[whole_hour()], 20)
    };
    let before = bucketing_candidates(&fixture.database.client, &config, &classify(1)).await?;
    let third = before
        .iter()
        .find(|row| row.trace_id == "third")
        .expect("third run")
        .clone();
    record_assignment(
        &fixture.database.client,
        &config,
        &TaskAssignment {
            candidate: &third,
            agent: "assistant",
            taxonomy_id: "tax",
            taxonomy_version: 1,
            task_id: "notes",
            outcome: AssignmentOutcome::Assigned,
        },
    )
    .await?;

    let settled = bucketing_candidates(&fixture.database.client, &config, &classify(1)).await?;
    let next_version =
        bucketing_candidates(&fixture.database.client, &config, &classify(2)).await?;
    let rediscovery = discovery_eligible(
        &fixture.database.client,
        &config,
        &BucketingParams::windows("assistant", &[whole_hour()], 500),
    )
    .await?;

    assert_eq!(
        traces(&before),
        vec![
            "other/first",
            "team/third",
            "team/silent",
            "team/second",
            "team/first"
        ]
    );
    assert_eq!(
        traces(&settled),
        vec!["other/first", "team/silent", "team/second", "team/first"]
    );
    assert_eq!(traces(&next_version), traces(&before));
    assert_eq!(rediscovery, 4);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn only_unassigned_runs_inside_a_window_are_pending(
    #[future(awt)] runs: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = runs?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let rows = fetch::<ProfileRuns>(
        &fixture.database.client,
        config.storage().reader(),
        &ProfileRunsParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
            agent: "assistant".into(),
            start_ms: *START_MS,
            end_ms: *START_MS + 60 * MINUTE_MS,
            taxonomy_id: "tax".into(),
            taxonomy_version: 1,
            window_starts: vec![*START_MS + 15 * MINUTE_MS],
            window_ends: vec![i64::MAX],
            limit: 100,
        },
    )
    .await?;
    let profile = |team: &str, trace: &str| {
        rows.iter()
            .find(|row| row.team_id == team && row.trace_id == trace)
            .map(|row| row.profile_id.clone())
    };

    assert_eq!(profile("team", "third").as_deref(), Some("pending"));
    assert_eq!(profile("other", "first").as_deref(), Some("pending"));
    assert_eq!(profile("team", "second").as_deref(), Some("unclassified"));
    assert_eq!(
        profile("team", "recorded").as_deref(),
        Some("recorded:Billing")
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn runs_at_the_edge_of_retention_are_never_sampled_or_classified(
    #[future(awt)] runs: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = runs?;
    let config = Config::new(DATABASE.into(), &fixture.database.url, 7)?;
    let since_last_week = [(*START_MS - 8 * DAY_MS, *START_MS + 60 * MINUTE_MS)];
    let discovery = BucketingParams::windows("assistant", &since_last_week, 500);
    let classify = BucketingParams {
        taxonomy_id: "tax".into(),
        taxonomy_version: 1,
        require_input: 0,
        max_attempts: 3,
        ..BucketingParams::windows("assistant", &since_last_week, 20)
    };

    let eligible = discovery_eligible(&fixture.database.client, &config, &discovery).await?;
    let sample = discovery_samples(&fixture.database.client, &config, &discovery).await?;
    let candidates = bucketing_candidates(&fixture.database.client, &config, &classify).await?;

    assert_eq!(eligible, 4);
    assert!(!traces(&sample).contains(&"team/expiring".to_owned()));
    assert!(!traces(&candidates).contains(&"team/expiring".to_owned()));
    Ok(())
}
