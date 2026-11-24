use std::{collections::BTreeMap, future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method},
    routing::get,
};
use lens_auth::{Authentication, ReadScope, SessionRepository, trace_read_scope};
use lens_contract::profiles::{
    ClassificationWindow, DiscoveryModelChoice, ModelCapabilities, ModelColumn, ProfileDetail,
    ProfileRef, ProfileSignalCounts, ProfileSource, ProfileSummary, ProfileWindow, ProfilesMatrix,
    ProfilesOverview, Taxonomy, TaxonomyStatus, TaxonomyView,
};
use litellm_traces::query::named::ReadAccessParams;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub use crate::error::{ProfileHttpError, ProfileStoreError};
use crate::{auth, routing::PublicRoutes};

pub mod aggregate;
mod layout;
pub mod taxonomy;

use aggregate::{Column, Run, RunCalls, View, display_name, source};

pub const MAX_RUNS: u32 = 5_000;
pub const MAX_SPANS: usize = 50_000;
const DETAIL_RUNS: usize = 50;
const UNCERTAIN_NAME: &str = "Other / uncertain";

#[derive(Clone, Debug, PartialEq)]
pub struct RunQuery {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub taxonomy_id: String,
    pub taxonomy_version: u32,
    pub windows: Vec<ClassificationWindow>,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfiguredModel {
    pub model_id: String,
    pub model_group: String,
    pub provider: String,
    pub capabilities: ModelCapabilities,
}

pub trait ProfileStore: Send + Sync {
    fn runs(
        &self,
        scope: ReadAccessParams,
        query: RunQuery,
    ) -> impl Future<Output = Result<Vec<Run>, ProfileStoreError>> + Send;
    fn calls(
        &self,
        scope: ReadAccessParams,
        runs: Vec<Run>,
        max_spans: usize,
    ) -> impl Future<Output = Result<RunCalls, ProfileStoreError>> + Send;
    fn configured_models(&self) -> impl Future<Output = Vec<ConfiguredModel>> + Send;
    fn signal_counts(
        &self,
        signal_id: &str,
        runs: &[Run],
    ) -> impl Future<Output = Result<Option<ProfileSignalCounts>, ProfileStoreError>> + Send;
}

/// Enhanced taxonomies are built from every team's runs, so only callers who can already read
/// every team see them; scoped callers keep the recorded and local profiles.
pub fn taxonomy_id(scope: &ReadAccessParams, agent: &str) -> Option<String> {
    scope.all_teams.then(|| {
        format!(
            "{:x}",
            Sha256::digest(format!("global\0{agent}").as_bytes())
        )
    })
}

struct App<R, S> {
    authentication: Arc<Authentication<R>>,
    store: S,
    defaults: DiscoveryModelChoice,
}

pub fn router<
    R: SessionRepository + 'static,
    S: ProfileStore + taxonomy::TaxonomyStore + taxonomy::DiscoveryCatalog + 'static,
>(
    authentication: Arc<Authentication<R>>,
    store: S,
    defaults: DiscoveryModelChoice,
) -> Router {
    Router::new()
        .public_route("/lens/profiles/overview", get(overview::<R, S>))
        .public_route(
            "/lens/profiles/signals/{signal_id}",
            get(signal_counts::<R, S>),
        )
        .public_route("/lens/profiles/matrix", get(matrix::<R, S>))
        .public_route("/lens/profiles/detail", get(detail::<R, S>))
        .public_route("/lens/profiles/taxonomy", get(taxonomy_view::<R, S>))
        .merge(taxonomy::router::<R, S>())
        .with_state(Arc::new(App {
            authentication,
            store,
            defaults,
        }))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowQuery {
    agent: String,
    start_ms: i64,
    end_ms: i64,
    #[serde(default)]
    profile_id: Option<String>,
}

impl WindowQuery {
    fn validate(self) -> Result<Self, ProfileHttpError> {
        if self.agent.trim().is_empty() {
            return Err(ProfileHttpError::Invalid("Select an agent"));
        }
        if self.end_ms <= self.start_ms {
            return Err(ProfileHttpError::Invalid("end_ms must be after start_ms"));
        }
        Ok(self)
    }

    fn window(&self) -> ProfileWindow {
        ProfileWindow {
            agent: self.agent.clone(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
        }
    }
}

fn scope(identity: &lens_contract::auth::Identity) -> Result<ReadAccessParams, ProfileHttpError> {
    match trace_read_scope(identity) {
        Some(ReadScope::AllRows) => Ok(ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
        }),
        Some(ReadScope::OwnedRows { user_id, team_ids }) => Ok(ReadAccessParams {
            all_teams: false,
            user_id,
            team_ids,
        }),
        None => Err(ProfileHttpError::Forbidden(
            "Not allowed to view agent traces",
        )),
    }
}

struct Loaded {
    runs: Vec<Run>,
    calls: RunCalls,
    truncated: bool,
    taxonomy: Option<Taxonomy>,
    draft: Option<Taxonomy>,
}

fn map_layout(state: &taxonomy::AgentState) -> Option<&Taxonomy> {
    state.active().or_else(|| {
        state
            .under_review()
            .filter(|working| working.status == TaxonomyStatus::Draft)
    })
}

impl Loaded {
    fn view<'a>(&'a self, agent: &'a str) -> View<'a> {
        View {
            runs: &self.runs,
            calls: &self.calls,
            agent,
            truncated: self.truncated,
        }
    }

    fn ranked(&self, agent: &str) -> Vec<(String, aggregate::ProfileFigures)> {
        let mut ranked: Vec<_> = self
            .view(agent)
            .by_profile()
            .into_iter()
            .map(|(profile_id, figures)| (profile_id.to_owned(), figures))
            .collect();
        ranked.sort_by(|(left_id, left), (right_id, right)| {
            right
                .runs
                .cmp(&left.runs)
                .then_with(|| left_id.cmp(right_id))
        });
        ranked
    }

    fn name(&self, profile_id: &str) -> String {
        if matches!(source(profile_id), ProfileSource::Uncertain) {
            return UNCERTAIN_NAME.into();
        }
        let category = self
            .runs
            .iter()
            .find(|run| run.profile_id == profile_id)
            .map_or("", |run| run.category.as_str());
        display_name(profile_id, category, self.taxonomy.as_ref())
    }

    fn summaries(&self, agent: &str) -> Vec<ProfileSummary> {
        let ranked = self.ranked(agent);
        let total = self.runs.len().max(1) as f64;
        ranked
            .iter()
            .map(|(profile_id, figures)| ProfileSummary {
                profile_id: profile_id.clone(),
                name: self.name(profile_id),
                source: source(profile_id),
                run_count: figures.runs,
                share: figures.runs as f64 / total,
                request_count: figures.requests,
                cost_observations: figures.cost_observations,
                spend_usd: figures.spend,
            })
            .collect()
    }
}

/// Idle configured deployments reveal the gateway's inventory, so only callers who can read every
/// team see them; scoped callers see the columns their own traffic used.
async fn inventory<S: ProfileStore>(store: &S, scope: &ReadAccessParams) -> Vec<ConfiguredModel> {
    if scope.all_teams {
        store.configured_models().await
    } else {
        Vec::new()
    }
}

async fn load_runs<S: ProfileStore + taxonomy::TaxonomyStore>(
    store: &S,
    scope: &ReadAccessParams,
    query: &WindowQuery,
) -> Result<Loaded, ProfileHttpError> {
    let state = match taxonomy_id(scope, &query.agent) {
        Some(id) => Some(store.load(&id).await?.0),
        None => None,
    };
    let taxonomy = state.as_ref().and_then(|state| state.active().cloned());
    let draft = state
        .as_ref()
        .filter(|_| taxonomy.is_none())
        .and_then(map_layout)
        .cloned();
    let frozen = taxonomy.as_ref();
    let mut runs = store
        .runs(
            scope.clone(),
            RunQuery {
                agent: query.agent.clone(),
                start_ms: query.start_ms,
                end_ms: query.end_ms,
                taxonomy_id: frozen
                    .map_or_else(String::new, |taxonomy| taxonomy.taxonomy_id.clone()),
                taxonomy_version: frozen.map_or(0, |taxonomy| taxonomy.version),
                windows: frozen.map_or_else(Vec::new, |taxonomy| taxonomy.windows.clone()),
                limit: MAX_RUNS + 1,
            },
        )
        .await?;
    let truncated = runs.len() > MAX_RUNS as usize;
    runs.truncate(MAX_RUNS as usize);
    Ok(Loaded {
        runs,
        calls: RunCalls::default(),
        truncated,
        taxonomy,
        draft,
    })
}

async fn load<S: ProfileStore + taxonomy::TaxonomyStore>(
    store: &S,
    scope: &ReadAccessParams,
    query: &WindowQuery,
) -> Result<Loaded, ProfileHttpError> {
    let mut loaded = load_runs(store, scope, query).await?;
    loaded.calls = store
        .calls(scope.clone(), loaded.runs.clone(), MAX_SPANS)
        .await?;
    Ok(loaded)
}

async fn overview<R: SessionRepository, S: ProfileStore + taxonomy::TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<WindowQuery>,
) -> Result<Json<ProfilesOverview>, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    let query = query.validate()?;
    let loaded = load(&app.store, &scope, &query).await?;
    let view = loaded.view(&query.agent);
    Ok(Json(ProfilesOverview {
        window: query.window(),
        coverage: view.coverage(),
        totals: view.totals(),
        points: layout::points(
            &loaded.runs,
            loaded.taxonomy.as_ref().or(loaded.draft.as_ref()),
        ),
        profiles: loaded.summaries(&query.agent),
    }))
}

