mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use lens_contract::{
    auth::{Identity, Role},
    profiles::{
        Capability, ClassificationWindow, DiscoveryModelChoice, DiscoveryModelOptions,
        DiscoveryPreview, DiscoveryPreviewRun, DiscoveryRequest, DiscoveryRun, ModelCapabilities,
        ProfileSettings, TaskDefinition, Taxonomy, TaxonomyModels, TaxonomyStatus,
    },
};
use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Call, Charge, Column, Run, RunCalls, run_key},
    taxonomy::{AgentState, Changes, DiscoveryCatalog, TaxonomyStore},
};
use litellm_traces::{LedgerRequest, query::named::ReadAccessParams};
use reqwest::{Method, StatusCode};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use support::{Database, Server, database};

const AGENT: &str = "assistant";

#[derive(Default)]
struct Shared {
    states: HashMap<String, (AgentState, u64)>,
    queries: Vec<RunQuery>,
    previews: Vec<DiscoveryRequest>,
}

#[derive(Clone, Default)]
struct Memory {
    shared: Arc<Mutex<Shared>>,
    runs: Vec<Run>,
    calls: RunCalls,
    unavailable: bool,
}

impl Memory {
    fn with(state: AgentState) -> Self {
        let memory = Self::default();
        memory.put(state);
        memory
    }

    fn put(&self, state: AgentState) {
        let id = lens_server::profiles::taxonomy_id(&every_team(), AGENT).expect("id");
        self.lock().states.insert(id, (state, 1));
    }

    fn state(&self) -> AgentState {
        self.lock()
            .states
            .values()
            .next()
            .map(|(state, _)| state.clone())
            .unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().expect("store")
    }
}

impl ProfileStore for Memory {
    async fn runs(
        &self,
        _: ReadAccessParams,
        query: RunQuery,
    ) -> Result<Vec<Run>, ProfileStoreError> {
        self.lock().queries.push(query);
        if self.unavailable {
            return Err(ProfileStoreError::Unavailable);
        }
        Ok(self.runs.clone())
    }

    async fn calls(
        &self,
        _: ReadAccessParams,
        _: Vec<Run>,
        _: usize,
    ) -> Result<RunCalls, ProfileStoreError> {
        Ok(self.calls.clone())
    }

    async fn configured_models(&self) -> Vec<ConfiguredModel> {
        vec![ConfiguredModel {
            model_id: "dep-text".into(),
            model_group: "text-only".into(),
            provider: "openai".into(),
            capabilities: ModelCapabilities {
                function_calling: Some(true),
                vision: Some(false),
                reasoning: None,
                structured_output: None,
            },
        }]
    }
}

impl TaxonomyStore for Memory {
    type Heads = u64;

