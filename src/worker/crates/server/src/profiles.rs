use std::{collections::BTreeMap, future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, Method},
    routing::get,
};
use lens_auth::{Authentication, ReadScope, SessionRepository, trace_read_scope};
use lens_contract::profiles::{
    Capability, Compatibility, CompatibilityStatus, ModelCapabilities, ModelColumn,
    ProfileDetail, ProfileRef, ProfileSummary, ProfileWindow, ProfilesMatrix, ProfilesOverview,
    Taxonomy, TaxonomyModels, TaxonomyView,
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

#[derive(Clone, Debug, PartialEq)]
pub struct RunQuery {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub taxonomy_id: String,
    pub taxonomy_version: u32,
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
    discovery: Option<TaxonomyModels>,
}

pub fn router<R: SessionRepository + 'static, S: ProfileStore + taxonomy::TaxonomyStore + 'static>(
    authentication: Arc<Authentication<R>>,
    store: S,
    discovery: Option<TaxonomyModels>,
) -> Router {
    Router::new()
        .public_route("/lens/profiles/overview", get(overview::<R, S>))
        .public_route("/lens/profiles/matrix", get(matrix::<R, S>))
        .public_route("/lens/profiles/detail", get(detail::<R, S>))
        .public_route("/lens/profiles/taxonomy", get(taxonomy_view::<R, S>))
        .merge(taxonomy::router::<R, S>())
        .with_state(Arc::new(App {
            authentication,
            store,
            discovery,
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
        None => Err(ProfileHttpError::Forbidden("Not allowed to view agent traces")),
    }
}

struct Loaded {
    runs: Vec<Run>,
    calls: RunCalls,
    truncated: bool,
    taxonomy: Option<Taxonomy>,
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
            right.runs.cmp(&left.runs).then_with(|| left_id.cmp(right_id))
        });
        ranked
    }

    fn name(&self, profile_id: &str) -> String {
        let category = self
            .runs
            .iter()
            .find(|run| run.profile_id == profile_id)
            .map_or("", |run| run.category.as_str());
        display_name(profile_id, category, self.taxonomy.as_ref())
    }

    fn summaries(&self, agent: &str) -> Vec<ProfileSummary> {
        let ranked = self.ranked(agent);
        let ids: Vec<&str> = ranked.iter().map(|(id, _)| id.as_str()).collect();
        let centers = layout::centers(&ids, self.taxonomy.as_ref());
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
                center: centers[profile_id],
            })
            .collect()
    }

    fn required(&self, profile_id: &str) -> Vec<Capability> {
        profile_id
            .strip_prefix("task:")
            .and_then(|task_id| {
                self.taxonomy
                    .as_ref()?
                    .tasks
                    .iter()
                    .find(|task| task.task_id == task_id)
                    .map(|task| task.required_capabilities.clone())
            })
            .unwrap_or_default()
    }
}