async fn signal_counts<R: SessionRepository, S: ProfileStore + taxonomy::TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Path(signal_id): Path<String>,
    Query(query): Query<WindowQuery>,
) -> Result<Json<ProfileSignalCounts>, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    crate::signals::authorize(identity.user_role, false)
        .map_err(|_| ProfileHttpError::Forbidden("Not allowed to view Signals"))?;
    let scope = scope(&identity)?;
    let query = query.validate()?;
    let loaded = load_runs(&app.store, &scope, &query).await?;
    app.store
        .signal_counts(&signal_id, &loaded.runs)
        .await?
        .map(Json)
        .ok_or(ProfileHttpError::Invalid("Select a configured Signal"))
}

async fn matrix<R: SessionRepository, S: ProfileStore + taxonomy::TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<WindowQuery>,
) -> Result<Json<ProfilesMatrix>, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    let query = query.validate()?;
    let loaded = load(&app.store, &scope, &query).await?;
    let configured = inventory(&app.store, &scope).await;
    let view = loaded.view(&query.agent);
    let cells = view.cells(None);
    let profiles: Vec<ProfileRef> = loaded
        .ranked(&query.agent)
        .into_iter()
        .map(|(profile_id, _)| ProfileRef {
            name: loaded.name(&profile_id),
            source: source(&profile_id),
            required_capabilities: None,
            profile_id,
        })
        .collect();
    let models = columns(&configured, cells.iter().map(|(column, _)| column));
    Ok(Json(ProfilesMatrix {
        window: query.window(),
        coverage: view.coverage(),
        other_agent_requests: view.totals().other_agent_requests,
        cells: cells.into_iter().map(|(_, cell)| cell).collect(),
        profiles,
        models,
        compatibility: Vec::new(),
    }))
}