    async fn load(&self, taxonomy_id: &str) -> Result<(AgentState, u64), ProfileStoreError> {
        Ok(self
            .lock()
            .states
            .get(taxonomy_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save(
        &self,
        taxonomy_id: &str,
        heads: u64,
        changes: Changes,
    ) -> Result<(), ProfileStoreError> {
        let mut shared = self.lock();
        let (state, version) = shared.states.entry(taxonomy_id.to_owned()).or_default();
        if *version != heads {
            return Err(ProfileStoreError::Conflict);
        }
        *state = AgentState {
            working: changes.working.or(state.working.take()),
            active: changes.active.or(state.active.take()),
            settings: changes.settings.or(state.settings.take()),
        };
        *version += 1;
        Ok(())
    }
}

impl DiscoveryCatalog for Memory {
    async fn options(&self) -> DiscoveryModelOptions {
        DiscoveryModelOptions {
            discovery: vec!["haiku".into()],
            embedding: vec!["qwen".into()],
            classification: vec!["jev".into()],
        }
    }

    async fn preview(
        &self,
        request: DiscoveryRequest,
    ) -> Result<DiscoveryPreview, ProfileStoreError> {
        self.lock().previews.push(request.clone());
        Ok(DiscoveryPreview {
            eligible_runs: 42,
            sample_runs: request.max_runs.min(42),
            previews: vec![DiscoveryPreviewRun {
                trace_id: "t1".into(),
                start_ms: 5,
                input_preview: "Why was I charged twice?".into(),
            }],
        })
    }
}

fn every_team() -> ReadAccessParams {
    ReadAccessParams {
        all_teams: true,
        user_id: String::new(),
        team_ids: Vec::new(),
    }
}

fn choice() -> DiscoveryModelChoice {
    DiscoveryModelChoice {
        discovery: "haiku".into(),
        embedding: "qwen".into(),
        classification: "jev".into(),
    }
}

fn settings(enabled: bool) -> ProfileSettings {
    ProfileSettings {
        agent: AGENT.into(),
        enabled,
        models: choice(),
        revision: 2,
        updated_at: None,
    }
}

fn billing() -> TaskDefinition {
    TaskDefinition {
        task_id: "billing".into(),
        name: "Answer billing questions".into(),
        description: "Questions about invoices".into(),
        inclusion_criteria: String::new(),
        exclusion_criteria: String::new(),
        required_capabilities: vec![Capability::Vision],
        example_trace_ids: Vec::new(),
    }
}

fn taxonomy(status: TaxonomyStatus, windows: Vec<ClassificationWindow>) -> Taxonomy {
    Taxonomy {
        taxonomy_id: "tax".into(),
        agent: AGENT.into(),
        version: 2,
        revision: 5,
        status,
        sample: false,
        tasks: vec![billing()],
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        models: TaxonomyModels {
            distill: "haiku".into(),
            embedding: "qwen".into(),
            synthesis: "haiku".into(),
            decision: "jev".into(),
        },
        discovery: Some(DiscoveryRun {
            start_ms: 1_000,
            end_ms: 9_000,
            requested_runs: 300,
            sampled_runs: 40,
        }),
        windows,
    }
}

fn open() -> Vec<ClassificationWindow> {
    vec![ClassificationWindow {
        start_ms: 100,
        end_ms: None,
    }]
}

fn run(trace: &str, profile_id: &str, category: &str) -> Run {
    Run {
        team_id: "team".into(),
        api_key_hash: "key".into(),
        trace_id: trace.into(),
        start_ms: 10,
        profile_id: profile_id.into(),
        category: category.into(),
        input_preview: format!("request {trace}"),
        agent_count: 1,
    }
}

fn traffic(state: AgentState) -> Memory {
    let billed = run("t1", "task:billing", "");
    let calls = RunCalls::from([(
        run_key(&billed),
        vec![Call {
            span_id: "s1".into(),
            agent: AGENT.into(),
            duration_ms: 1_200.0,
            charges: vec![Charge {
                identity: LedgerRequest {
                    team_id: "team".into(),
                    start_ms: 10,
                    request_id: "r1".into(),
                },
                spend: Some(0.25),
                column: Column::Deployment {
                    model_id: "dep-vision".into(),
                    model_group: "sonnet".into(),
                    provider: "anthropic".into(),
                },
            }],
            fallback: Column::Unmatched {
                model: "sonnet".into(),
            },
        }],
    )]);
    Memory {
        runs: vec![
            billed,
            run("t2", "recorded:Refunds", "Refunds"),
            run("t3", "cluster:c1", "login help"),
        ],
        calls,
        ..Memory::with(state)
    }
}

fn token(role: Role) -> String {
    identity::delegated(Identity {
        user_role: role,
        user_id: Some("caller".into()),
        log_team_ids: vec!["team".into()],
        ..Identity::default()
    })
}

async fn serve(database: &Database, store: Memory) -> Server {
    database
        .serve_router(false, |auth| {
            lens_server::profiles::router(
                auth,
                store,
                DiscoveryModelChoice {
                    discovery: "haiku".into(),
                    ..DiscoveryModelChoice::default()
                },
            )
        })
        .await
}

async fn call(
    server: &Server,
    method: Method,
    path: &str,
    role: Option<Role>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = server
        .client
        .request(method, server.url.join(path).expect("url"));
    let request = match role {
        Some(role) => request.bearer_auth(token(role)),
        None => request,
    };
    let request = match body {
        Some(body) => request.json(&body),
        None => request,
    };
    let response = request.send().await.expect("response");
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

fn window_query(extra: &str) -> String {
    format!("?agent={AGENT}&start_ms=0&end_ms=100{extra}")
}

#[fixture]
fn admin() -> Option<Role> {
    Some(Role::ProxyAdmin)
}

#[rstest]
#[case::anonymous(None, StatusCode::UNAUTHORIZED)]
#[case::scoped_user(Some(Role::InternalUser), StatusCode::FORBIDDEN)]
#[case::read_only_admin(Some(Role::ProxyAdminViewer), StatusCode::FORBIDDEN)]
#[tokio::test]
async fn only_every_team_admins_can_change_discovery_settings(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] expected: StatusCode,
) {
    let store = Memory::default();
    let server = serve(&database, store.clone()).await;

    let (status, _) = call(
        &server,
        Method::PUT,
        "/lens/profiles/settings",
        role,
        Some(json!({"agent": AGENT, "revision": 0, "enabled": true, "models": choice()})),
    )
    .await;

    assert_eq!(status, expected);
    assert_eq!(store.state(), AgentState::default());
}

#[rstest]
#[case::admin(Role::ProxyAdmin, StatusCode::OK, json!(true))]
#[case::read_only_admin(Role::ProxyAdminViewer, StatusCode::OK, json!(false))]
#[case::scoped_user(Role::InternalUser, StatusCode::FORBIDDEN, Value::Null)]
#[tokio::test]
async fn settings_start_off_and_show_deployment_defaults_to_every_team_admins(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] expected: StatusCode,
    #[case] can_write: Value,
) {
    let server = serve(&database, Memory::default()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/settings?agent={AGENT}"),
        Some(role),
        None,
    )
    .await;

    assert_eq!(status, expected);
    if status == StatusCode::OK {
        assert_eq!(body["settings"]["enabled"], json!(false));
        assert_eq!(body["settings"]["agent"], json!(AGENT));
        assert_eq!(body["defaults"]["discovery"], json!("haiku"));
        assert_eq!(body["options"]["classification"], json!(["jev"]));
        assert_eq!(body["can_write"], can_write);
    }
}

#[rstest]
#[case::chat_model_as_classifier(json!({"discovery": "haiku", "embedding": "qwen", "classification": "haiku"}), "The classification model is not an available Jev decisions model on the gateway")]
#[case::unknown_embedding(json!({"discovery": "haiku", "embedding": "gone", "classification": "jev"}), "The embedding model is not an available embedding model on the gateway")]
#[case::missing_discovery_model(json!({"discovery": "", "embedding": "qwen", "classification": "jev"}), "Choose a model for each step before turning on task discovery")]
#[tokio::test]
async fn opting_in_rejects_models_the_gateway_does_not_offer_for_their_role(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] models: Value,
    #[case] detail: &str,
) {
    let store = Memory::default();
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::PUT,
        "/lens/profiles/settings",
        admin,
        Some(json!({"agent": AGENT, "revision": 0, "enabled": true, "models": models})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], json!(detail));
    assert_eq!(store.state().settings, None);
}