async fn load<S: ProfileStore + taxonomy::TaxonomyStore>(
    store: &S,
    scope: &ReadAccessParams,
    query: &WindowQuery,
) -> Result<Loaded, ProfileHttpError> {
    let taxonomy = match taxonomy_id(scope, &query.agent) {
        Some(id) => store.read(&id).await?.0,
        None => None,
    };
    let frozen = taxonomy
        .as_ref()
        .filter(|taxonomy| taxonomy.frozen_at.is_some());
    let mut runs = store
        .runs(
            scope.clone(),
            RunQuery {
                agent: query.agent.clone(),
                start_ms: query.start_ms,
                end_ms: query.end_ms,
                taxonomy_id: frozen.map_or_else(String::new, |taxonomy| taxonomy.taxonomy_id.clone()),
                taxonomy_version: frozen.map_or(0, |taxonomy| taxonomy.version),
                limit: MAX_RUNS + 1,
            },
        )
        .await?;
    let truncated = runs.len() > MAX_RUNS as usize;
    runs.truncate(MAX_RUNS as usize);
    let calls = store.calls(scope.clone(), runs.clone(), MAX_SPANS).await?;
    let truncated = truncated || calls.len() < runs.len();
    Ok(Loaded {
        runs,
        calls,
        truncated,
        taxonomy,
    })
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
    let profiles = loaded.summaries(&query.agent);
    let centers: BTreeMap<_, _> = profiles
        .iter()
        .map(|profile| (profile.profile_id.clone(), profile.center))
        .collect();
    Ok(Json(ProfilesOverview {
        window: query.window(),
        coverage: view.coverage(),
        totals: view.totals(),
        points: layout::points(&loaded.runs, &centers, loaded.taxonomy.as_ref()),
        profiles,
        taxonomy: loaded.taxonomy.as_ref().map(Taxonomy::summary),
    }))
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
    let configured = app.store.configured_models().await;
    let view = loaded.view(&query.agent);
    let cells = view.cells(None);
    let profiles: Vec<ProfileRef> = loaded
        .ranked(&query.agent)
        .into_iter()
        .map(|(profile_id, _)| ProfileRef {
            name: loaded.name(&profile_id),
            source: source(&profile_id),
            required_capabilities: loaded.required(&profile_id),
            profile_id,
        })
        .collect();
    let models = columns(&configured, cells.iter().map(|(column, _)| column));
    let compatibilities = profiles
        .iter()
        .flat_map(|profile| {
            models
                .iter()
                .map(move |model| compatibility(profile, model))
        })
        .collect();
    Ok(Json(ProfilesMatrix {
        window: query.window(),
        coverage: view.coverage(),
        other_agent_requests: view.totals().other_agent_requests,
        cells: cells.into_iter().map(|(_, cell)| cell).collect(),
        profiles,
        models,
        compatibility: compatibilities,
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
    let definition = profile.profile_id.strip_prefix("task:").and_then(|task_id| {
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
            discovery_available: false,
            can_write: false,
        }));
    };
    Ok(Json(TaxonomyView {
        taxonomy: app.store.read(&id).await?.0,
        discovery_available: app.discovery.is_some(),
        can_write: taxonomy::can_write(&identity),
    }))
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

fn supports(capabilities: &ModelCapabilities, capability: Capability) -> Option<bool> {
    match capability {
        Capability::FunctionCalling => capabilities.function_calling,
        Capability::Vision => capabilities.vision,
        Capability::Reasoning => capabilities.reasoning,
        Capability::StructuredOutput => capabilities.structured_output,
    }
}

fn compatibility(profile: &ProfileRef, model: &ModelColumn) -> Compatibility {
    let missing: Vec<Capability> = profile
        .required_capabilities
        .iter()
        .copied()
        .filter(|capability| supports(&model.capabilities, *capability) == Some(false))
        .collect();
    let unknown: Vec<Capability> = profile
        .required_capabilities
        .iter()
        .copied()
        .filter(|capability| supports(&model.capabilities, *capability).is_none())
        .collect();
    let status = if !missing.is_empty() {
        CompatibilityStatus::Incompatible
    } else if !unknown.is_empty() {
        CompatibilityStatus::Unknown
    } else {
        CompatibilityStatus::Compatible
    };
    Compatibility {
        profile_id: profile.profile_id.clone(),
        column_key: model.column_key.clone(),
        status,
        missing,
        unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::ProfileSource;
    use rstest::rstest;

    fn column(capabilities: ModelCapabilities) -> ModelColumn {
        ModelColumn {
            column_key: "id:dep".into(),
            model_config_id: Some("dep".into()),
            model_name: "model".into(),
            model_group: "model".into(),
            provider: "provider".into(),
            configured: true,
            capabilities,
        }
    }

    fn profile(required: Vec<Capability>) -> ProfileRef {
        ProfileRef {
            profile_id: "task:a".into(),
            name: "Task".into(),
            source: ProfileSource::Enhanced,
            required_capabilities: required,
        }
    }

    #[rstest]
    #[case::supported(Some(true), CompatibilityStatus::Compatible)]
    #[case::unsupported(Some(false), CompatibilityStatus::Incompatible)]
    #[case::undeclared(None, CompatibilityStatus::Unknown)]
    fn compatibility_follows_declared_metadata(
        #[case] vision: Option<bool>,
        #[case] expected: CompatibilityStatus,
    ) {
        let model = column(ModelCapabilities {
            vision,
            ..ModelCapabilities::default()
        });

        let result = compatibility(&profile(vec![Capability::Vision]), &model);

        assert_eq!(result.status, expected);
    }

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
        assert!(models.iter().any(|model| model.model_config_id.as_deref() == Some("idle") && model.configured));
        assert!(models.iter().any(|model| model.model_config_id.is_none() && !model.configured));
    }

    #[rstest]
    #[case::admin(true, true)]
    #[case::scoped(false, false)]
    fn only_every_team_scopes_get_an_enhanced_taxonomy(#[case] all_teams: bool, #[case] visible: bool) {
        let scope = ReadAccessParams {
            all_teams,
            user_id: "user".into(),
            team_ids: vec!["team".into()],
        };

        assert_eq!(taxonomy_id(&scope, "agent").is_some(), visible);
    }
}
