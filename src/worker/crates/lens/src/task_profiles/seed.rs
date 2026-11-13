use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::profiles::{
    Capability, DiscoveryPoint, DiscoveryRun, TaskDefinition, Taxonomy, TaxonomyModels,
    TaxonomyStatus,
};
use lens_server::profiles::taxonomy::{AgentState, Changes};
use litellm_storage_clickhouse::{execute_statement, state::ClickHouseState};
use litellm_traces::{Tenant, query::named::ReadAccessParams};
use litellm_traces_clickhouse::{
    Config, InsertTable, insert_rows, insert_shared_rows,
    profiles::{AssignmentOutcome, BucketingCandidate, TaskAssignment, record_assignment},
    span_rows,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Heads, load_state, save_state};
use crate::Error;

pub const TEAM: &str = "lens-profiles-demo";
const KEY: &str = "lens-profiles-demo-key";
const USER: &str = "lens-profiles-demo-user";
const RUNS_PER_AGENT: usize = 32;
const RUN_SPACING_MINUTES: i64 = 37;

struct Deployment {
    id: &'static str,
    group: &'static str,
}

const SONNET: Deployment = Deployment {
    id: "demo-claude-sonnet",
    group: "claude-sonnet-5-5",
};
const HAIKU: Deployment = Deployment {
    id: "demo-claude-haiku",
    group: "claude-haiku-5-5",
};

struct Task {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    capabilities: &'static [Capability],
    prompts: &'static [&'static str],
}

struct Agent {
    name: &'static str,
    service: &'static str,
    tasks: &'static [Task],
    recorded: Option<&'static str>,
    taxonomy: Option<TaxonomyStatus>,
}

const AGENTS: [Agent; 3] = [
    Agent {
        name: "lens-demo-research",
        service: "lens-demo-research-service",
        recorded: Some("Literature review"),
        taxonomy: Some(TaxonomyStatus::Frozen),
        tasks: &[
            Task {
                id: "demo-research-summarize",
                name: "Summarize sources",
                description: "Condense one or more documents into the points the user asked about.",
                capabilities: &[],
                prompts: &[
                    "Summarize the attached incident report in five bullet points",
                    "Give me the key findings of these three papers on retrieval",
                    "Condense this meeting transcript into decisions and owners",
                ],
            },
            Task {
                id: "demo-research-compare",
                name: "Compare options",
                description: "Weigh two or more alternatives against criteria and recommend one.",
                capabilities: &[Capability::Reasoning],
                prompts: &[
                    "Compare Postgres and ClickHouse for our event analytics workload",
                    "Which of these three vendors fits a 50 person team best and why",
                    "Weigh the tradeoffs of batching versus streaming for these jobs",
                ],
            },
            Task {
                id: "demo-research-chart",
                name: "Read charts and screenshots",
                description: "Answer questions about an attached chart, diagram or screenshot.",
                capabilities: &[Capability::Vision],
                prompts: &[
                    "What does this latency chart say about last week's deploy",
                    "Explain the architecture diagram in this screenshot",
                ],
            },
        ],
    },
    Agent {
        name: "lens-demo-coding",
        service: "lens-demo-coding-service",
        recorded: None,
        taxonomy: Some(TaxonomyStatus::Draft),
        tasks: &[
            Task {
                id: "demo-coding-debug",
                name: "Debug failing tests",
                description: "Find why a test fails and change code or the test until it passes.",
                capabilities: &[Capability::FunctionCalling],
                prompts: &[
                    "The login integration test fails on CI, find out why and fix it",
                    "Fix the flaky retry test in the payments module",
                    "This unit test started failing after the upgrade, debug it",
                ],
            },
            Task {
                id: "demo-coding-feature",
                name: "Implement a feature",
                description: "Add new behavior described by the user across one or more files.",
                capabilities: &[Capability::FunctionCalling],
                prompts: &[
                    "Add pagination to the users list endpoint",
                    "Implement CSV export for the invoices page",
                ],
            },
            Task {
                id: "demo-coding-compare",
                name: "Compare options",
                description: "Weigh libraries or designs for a coding decision.",
                capabilities: &[],
                prompts: &[
                    "Should we use tokio or async-std for this service",
                    "Compare these two caching designs for the API",
                ],
            },
            Task {
                id: "demo-coding-schema",
                name: "Generate structured config",
                description: "Produce machine-readable config or schema that must validate.",
                capabilities: &[Capability::StructuredOutput],
                prompts: &[
                    "Write a JSON schema for the webhook payload",
                    "Generate the Helm values file for staging",
                ],
            },
        ],
    },
    Agent {
        name: "lens-demo-support",
        service: "lens-demo-support-service",
        recorded: Some("Billing question"),
        taxonomy: None,
        tasks: &[
            Task {
                id: "demo-support-reset",
                name: "Account access",
                description: "",
                capabilities: &[],
                prompts: &[
                    "I can't log in after resetting my password",
                    "How do I enable two factor authentication on my account",
                ],
            },
            Task {
                id: "demo-support-bug",
                name: "Bug report",
                description: "",
                capabilities: &[],
                prompts: &[
                    "The export button does nothing when I click it",
                    "Dashboard charts show no data since this morning",
                ],
            },
        ],
    },
];