#[rstest]
#[tokio::test]
async fn saving_settings_persists_them_and_rejects_a_stale_form(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = Memory::default();
    let server = serve(&database, store.clone()).await;
    let save = |revision: u32| json!({"agent": AGENT, "revision": revision, "enabled": true, "models": choice()});

    let (saved, body) = call(
        &server,
        Method::PUT,
        "/lens/profiles/settings",
        admin,
        Some(save(0)),
    )
    .await;
    let (stale, _) = call(
        &server,
        Method::PUT,
        "/lens/profiles/settings",
        admin,
        Some(save(0)),
    )
    .await;
    let (_, reloaded) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/settings?agent={AGENT}"),
        admin,
        None,
    )
    .await;

    assert_eq!(saved, StatusCode::OK);
    assert_eq!(body["revision"], json!(1));
    assert_eq!(stale, StatusCode::CONFLICT);
    assert_eq!(reloaded["settings"]["enabled"], json!(true));
    assert_eq!(reloaded["settings"]["models"], json!(choice()));
}

#[rstest]
#[tokio::test]
async fn turning_discovery_off_closes_classification_even_with_models_the_gateway_dropped(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = Memory::with(AgentState {
        active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
        settings: Some(settings(true)),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::PUT,
        "/lens/profiles/settings",
        admin,
        Some(json!({"agent": AGENT, "revision": 2, "enabled": false, "models": {"discovery": "retired", "embedding": "", "classification": ""}})),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], json!(false));
    assert!(
        store.state().active.expect("active").windows[0]
            .end_ms
            .is_some()
    );
}

#[rstest]
#[case::read_only_admin(Role::ProxyAdminViewer, StatusCode::OK)]
#[case::scoped_user(Role::InternalUser, StatusCode::FORBIDDEN)]
#[tokio::test]
async fn previews_are_read_only_and_limited_to_every_team_admins(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] expected: StatusCode,
) {
    let store = Memory::default();
    let server = serve(&database, store.clone()).await;
    let request = json!({"agent": AGENT, "start_ms": 0, "end_ms": 10, "max_runs": 300});

    let (status, body) = call(
        &server,
        Method::POST,
        "/lens/profiles/discovery/preview",
        Some(role),
        Some(request),
    )
    .await;

    assert_eq!(status, expected);
    assert_eq!(
        store.lock().previews.len(),
        usize::from(expected == StatusCode::OK)
    );
    if expected == StatusCode::OK {
        assert_eq!(body["sample_runs"], json!(42));
        assert_eq!(store.state(), AgentState::default());
    }
}