async fn detail<R: SessionRepository, S: ProfileStore + taxonomy::TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<WindowQuery>,
) -> Result<Json<ProfileDetail>, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    let query = query.validate()?;
    let requested = query
        .profile_id
        .clone()
        .filter(|profile| !profile.is_empty())
        .ok_or(ProfileHttpError::Invalid("Choose a profile"))?;
    let loaded = load(&app.store, &scope, &query).await?;
    let profile = loaded
        .summaries(&query.agent)
        .into_iter()
        .find(|profile| profile.profile_id == requested || profile.name == requested)
        .ok_or(ProfileHttpError::NotFound)?;
    let view = loaded.view(&query.agent);
    let definition = profile
        .profile_id
        .strip_prefix("task:")
        .and_then(|task_id| {
            loaded
                .taxonomy
                .as_ref()?
                .tasks
                .iter()
                .find(|task| task.task_id == task_id)
                .cloned()
        });
    Ok(Json(ProfileDetail {
        window: query.window(),
        coverage: view.coverage(),
        runs: view.runs_for(&profile.profile_id, DETAIL_RUNS),
        models: view
            .cells(Some(&profile.profile_id))
            .into_iter()
            .map(|(_, cell)| cell)
            .collect(),
        definition,
        profile,
    }))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentQuery {
    agent: String,
}