fn hex(seed: &str, characters: usize) -> String {
    format!("{:x}", Sha256::digest(seed.as_bytes()))[..characters].to_owned()
}

fn unit(seed: &str) -> f64 {
    let digest = Sha256::digest(seed.as_bytes());
    f64::from(u16::from_be_bytes([digest[0], digest[1]])) / 65_535.0
}

fn attributes(pairs: &[(&str, Value)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::Number(number) => json!({"intValue": number.to_string()}),
                    other => json!({"stringValue": other.as_str().unwrap_or_default()}),
                };
                json!({"key": key, "value": value})
            })
            .collect(),
    )
}

struct Span {
    trace_id: String,
    span_id: String,
    parent: String,
    name: String,
    start_ns: i64,
    duration_ns: i64,
    attributes: Value,
}

impl Span {
    fn json(&self) -> Value {
        json!({
            "traceId": self.trace_id, "spanId": self.span_id, "parentSpanId": self.parent,
            "name": self.name, "kind": 1,
            "startTimeUnixNano": self.start_ns.to_string(),
            "endTimeUnixNano": (self.start_ns + self.duration_ns).to_string(),
            "attributes": self.attributes, "status": {"code": 1}
        })
    }
}

fn resource(agent: &Agent, spans: &[Span]) -> Value {
    json!({
        "resource": {"attributes": attributes(&[
            ("service.name", json!(agent.service)),
            ("gen_ai.agent.name", json!(agent.name)),
        ])},
        "scopeSpans": [{"scope": {"name": "lens-profiles-demo"}, "spans": spans.iter().map(Span::json).collect::<Vec<_>>()}]
    })
}

#[derive(Default)]
struct Batch {
    resources: Vec<Value>,
    spend: Vec<serde_json::Map<String, Value>>,
    runs: Vec<SeededRun>,
}

struct SeededRun {
    agent: &'static str,
    trace_id: String,
    start_ms: i64,
    task_id: &'static str,
    recorded: bool,
    index: usize,
}

struct Call<'a> {
    trace_id: &'a str,
    parent: &'a str,
    seed: String,
    start_ms: i64,
    index: usize,
    deployment: &'a Deployment,
}

fn spend_row(
    call: &Call<'_>,
    request: &str,
    response: &str,
    litellm_call: &str,
    spend: Option<f64>,
    status: &str,
    model_id: &str,
) -> serde_json::Map<String, Value> {
    let row = json!({
        "request_id": request, "response_id": response, "litellm_call_id": litellm_call,
        "team_id": TEAM, "api_key": KEY, "user": USER,
        "model": format!("anthropic/{}", call.deployment.group), "model_group": call.deployment.group,
        "model_id": model_id, "custom_llm_provider": "anthropic",
        "spend": spend, "start_time": call.start_ms, "end_time": call.start_ms + 900,
        "status": status, "prompt_tokens": 1200, "completion_tokens": 300, "total_tokens": 1500,
    });
    row.as_object().cloned().unwrap_or_default()
}