#[rstest]
#[case::too_small(json!({"agent": AGENT, "start_ms": 0, "end_ms": 10, "max_runs": 19}))]
#[case::empty_range(json!({"agent": AGENT, "start_ms": 10, "end_ms": 10, "max_runs": 100}))]
#[tokio::test]
async fn previews_reject_an_unusable_range_or_size(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] request: Value,
) {
    let store = Memory::default();
    let server = serve(&database, store.clone()).await;

    let (status, _) = call(
        &server,
        Method::POST,
        "/lens/profiles/discovery/preview",
        admin,
        Some(request),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(store.lock().previews.is_empty());
}

#[rstest]
#[case::never_opted_in(None, "Turn on task discovery for this agent first")]
#[case::turned_off(Some(settings(false)), "Turn on task discovery for this agent first")]
#[case::model_left_the_gateway(Some(ProfileSettings { models: DiscoveryModelChoice { discovery: "retired".into(), ..choice() }, ..settings(true) }), "The task discovery model is not an available chat model on the gateway")]
#[tokio::test]
async fn discovery_starts_only_for_an_opted_in_agent_with_available_models(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] saved: Option<ProfileSettings>,
    #[case] detail: &str,
) {
    let store = Memory::with(AgentState {
        settings: saved,
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::POST,
        "/lens/profiles/discovery",
        admin,
        Some(json!({"agent": AGENT, "start_ms": 0, "end_ms": 10, "max_runs": 100})),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], json!(detail));
    assert_eq!(store.state().working, None);
}