async fn taxonomy_view<R: SessionRepository, S: ProfileStore + taxonomy::TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<AgentQuery>,
) -> Result<Json<TaxonomyView>, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    let Some(id) = taxonomy_id(&scope, &query.agent) else {
        return Ok(Json(TaxonomyView {
            taxonomy: None,
            active: None,
            discovery_available: false,
            can_write: false,
        }));
    };
    let (state, _) = app.store.load(&id).await?;
    Ok(Json(taxonomy::taxonomy_view(&state, &identity)))
}

fn columns<'a>(
    configured: &[ConfiguredModel],
    observed: impl Iterator<Item = &'a Column>,
) -> Vec<ModelColumn> {
    let mut columns: BTreeMap<String, ModelColumn> = configured
        .iter()
        .map(|model| {
            let column = Column::Deployment {
                model_id: model.model_id.clone(),
                model_group: model.model_group.clone(),
                provider: model.provider.clone(),
            };
            (
                column.key(),
                ModelColumn {
                    column_key: column.key(),
                    model_config_id: Some(model.model_id.clone()),
                    model_name: model.model_group.clone(),
                    model_group: model.model_group.clone(),
                    provider: model.provider.clone(),
                    configured: true,
                    capabilities: model.capabilities.clone(),
                },
            )
        })
        .collect();
    for column in observed {
        columns.entry(column.key()).or_insert_with(|| match column {
            Column::Deployment {
                model_id,
                model_group,
                provider,
            } => ModelColumn {
                column_key: column.key(),
                model_config_id: Some(model_id.clone()),
                model_name: model_group.clone(),
                model_group: model_group.clone(),
                provider: provider.clone(),
                configured: false,
                capabilities: ModelCapabilities::default(),
            },
            Column::UnknownDeployment { model_group } => ModelColumn {
                column_key: column.key(),
                model_config_id: None,
                model_name: model_group.clone(),
                model_group: model_group.clone(),
                provider: String::new(),
                configured: false,
                capabilities: ModelCapabilities::default(),
            },
            Column::Unmatched { model } => ModelColumn {
                column_key: column.key(),
                model_config_id: None,
                model_name: model.clone(),
                model_group: String::new(),
                provider: String::new(),
                configured: false,
                capabilities: ModelCapabilities::default(),
            },
        });
    }
    columns.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::TaxonomyModels;
    use rstest::rstest;

    #[rstest]
    fn configured_models_without_traffic_stay_as_columns() {
        let configured = [ConfiguredModel {
            model_id: "idle".into(),
            model_group: "idle-model".into(),
            provider: "openai".into(),
            capabilities: ModelCapabilities::default(),
        }];
        let observed = [Column::Unmatched {
            model: "gpt".into(),
        }];

        let models = columns(&configured, observed.iter());

        assert_eq!(models.len(), 2);
        assert!(
            models
                .iter()
                .any(|model| model.model_config_id.as_deref() == Some("idle") && model.configured)
        );
        assert!(
            models
                .iter()
                .any(|model| model.model_config_id.is_none() && !model.configured)
        );
    }

    fn listed(trace_id: &str, profile_id: &str, category: &str) -> Run {
        Run {
            team_id: "team".into(),
            api_key_hash: "key".into(),
            trace_id: trace_id.into(),
            start_ms: 1,
            end_ms: 1,
            profile_id: profile_id.into(),
            category: category.into(),
            input_preview: String::new(),
            agent_count: 1,
        }
    }

    #[rstest]
    #[case::stored_catch_all_label_first(["Other / uncertain", "Maybe billing"])]
    #[case::new_label_first(["Maybe billing", "Other / uncertain"])]
    #[case::only_new_labels(["Maybe billing", "Draft refunds"])]
    fn the_uncertain_profile_keeps_its_name_whatever_labels_its_runs_carry(
        #[case] uncertain_labels: [&str; 2],
    ) {
        let loaded = Loaded {
            runs: vec![
                listed("t1", "uncertain", uncertain_labels[0]),
                listed("t2", "recorded:Coding", "Coding"),
                listed("t3", "uncertain", uncertain_labels[1]),
                listed("t4", "cluster:c1", "Summarize sources"),
                listed("t5", "unclassified", "Unclassified"),
            ],
            calls: RunCalls::new(),
            truncated: false,
            taxonomy: None,
            draft: None,
        };

        let names: BTreeMap<String, String> = loaded
            .summaries("agent")
            .into_iter()
            .map(|profile| (profile.profile_id, profile.name))
            .collect();

        assert_eq!(
            names,
            BTreeMap::from([
                ("cluster:c1".to_owned(), "Summarize sources".to_owned()),
                ("recorded:Coding".to_owned(), "Coding".to_owned()),
                ("uncertain".to_owned(), "Other / uncertain".to_owned()),
                ("unclassified".to_owned(), "Unclassified".to_owned()),
            ])
        );
    }

    #[rstest]
    #[case::admin(true, true)]
    #[case::scoped(false, false)]
    fn only_every_team_scopes_get_an_enhanced_taxonomy(
        #[case] all_teams: bool,
        #[case] visible: bool,
    ) {
        let scope = ReadAccessParams {
            all_teams,
            user_id: "user".into(),
            team_ids: vec!["team".into()],
        };

        assert_eq!(taxonomy_id(&scope, "agent").is_some(), visible);
    }

    fn stored(taxonomy_id: &str, status: TaxonomyStatus) -> Taxonomy {
        Taxonomy {
            taxonomy_id: taxonomy_id.into(),
            agent: "agent".into(),
            version: 1,
            revision: 1,
            status,
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

    #[rstest]
    #[case::active_only(None, Some("active"), Some("active"))]
    #[case::completed_draft_without_active(Some(TaxonomyStatus::Draft), None, Some("working"))]
    #[case::active_preferred_over_completed_draft(
        Some(TaxonomyStatus::Draft),
        Some("active"),
        Some("active")
    )]
    #[case::active_preferred_during_rediscovery(
        Some(TaxonomyStatus::Discovering),
        Some("active"),
        Some("active")
    )]
    #[case::running_discovery_has_no_layout(Some(TaxonomyStatus::Discovering), None, None)]
    #[case::failed_discovery_has_no_layout(Some(TaxonomyStatus::Failed), None, None)]
    #[case::nothing_stored(None, None, None)]
    fn the_map_prefers_the_active_taxonomy_and_falls_back_only_to_a_completed_draft(
        #[case] working: Option<TaxonomyStatus>,
        #[case] active: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let state = taxonomy::AgentState {
            working: working.map(|status| stored("working", status)),
            active: active.map(|id| stored(id, TaxonomyStatus::Frozen)),
            settings: None,
        };

        assert_eq!(
            map_layout(&state).map(|taxonomy| taxonomy.taxonomy_id.as_str()),
            expected
        );
    }

    struct Inventory;

    impl ProfileStore for Inventory {
        async fn signal_counts(
            &self,
            _: &str,
            _: &[Run],
        ) -> Result<Option<ProfileSignalCounts>, ProfileStoreError> {
            Ok(None)
        }

        async fn runs(
            &self,
            _: ReadAccessParams,
            _: RunQuery,
        ) -> Result<Vec<Run>, ProfileStoreError> {
            Ok(Vec::new())
        }

        async fn calls(
            &self,
            _: ReadAccessParams,
            _: Vec<Run>,
            _: usize,
        ) -> Result<RunCalls, ProfileStoreError> {
            Ok(RunCalls::new())
        }

        async fn configured_models(&self) -> Vec<ConfiguredModel> {
            vec![ConfiguredModel {
                model_id: "idle".into(),
                model_group: "idle-model".into(),
                provider: "openai".into(),
                capabilities: ModelCapabilities::default(),
            }]
        }
    }

    #[rstest]
    #[case::every_team(true, 1)]
    #[case::scoped(false, 0)]
    #[tokio::test]
    async fn idle_gateway_deployments_are_listed_only_for_every_team_scopes(
        #[case] all_teams: bool,
        #[case] expected: usize,
    ) {
        let scope = ReadAccessParams {
            all_teams,
            user_id: "user".into(),
            team_ids: vec!["team".into()],
        };

        assert_eq!(inventory(&Inventory, &scope).await.len(), expected);
    }
}