/// One model call with its ledger rows. Variations cover the cases the view must not hide:
/// a retry whose first attempt failed but was charged, a missing cost, a call no ledger row
/// matches, a historical row without a deployment ID, and a call without a duration.
fn call(batch: &mut Batch, call: Call<'_>) -> Span {
    let span_id = hex(&format!("{}-span", call.seed), 16);
    let response = format!("resp-{}", hex(&call.seed, 20));
    let price = if call.deployment.id == SONNET.id {
        0.012
    } else {
        0.002
    };
    let spend = Some(price * (1.0 + (call.index % 5) as f64 * 0.15));
    let retry = call.index % 11 == 3;
    let unmatched = call.index % 17 == 7;
    let model_id = if call.index % 7 == 6 {
        ""
    } else {
        call.deployment.id
    };
    let duration_ns = if call.index % 19 == 2 {
        0
    } else {
        400_000_000 + (call.index % 9) as i64 * 350_000_000
    };
    let litellm_call = format!("call-{}", hex(&call.seed, 20));
    let mut pairs = vec![
        ("gen_ai.operation.name", json!("chat")),
        ("gen_ai.request.model", json!(call.deployment.group)),
        ("gen_ai.usage.input_tokens", json!(1200)),
        ("gen_ai.usage.output_tokens", json!(300)),
    ];
    if retry {
        pairs.push(("litellm.call_id", json!(litellm_call)));
        batch.spend.push(spend_row(
            &call,
            &format!("{litellm_call}-1"),
            "",
            &litellm_call,
            Some(price * 0.25),
            "failure",
            model_id,
        ));
        batch.spend.push(spend_row(
            &call,
            &format!("{litellm_call}-2"),
            "",
            &litellm_call,
            spend,
            "success",
            model_id,
        ));
    } else {
        pairs.push(("gen_ai.response.id", json!(response)));
        if !unmatched {
            let cost = if call.index % 13 == 5 { None } else { spend };
            batch.spend.push(spend_row(
                &call,
                &format!("req-{}", hex(&call.seed, 20)),
                &response,
                &litellm_call,
                cost,
                "success",
                model_id,
            ));
        }
    }
    Span {
        trace_id: call.trace_id.to_owned(),
        span_id,
        parent: call.parent.to_owned(),
        name: format!("chat {}", call.deployment.group),
        start_ns: call.start_ms * 1_000_000,
        duration_ns,
        attributes: attributes(&pairs),
    }
}

fn runs(batch: &mut Batch, now: DateTime<Utc>) {
    for (agent_index, agent) in AGENTS.iter().enumerate() {
        for index in 0..RUNS_PER_AGENT {
            let task = &agent.tasks[index % agent.tasks.len()];
            let prompt = task.prompts[(index / agent.tasks.len()) % task.prompts.len()];
            let recorded = agent.recorded.filter(|_| index % 8 == 5);
            let trace_id = hex(&format!("{}-{index}", agent.name), 32);
            let start_ms = now.timestamp_millis()
                - (index as i64 * RUN_SPACING_MINUTES + agent_index as i64 * 11 + 5) * 60_000;
            let root_id = hex(&format!("{trace_id}-root"), 16);
            let mut root_pairs = vec![
                ("gen_ai.operation.name", json!("invoke_agent")),
                ("gen_ai.agent.name", json!(agent.name)),
                ("gen_ai.prompt", json!(prompt)),
            ];
            if let Some(category) = recorded {
                root_pairs.push(("task_category", json!(category)));
            }
            let calls = 1 + index % 3;
            let mut spans = vec![Span {
                trace_id: trace_id.clone(),
                span_id: root_id.clone(),
                parent: String::new(),
                name: format!("invoke_agent {}", agent.name),
                start_ns: start_ms * 1_000_000,
                duration_ns: 30_000_000_000,
                attributes: attributes(&root_pairs),
            }];
            for position in 0..calls {
                let deployment = if (index + position) % 3 == 0 {
                    &SONNET
                } else {
                    &HAIKU
                };
                spans.push(call(
                    batch,
                    Call {
                        trace_id: &trace_id,
                        parent: &root_id,
                        seed: format!("{trace_id}-{position}"),
                        start_ms: start_ms + 1_000 + position as i64 * 4_000,
                        index: index + position,
                        deployment,
                    },
                ));
            }
            batch.resources.push(resource(agent, &spans));
            if agent.name == "lens-demo-research" && index % 6 == 0 {
                let coder = &AGENTS[1];
                let child = hex(&format!("{trace_id}-coder"), 16);
                let child_spans = vec![
                    Span {
                        trace_id: trace_id.clone(),
                        span_id: child.clone(),
                        parent: root_id.clone(),
                        name: format!("invoke_agent {}", coder.name),
                        start_ns: (start_ms + 15_000) * 1_000_000,
                        duration_ns: 10_000_000_000,
                        attributes: attributes(&[
                            ("gen_ai.operation.name", json!("invoke_agent")),
                            ("gen_ai.agent.name", json!(coder.name)),
                        ]),
                    },
                    call(
                        batch,
                        Call {
                            trace_id: &trace_id,
                            parent: &child,
                            seed: format!("{trace_id}-coder-call"),
                            start_ms: start_ms + 16_000,
                            index: index + 1,
                            deployment: &SONNET,
                        },
                    ),
                ];
                batch.resources.push(resource(coder, &child_spans));
            }
            batch.runs.push(SeededRun {
                agent: agent.name,
                trace_id,
                start_ms,
                task_id: task.id,
                recorded: recorded.is_some(),
                index,
            });
        }
    }
}