#[rstest]
#[tokio::test]
async fn a_discovery_snapshots_the_saved_models_and_refuses_a_second_one(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = Memory::with(AgentState {
        active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
        settings: Some(settings(true)),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;
    let request = json!({"agent": AGENT, "start_ms": 0, "end_ms": 10, "max_runs": 100});

    let (status, body) = call(
        &server,
        Method::POST,
        "/lens/profiles/discovery",
        admin,
        Some(request.clone()),
    )
    .await;
    let (again, _) = call(
        &server,
        Method::POST,
        "/lens/profiles/discovery",
        admin,
        Some(request),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["taxonomy"]["status"], json!("discovering"));
    assert_eq!(
        body["taxonomy"]["models"],
        json!({"distill": "haiku", "embedding": "qwen", "synthesis": "haiku", "decision": "jev"})
    );
    assert_eq!(
        body["active"]["tasks"][0]["name"],
        json!("Answer billing questions")
    );
    assert_eq!(again, StatusCode::CONFLICT);
}

#[rstest]
#[tokio::test]
async fn an_admin_edits_and_activates_a_draft_with_a_backfill_over_http(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = Memory::with(AgentState {
        working: Some(taxonomy(TaxonomyStatus::Draft, Vec::new())),
        settings: Some(settings(true)),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;
    let edit = json!({"agent": AGENT, "revision": 5, "name": "Billing", "description": "", "inclusion_criteria": "", "exclusion_criteria": "", "required_capabilities": []});

    let (edited, body) = call(
        &server,
        Method::PATCH,
        "/lens/profiles/taxonomy/tasks/billing",
        admin,
        Some(edit),
    )
    .await;
    let (activated, view) = call(
        &server,
        Method::POST,
        "/lens/profiles/taxonomy/activate",
        admin,
        Some(json!({"agent": AGENT, "revision": 6, "backfill": true})),
    )
    .await;

    assert_eq!(edited, StatusCode::OK);
    assert_eq!(body["taxonomy"]["tasks"][0]["name"], json!("Billing"));
    assert_eq!(activated, StatusCode::OK);
    assert_eq!(view["taxonomy"], Value::Null);
    assert_eq!(view["active"]["tasks"][0]["name"], json!("Billing"));
    assert_eq!(
        view["active"]["windows"][0],
        json!({"start_ms": 1_000, "end_ms": 9_000})
    );
    assert_eq!(view["active"]["windows"][1]["end_ms"], Value::Null);
}

#[rstest]
#[case::missing_task("/lens/profiles/taxonomy/tasks/unknown", Method::PATCH, json!({"agent": AGENT, "revision": 5, "name": "x", "description": "", "inclusion_criteria": "", "exclusion_criteria": "", "required_capabilities": []}), StatusCode::NOT_FOUND)]
#[case::blank_name("/lens/profiles/taxonomy/tasks/billing", Method::PATCH, json!({"agent": AGENT, "revision": 5, "name": " ", "description": "", "inclusion_criteria": "", "exclusion_criteria": "", "required_capabilities": []}), StatusCode::BAD_REQUEST)]
#[case::stale_activation("/lens/profiles/taxonomy/activate", Method::POST, json!({"agent": AGENT, "revision": 4, "backfill": false}), StatusCode::CONFLICT)]
#[case::nothing_to_resume("/lens/profiles/taxonomy/resume", Method::POST, json!({"agent": AGENT, "revision": 5}), StatusCode::NOT_FOUND)]
#[case::unknown_field("/lens/profiles/taxonomy/activate", Method::POST, json!({"agent": AGENT, "revision": 5, "everything": true}), StatusCode::UNPROCESSABLE_ENTITY)]
#[tokio::test]
async fn draft_changes_report_why_they_were_refused(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] path: &str,
    #[case] method: Method,
    #[case] body: Value,
    #[case] expected: StatusCode,
) {
    let draft = AgentState {
        working: Some(taxonomy(TaxonomyStatus::Draft, Vec::new())),
        settings: Some(settings(true)),
        ..AgentState::default()
    };
    let store = Memory::with(draft.clone());
    let server = serve(&database, store.clone()).await;

    let (status, _) = call(&server, method, path, admin, Some(body)).await;

    assert_eq!(status, expected);
    assert_eq!(store.state(), draft);
}

#[rstest]
#[tokio::test]
async fn paused_classification_resumes_and_the_active_tasks_can_be_removed(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let paused = vec![ClassificationWindow {
        start_ms: 100,
        end_ms: Some(200),
    }];
    let store = Memory::with(AgentState {
        active: Some(taxonomy(TaxonomyStatus::Frozen, paused)),
        settings: Some(settings(true)),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;

    let (resumed, view) = call(
        &server,
        Method::POST,
        "/lens/profiles/taxonomy/resume",
        admin,
        Some(json!({"agent": AGENT, "revision": 5})),
    )
    .await;
    let (removed, after) = call(
        &server,
        Method::POST,
        "/lens/profiles/taxonomy/reset",
        admin,
        Some(json!({"agent": AGENT, "revision": 6, "target": "active"})),
    )
    .await;

    assert_eq!(resumed, StatusCode::OK);
    assert_eq!(view["active"]["windows"][1]["end_ms"], Value::Null);
    assert_eq!(removed, StatusCode::OK);
    assert_eq!(after["active"], Value::Null);
    assert_eq!(
        store.state().active.map(|active| active.status),
        Some(TaxonomyStatus::Cleared)
    );
}

#[rstest]
#[case::admin(Role::ProxyAdmin, json!(true), json!("Answer billing questions"))]
#[case::read_only_admin(Role::ProxyAdminViewer, json!(false), json!("Answer billing questions"))]
#[case::scoped_user(Role::InternalUser, json!(false), Value::Null)]
#[tokio::test]
async fn the_taxonomy_view_shows_enhanced_tasks_only_to_every_team_callers(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] can_write: Value,
    #[case] task: Value,
) {
    let server = serve(
        &database,
        Memory::with(AgentState {
            active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
            settings: Some(settings(true)),
            ..AgentState::default()
        }),
    )
    .await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/taxonomy?agent={AGENT}"),
        Some(role),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["can_write"], can_write);
    assert_eq!(body["active"]["tasks"][0]["name"], task);
}

#[rstest]
#[tokio::test]
async fn the_overview_resolves_runs_against_the_active_taxonomy_and_its_windows(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = traffic(AgentState {
        working: Some(taxonomy(TaxonomyStatus::Discovering, Vec::new())),
        active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/overview{}", window_query("")),
        admin,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["taxonomy"]["status"], json!("frozen"));
    assert_eq!(
        body["profiles"].as_array().map(|profiles| profiles
            .iter()
            .map(|profile| profile["name"].clone())
            .collect::<Vec<_>>()),
        Some(vec![
            json!("login help"),
            json!("Refunds"),
            json!("Answer billing questions")
        ])
    );
    assert_eq!(body["totals"]["spend_usd"], json!(0.25));
    assert_eq!(
        store.lock().queries.last().map(|query| (
            query.taxonomy_id.clone(),
            query.taxonomy_version,
            query.windows.clone(),
            query.limit
        )),
        Some(("tax".to_owned(), 2, open(), 5_001))
    );
}

#[rstest]
#[tokio::test]
async fn scoped_callers_never_resolve_against_an_enhanced_taxonomy(
    #[future(awt)] database: Database,
) {
    let store = traffic(AgentState {
        active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
        ..AgentState::default()
    });
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/overview{}", window_query("")),
        Some(Role::InternalUser),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["taxonomy"], Value::Null);
    assert_eq!(
        store.lock().queries.last().map(|query| (
            query.taxonomy_id.clone(),
            query.taxonomy_version,
            query.windows.len()
        )),
        Some((String::new(), 0, 0))
    );
}

#[rstest]
#[case::admin(Role::ProxyAdmin, 2, Some(json!("incompatible")))]
#[case::scoped_user(Role::InternalUser, 1, None)]
#[tokio::test]
async fn the_matrix_lists_idle_deployments_only_to_every_team_callers_and_compares_requirements(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] columns: usize,
    #[case] idle_fit: Option<Value>,
) {
    let server = serve(
        &database,
        traffic(AgentState {
            active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
            ..AgentState::default()
        }),
    )
    .await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/matrix{}", window_query("")),
        Some(role),
        None,
    )
    .await;
    let fit = |column: &str| {
        body["compatibility"]
            .as_array()
            .and_then(|entries| {
                entries.iter().find(|entry| {
                    entry["profile_id"] == json!("task:billing")
                        && entry["column_key"] == json!(column)
                })
            })
            .map(|entry| entry["status"].clone())
    };

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"].as_array().map(Vec::len), Some(columns));
    assert_eq!(fit("id:dep-vision"), Some(json!("unknown")));
    assert_eq!(fit("id:dep-text"), idle_fit);
}

#[rstest]
#[case::by_id("&profile_id=task:billing")]
#[case::by_legacy_label("&profile_id=Answer%20billing%20questions")]
#[tokio::test]
async fn the_detail_returns_a_reviewed_definition_with_its_runs(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] profile: &str,
) {
    let server = serve(
        &database,
        traffic(AgentState {
            active: Some(taxonomy(TaxonomyStatus::Frozen, open())),
            ..AgentState::default()
        }),
    )
    .await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/detail{}", window_query(profile)),
        admin,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["profile"]["profile_id"], json!("task:billing"));
    assert_eq!(
        body["definition"]["description"],
        json!("Questions about invoices")
    );
    assert_eq!(body["runs"][0]["trace_id"], json!("t1"));
    assert_eq!(body["models"][0]["column_key"], json!("id:dep-vision"));
}

