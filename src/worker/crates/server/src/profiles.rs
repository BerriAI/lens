use std::{collections::BTreeMap, future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, Method},
    routing::get,
};
use lens_auth::{Authentication, ReadScope, SessionRepository, trace_read_scope};
use lens_contract::profiles::{
    ModelCapabilities, ModelColumn, ProfileDetail, ProfileRef, ProfileSummary, ProfileWindow,
    ProfilesMatrix, ProfilesOverview,
};
use litellm_traces::query::named::ReadAccessParams;
use serde::Deserialize;

pub use crate::error::{ProfileHttpError, ProfileStoreError};
use crate::{auth, routing::PublicRoutes};

pub mod aggregate;

use aggregate::{Column, Run, RunCalls, View, source};

pub const MAX_RUNS: u32 = 5_000;
pub const MAX_SPANS: usize = 50_000;
const DETAIL_RUNS: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub struct RunQuery {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
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

struct App<R, S> {
    authentication: Arc<Authentication<R>>,
    store: S,
}

pub fn router<R: SessionRepository + 'static, S: ProfileStore + 'static>(
    authentication: Arc<Authentication<R>>,
    store: S,
) -> Router {
    Router::new()
        .public_route("/lens/profiles/overview", get(overview::<R, S>))
        .public_route("/lens/profiles/matrix", get(matrix::<R, S>))
        .public_route("/lens/profiles/detail", get(detail::<R, S>))
        .with_state(Arc::new(App {
            authentication,
            store,
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
        self.runs
            .iter()
            .find(|run| run.profile_id == profile_id)
            .map_or_else(String::new, |run| run.category.clone())
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

async fn load<S: ProfileStore>(
    store: &S,
    scope: &ReadAccessParams,
    query: &WindowQuery,
) -> Result<Loaded, ProfileHttpError> {
    let mut runs = store
        .runs(
            scope.clone(),
            RunQuery {
                agent: query.agent.clone(),
                start_ms: query.start_ms,
                end_ms: query.end_ms,
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
    })
}

async fn overview<R: SessionRepository, S: ProfileStore>(
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
        profiles: loaded.summaries(&query.agent),
    }))
}

async fn matrix<R: SessionRepository, S: ProfileStore>(
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
    }))
}

async fn detail<R: SessionRepository, S: ProfileStore>(
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
    Ok(Json(ProfileDetail {
        window: query.window(),
        coverage: view.coverage(),
        runs: view.runs_for(&profile.profile_id, DETAIL_RUNS),
        models: view
            .cells(Some(&profile.profile_id))
            .into_iter()
            .map(|(_, cell)| cell)
            .collect(),
        profile,
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

#[cfg(test)]
mod tests {
    use super::*;
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

    struct Inventory;

    impl ProfileStore for Inventory {
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