fn definition(task: &Task, examples: Vec<String>) -> TaskDefinition {
    TaskDefinition {
        task_id: task.id.into(),
        name: task.name.into(),
        description: task.description.into(),
        inclusion_criteria: format!("The request asks to {}", task.name.to_lowercase()),
        exclusion_criteria: String::new(),
        required_capabilities: task.capabilities.to_vec(),
        example_trace_ids: examples,
    }
}

fn taxonomy(
    agent: &Agent,
    status: TaxonomyStatus,
    runs: &[SeededRun],
    now: DateTime<Utc>,
) -> Taxonomy {
    let id = demo_taxonomy_id(agent.name);
    let own: Vec<&SeededRun> = runs
        .iter()
        .filter(|run| run.agent == agent.name && !run.recorded)
        .collect();
    let sampled: Vec<&&SeededRun> = own.iter().filter(|run| run.index < 24).collect();
    let center = |task_index: usize| {
        let angle = task_index as f64 * 2.1;
        (50.0 + 28.0 * angle.cos(), 50.0 + 28.0 * angle.sin())
    };
    let points: Vec<DiscoveryPoint> = sampled
        .iter()
        .map(|run| {
            let task_index = agent
                .tasks
                .iter()
                .position(|task| task.id == run.task_id)
                .unwrap_or_default();
            let (x, y) = center(task_index);
            DiscoveryPoint {
                team_id: TEAM.into(),
                api_key_hash: KEY.into(),
                trace_id: run.trace_id.clone(),
                task_id: Some(run.task_id.into()),
                x: x + (unit(&format!("{}x", run.trace_id)) - 0.5) * 12.0,
                y: y + (unit(&format!("{}y", run.trace_id)) - 0.5) * 12.0,
            }
        })
        .collect();
    let tasks = agent
        .tasks
        .iter()
        .map(|task| {
            definition(
                task,
                own.iter()
                    .filter(|run| run.task_id == task.id)
                    .take(3)
                    .map(|run| run.trace_id.clone())
                    .collect(),
            )
        })
        .collect();
    Taxonomy {
        taxonomy_id: id,
        agent: agent.name.into(),
        version: 1,
        revision: if status == TaxonomyStatus::Frozen {
            3
        } else {
            2
        },
        status,
        sample: true,
        tasks,
        job: None,
        discovered_at: Some(now - chrono::Duration::hours(3)),
        frozen_at: (status == TaxonomyStatus::Frozen).then(|| now - chrono::Duration::hours(2)),
        error: None,
        models: TaxonomyModels::default(),
        discovery: Some(DiscoveryRun {
            start_ms: (now - chrono::Duration::days(7)).timestamp_millis(),
            end_ms: (now - chrono::Duration::hours(3)).timestamp_millis(),
            requested_runs: 300,
            sampled_runs: u32::try_from(points.len()).unwrap_or(u32::MAX),
        }),
        points,
        windows: Vec::new(),
    }
}

async fn clear(client: &litellm_http::Client, config: &Config) -> Result<(), Error> {
    let database = config.storage().database();
    let tables = [
        ("otel_traces", "TeamId"),
        ("agent_traces_by_key", "TeamId"),
        ("lens_task_profiles", "TeamId"),
        ("lens_task_clusters", "TeamId"),
        ("lens_task_profile_assignments", "TeamId"),
        ("spend_logs", "team_id"),
        ("lens_call_costs", "team_id"),
    ];
    for (table, column) in tables {
        execute_statement(
            client,
            config.storage().writer(),
            &format!("ALTER TABLE `{database}`.{table} DELETE WHERE {column} = '{TEAM}' SETTINGS mutations_sync = 2"),
            Duration::from_secs(120),
        )
        .await?;
    }
    Ok(())
}

fn demo_taxonomy_id(agent: &str) -> String {
    lens_server::profiles::taxonomy_id(
        &ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
        },
        agent,
    )
    .unwrap_or_default()
}

/// Only absent or sample taxonomies may be replaced; anything else belongs to a real user.
fn replaceable(existing: &AgentState) -> bool {
    [&existing.working, &existing.active]
        .into_iter()
        .flatten()
        .all(|taxonomy| taxonomy.sample)
}

async fn guard(state: &ClickHouseState) -> Result<Vec<Heads>, Error> {
    let mut heads = Vec::with_capacity(AGENTS.len());
    for agent in &AGENTS {
        let (existing, snapshot) = load_state(state, &demo_taxonomy_id(agent.name))
            .await
            .map_err(|_| Error::Unavailable)?;
        if !replaceable(&existing) {
            return Err(Error::Configuration(
                "A demo agent already has a non-sample task taxonomy; the seed changed nothing",
            ));
        }
        heads.push(snapshot);
    }
    Ok(heads)
}

