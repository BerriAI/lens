use std::collections::{BTreeMap, BTreeSet};

use lens_contract::{
    activity::ActivitySelection,
    investigations::Scope,
    profiles::{
        ClassificationWindow, DiscoveryModelOptions, DiscoveryRequest, TaskDefinition, Taxonomy,
        TaxonomyModels, TaxonomyStatus,
    },
};
use lens_server::profiles::{
    ProfileStore, RunQuery,
    taxonomy::{Changes, DiscoveryCatalog, TaxonomyStore},
    taxonomy_id,
};
use litellm_lens::{SampleRequest, SourceReader};
use litellm_traces::{Tenant, decode_otlp, query::named::ReadAccessParams};
use litellm_traces_clickhouse::{
    InsertTable, insert_rows, insert_shared_rows,
    profiles::{AssignmentOutcome, BucketingCandidate, TaskAssignment, record_assignment},
    span_rows,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

mod profile_store;
use profile_store::Isolated;

const AGENT: &str = "research-agent";
const TEAM: &str = "research-team";
const KEY: &str = "research-key";
const USER: &str = "research-user";
/// Recorded with the same name as the reserved `unknown` profile id, which must not match it.
const RECORDED: &str = "unknown";
const RUNS: usize = 32;
const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * MINUTE_MS;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Recorded,
    Task,
    Unknown,
    Pending,
}

fn kind(index: usize) -> Kind {
    match index % 4 {
        _ if index == RUNS - 1 => Kind::Pending,
        0 => Kind::Recorded,
        1 => Kind::Unknown,
        _ => Kind::Task,
    }
}

fn trace_id(index: usize) -> String {
    format!("{:032x}", index + 1)
}

fn every_team() -> ReadAccessParams {
    ReadAccessParams {
        all_teams: true,
        user_id: String::new(),
        team_ids: Vec::new(),
    }
}

fn attributes(pairs: &[(&str, &str)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(key, value)| json!({"key": key, "value": {"stringValue": value}}))
            .collect(),
    )
}

fn run_spans(index: usize, start_ms: i64) -> [Value; 2] {
    let prompt = format!("Summarize paper {index}");
    let response = format!("resp-{index}");
    let root: Vec<(&str, &str)> = [
        ("gen_ai.operation.name", "invoke_agent"),
        ("gen_ai.agent.name", AGENT),
        ("gen_ai.prompt", prompt.as_str()),
    ]
    .into_iter()
    .chain((kind(index) == Kind::Recorded).then_some(("task_category", RECORDED)))
    .collect();
    let span = |span_id: String, parent: String, start_ms: i64, pairs: &[(&str, &str)]| {
        json!({
            "traceId": trace_id(index), "spanId": span_id, "parentSpanId": parent, "name": pairs[0].1,
            "kind": 1, "startTimeUnixNano": (start_ms * 1_000_000).to_string(),
            "endTimeUnixNano": ((start_ms + 30_000) * 1_000_000).to_string(),
            "attributes": attributes(pairs), "status": {"code": 1}
        })
    };
    let root_id = format!("{:016x}", 2 * index + 1);
    [
        span(root_id.clone(), String::new(), start_ms, &root),
        span(
            format!("{:016x}", 2 * index + 2),
            root_id,
            start_ms + 1_000,
            &[
                ("gen_ai.operation.name", "chat"),
                ("gen_ai.request.model", "sonnet"),
                ("gen_ai.response.id", response.as_str()),
            ],
        ),
    ]
}