#[rstest]
#[case::no_profile(
    "/lens/profiles/detail?agent=assistant&start_ms=0&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[case::unknown_profile(
    "/lens/profiles/detail?agent=assistant&start_ms=0&end_ms=100&profile_id=task:gone",
    StatusCode::NOT_FOUND
)]
#[case::reversed_window(
    "/lens/profiles/overview?agent=assistant&start_ms=200&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[case::no_agent(
    "/lens/profiles/matrix?agent=%20&start_ms=0&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[tokio::test]
async fn profile_reads_reject_unusable_requests(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] path: &str,
    #[case] expected: StatusCode,
) {
    let server = serve(&database, traffic(AgentState::default())).await;

    let (status, _) = call(&server, Method::GET, path, admin, None).await;

    assert_eq!(status, expected);
}

#[rstest]
#[case::anonymous_caller(None, false, StatusCode::UNAUTHORIZED)]
#[case::caller_without_trace_access(Some(Role::Customer), false, StatusCode::FORBIDDEN)]
#[case::storage_outage(Some(Role::ProxyAdmin), true, StatusCode::SERVICE_UNAVAILABLE)]
#[tokio::test]
async fn profile_reads_fail_closed(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] unavailable: bool,
    #[case] expected: StatusCode,
) {
    let store = Memory {
        unavailable,
        ..traffic(AgentState::default())
    };
    let server = serve(&database, store).await;
    let token = role.map(|role| {
        identity::delegated(Identity {
            user_role: role,
            token: Some("anonymous-key".into()),
            ..Identity::default()
        })
    });
    let request = server.client.get(
        server
            .url
            .join(&format!("/lens/profiles/overview{}", window_query("")))
            .expect("url"),
    );
    let request = match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };

    let status = request.send().await.expect("response").status();

    assert_eq!(status, expected);
}