fn placeholder(agent: &str) -> Taxonomy {
    Taxonomy {
        taxonomy_id: demo_taxonomy_id(agent),
        agent: agent.into(),
        version: 0,
        revision: 1,
        status: TaxonomyStatus::Cleared,
        sample: true,
        tasks: Vec::new(),
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        models: TaxonomyModels::default(),
        discovery: None,
        windows: Vec::new(),
    }
}

/// Replaces the demo team's sample data with fresh timestamps. Only rows of the demo team and
/// the three demo agents' taxonomies are touched, and no model is called.
pub async fn seed(
    client: &litellm_http::Client,
    config: &Config,
    state: &ClickHouseState,
) -> Result<String, Error> {
    let now = Utc::now();
    let mut batch = Batch::default();
    runs(&mut batch, now);
    let snapshots = guard(state).await?;
    clear(client, config).await?;
    let payload = serde_json::to_vec(&json!({"resourceSpans": batch.resources}))?;
    let spans = litellm_traces::decode_otlp(&payload, Some("application/json"))
        .map_err(litellm_traces_clickhouse::Error::from)?;
    let tenant = Tenant {
        team_id: TEAM.into(),
        api_key_hash: KEY.into(),
        org_id: String::new(),
        user_id: USER.into(),
    };
    let database = config.storage().database();
    insert_shared_rows(
        client,
        config.storage().writer(),
        database,
        InsertTable::OtelTraces,
        span_rows(spans, &tenant),
    )
    .await?;
    insert_rows(
        client,
        config.storage().writer(),
        database,
        InsertTable::SpendLogs,
        batch
            .spend
            .into_iter()
            .map(|row| row.into_iter().collect())
            .collect(),
    )
    .await?;
    let mut assignments = 0;
    for (agent, heads) in AGENTS.iter().zip(snapshots) {
        let taxonomy = match agent.taxonomy {
            Some(status) => taxonomy(agent, status, &batch.runs, now),
            None => placeholder(agent.name),
        };
        let frozen = taxonomy.status == TaxonomyStatus::Frozen;
        let changes = Changes {
            working: Some(if frozen {
                placeholder(agent.name)
            } else {
                taxonomy.clone()
            }),
            active: Some(if frozen {
                taxonomy.clone()
            } else {
                placeholder(agent.name)
            }),
            settings: None,
        };
        save_state(state, heads, changes)
            .await
            .map_err(|_| Error::Unavailable)?;
        if agent.taxonomy.is_none() {
            continue;
        }
        for run in batch
            .runs
            .iter()
            .filter(|run| run.agent == agent.name && !run.recorded && run.index % 10 != 9)
        {
            let candidate = BucketingCandidate {
                team_id: TEAM.into(),
                api_key_hash: KEY.into(),
                trace_id: run.trace_id.clone(),
                user_ids: vec![USER.into()],
                start_ms: run.start_ms,
                input_preview: String::new(),
                attempts: 0,
            };
            let unknown = run.index % 9 == 4;
            record_assignment(
                client,
                config,
                &TaskAssignment {
                    candidate: &candidate,
                    agent: agent.name,
                    taxonomy_id: &taxonomy.taxonomy_id,
                    taxonomy_version: taxonomy.version,
                    task_id: if unknown { "" } else { run.task_id },
                    outcome: if unknown {
                        AssignmentOutcome::Unknown
                    } else {
                        AssignmentOutcome::Assigned
                    },
                },
            )
            .await?;
            assignments += 1;
        }
    }
    Ok(format!(
        "Task profile sample data seeded: {} runs across {} agents, {} sample assignments, no model calls",
        batch.runs.len(),
        AGENTS.len(),
        assignments
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::absent(None, true)]
    #[case::sample(Some(true), true)]
    #[case::real(Some(false), false)]
    fn the_seed_only_replaces_absent_or_sample_taxonomies(
        #[case] sample: Option<bool>,
        #[case] expected: bool,
        #[values(false, true)] active: bool,
    ) {
        let existing = sample.map(|sample| Taxonomy {
            sample,
            ..placeholder("lens-demo-research")
        });
        let state = if active {
            AgentState {
                active: existing,
                ..AgentState::default()
            }
        } else {
            AgentState {
                working: existing,
                ..AgentState::default()
            }
        };

        assert_eq!(replaceable(&state), expected);
    }
}