fn ledger(index: usize, start_ms: i64) -> BTreeMap<String, Value> {
    json!({
        "request_id": format!("req-{index}"), "response_id": format!("resp-{index}"),
        "litellm_call_id": format!("call-{index}"), "team_id": TEAM, "api_key": KEY, "user": USER,
        "model": "anthropic/sonnet", "model_group": "sonnet", "model_id": "dep-sonnet",
        "custom_llm_provider": "anthropic", "spend": 0.01,
        "start_time": start_ms + 1_000, "end_time": start_ms + 2_000, "status": "success",
        "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .collect()
}

fn summarize() -> TaskDefinition {
    TaskDefinition {
        task_id: "summarize".into(),
        name: "Summarize a paper".into(),
        description: String::new(),
        inclusion_criteria: String::new(),
        exclusion_criteria: String::new(),
        required_capabilities: Vec::new(),
        example_trace_ids: Vec::new(),
    }
}

struct Reviewed {
    isolated: Isolated,
    start_ms: i64,
    active: Taxonomy,
}

impl Reviewed {
    fn traces(&self, wanted: Kind) -> BTreeSet<String> {
        (0..RUNS)
            .filter(|index| kind(*index) == wanted)
            .map(trace_id)
            .collect()
    }

    fn query(&self) -> RunQuery {
        RunQuery {
            agent: AGENT.into(),
            start_ms: self.start_ms - DAY_MS,
            end_ms: self.start_ms + DAY_MS,
            taxonomy_id: self.active.taxonomy_id.clone(),
            taxonomy_version: self.active.version,
            windows: self.active.windows.clone(),
            limit: 5_001,
        }
    }

    async fn sampled(&self, scope: &Scope, profile: &str) -> BTreeSet<String> {
        let selection = ActivitySelection {
            agent_name: AGENT.into(),
            profile: Some(profile.try_into().expect("profile")),
            ..Default::default()
        };
        SourceReader(self.isolated.state.clone())
            .sample(
                scope,
                SampleRequest {
                    selection: &selection,
                    start: u64::try_from(self.start_ms - DAY_MS).expect("start"),
                    end: u64::try_from(self.start_ms + 3 * DAY_MS).expect("end"),
                    offset: 0,
                    page_size: 500,
                    preview: false,
                    cursor: "",
                },
            )
            .await
            .expect("sample")
            .executions
            .into_iter()
            .map(|execution| execution.trace_id)
            .collect()
    }
}

/// Runs ingested through OTLP and the spend ledger, then reviewed under an active taxonomy whose
/// classifications were recorded the way the worker records them; the last run is still pending.
#[fixture]
async fn reviewed() -> Reviewed {
    let isolated = profile_store::isolated().await;
    let start_ms = chrono::Utc::now().timestamp_millis() - 2 * DAY_MS;
    let run_start = |index: usize| start_ms + i64::try_from(index).expect("index") * 10 * MINUTE_MS;
    let payload = json!({"resourceSpans": [{
        "resource": {"attributes": attributes(&[("service.name", "research"), ("gen_ai.agent.name", AGENT)])},
        "scopeSpans": [{"scope": {"name": "profile-tasks-test"},
            "spans": (0..RUNS).flat_map(|index| run_spans(index, run_start(index))).collect::<Vec<_>>()}]
    }]});
    let spans = decode_otlp(
        &serde_json::to_vec(&payload).expect("payload"),
        Some("application/json"),
    )
    .expect("otlp");
    let tenant = Tenant {
        team_id: TEAM.into(),
        api_key_hash: KEY.into(),
        org_id: String::new(),
        user_id: USER.into(),
    };
    let writer = isolated.config.storage().writer();
    let database = isolated.config.storage().database();
    insert_shared_rows(
        &isolated.client,
        writer,
        database,
        InsertTable::OtelTraces,
        span_rows(spans, &tenant),
    )
    .await
    .expect("spans");
    insert_rows(
        &isolated.client,
        writer,
        database,
        InsertTable::SpendLogs,
        (0..RUNS)
            .map(|index| ledger(index, run_start(index)))
            .collect(),
    )
    .await
    .expect("ledger");
    let id = taxonomy_id(&every_team(), AGENT).expect("id");
    let active = Taxonomy {
        taxonomy_id: id.clone(),
        agent: AGENT.into(),
        version: 1,
        revision: 1,
        status: TaxonomyStatus::Frozen,
        tasks: vec![summarize()],
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        models: TaxonomyModels::default(),
        discovery: None,
        windows: vec![ClassificationWindow {
            start_ms: start_ms - MINUTE_MS,
            end_ms: None,
        }],
    };
    let (_, heads) = isolated.profiles.load(&id).await.expect("load");
    isolated
        .profiles
        .save(
            &id,
            heads,
            Changes {
                active: Some(active.clone()),
                ..Changes::default()
            },
        )
        .await
        .expect("active");
    for (index, outcome, task_id) in (0..RUNS).filter_map(|index| match kind(index) {
        Kind::Task => Some((index, AssignmentOutcome::Assigned, "summarize")),
        Kind::Unknown => Some((index, AssignmentOutcome::Unknown, "")),
        Kind::Recorded | Kind::Pending => None,
    }) {
        record_assignment(
            &isolated.client,
            &isolated.config,
            &TaskAssignment {
                candidate: &BucketingCandidate {
                    team_id: TEAM.into(),
                    api_key_hash: KEY.into(),
                    trace_id: trace_id(index),
                    user_ids: vec![USER.into()],
                    start_ms: run_start(index),
                    input_preview: String::new(),
                    attempts: 0,
                },
                agent: AGENT,
                taxonomy_id: &id,
                taxonomy_version: 1,
                task_id,
                outcome,
            },
        )
        .await
        .expect("assignment");
    }
    Reviewed {
        isolated,
        start_ms,
        active,
    }
}

#[rstest]
#[tokio::test]
async fn runs_resolve_to_their_recorded_category_assigned_task_or_classification_state(
    #[future(awt)] reviewed: Reviewed,
) {
    let profiles = &reviewed.isolated.profiles;
    let runs = profiles
        .runs(every_team(), reviewed.query())
        .await
        .expect("runs");
    let calls = profiles
        .calls(every_team(), runs.clone(), 50_000)
        .await
        .expect("calls");
    let resolved = |profile: &str| -> BTreeSet<String> {
        runs.iter()
            .filter(|run| run.profile_id == profile)
            .map(|run| run.trace_id.clone())
            .collect()
    };
    let priced = calls
        .values()
        .flatten()
        .flat_map(|call| &call.charges)
        .filter(|charge| charge.spend.is_some())
        .count();

    assert_eq!(runs.len(), RUNS);
    assert_eq!(
        resolved(&format!("recorded:{RECORDED}")),
        reviewed.traces(Kind::Recorded)
    );
    assert_eq!(resolved("task:summarize"), reviewed.traces(Kind::Task));
    assert_eq!(resolved("unknown"), reviewed.traces(Kind::Unknown));
    assert_eq!(resolved("pending"), reviewed.traces(Kind::Pending));
    assert_eq!(priced, RUNS);
}

#[rstest]
#[tokio::test]
async fn a_preview_counts_runs_without_recorded_categories_and_shows_inputs_without_a_gateway(
    #[future(awt)] reviewed: Reviewed,
) {
    let profiles = &reviewed.isolated.profiles;

    let preview = profiles
        .preview(DiscoveryRequest {
            agent: AGENT.into(),
            start_ms: reviewed.start_ms - DAY_MS,
            end_ms: reviewed.start_ms + DAY_MS,
            max_runs: 20,
        })
        .await
        .expect("preview");

    let unrecorded = RUNS - reviewed.traces(Kind::Recorded).len();
    assert_eq!(
        preview.eligible_runs,
        u64::try_from(unrecorded).expect("count")
    );
    assert_eq!(preview.sample_runs, 20);
    assert_eq!(preview.previews.len(), 3);
    assert!(
        preview
            .previews
            .iter()
            .all(|run| run.input_preview.starts_with("Summarize paper"))
    );
    assert_eq!(profiles.options().await, DiscoveryModelOptions::default());
}

#[rstest]
#[tokio::test]
async fn investigating_a_reviewed_task_samples_exactly_its_runs_for_every_team_callers(
    #[future(awt)] reviewed: Reviewed,
) {
    let every_team = Scope {
        team_id: String::new(),
        api_key_hash: String::new(),
        all_teams: true,
    };

    let sample = reviewed.sampled(&every_team, "task:summarize").await;

    assert_eq!(sample, reviewed.traces(Kind::Task));
}

#[rstest]
#[tokio::test]
async fn scoped_callers_cannot_investigate_by_an_enhanced_task_but_keep_recorded_categories(
    #[future(awt)] reviewed: Reviewed,
) {
    let team = Scope {
        team_id: TEAM.into(),
        api_key_hash: String::new(),
        all_teams: false,
    };

    let task = reviewed.sampled(&team, "task:summarize").await;
    let recorded = reviewed
        .sampled(&team, &format!("recorded:{RECORDED}"))
        .await;

    assert_eq!(task, BTreeSet::new());
    assert_eq!(recorded, reviewed.traces(Kind::Recorded));
}

#[rstest]
#[tokio::test]
async fn investigating_the_unknown_profile_skips_a_recorded_category_with_the_same_name(
    #[future(awt)] reviewed: Reviewed,
) {
    let every_team = Scope {
        team_id: String::new(),
        api_key_hash: String::new(),
        all_teams: true,
    };

    let unknown = reviewed.sampled(&every_team, "unknown").await;
    let recorded = reviewed
        .sampled(&every_team, &format!("recorded:{RECORDED}"))
        .await;

    assert_eq!(unknown, reviewed.traces(Kind::Unknown));
    assert_eq!(recorded, reviewed.traces(Kind::Recorded));
}
