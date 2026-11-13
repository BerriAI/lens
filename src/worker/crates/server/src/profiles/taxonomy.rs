use std::{future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method},
    routing::{get, patch, post},
};
use chrono::{DateTime, Duration, Utc};
use lens_auth::SessionRepository;
use lens_contract::{
    auth::{Identity, Role},
    profiles::{
        ActivationRequest, ClassificationWindow, DiscoveryJob, DiscoveryModelChoice,
        DiscoveryModelOptions, DiscoveryPoint, DiscoveryPreview, DiscoveryRequest, DiscoveryRun,
        ProfileSettings, ProfileSettingsUpdate, ProfileSettingsView, ResetRequest, TaskDefinition,
        TaskEdit, Taxonomy, TaxonomyModels, TaxonomyRevisionRequest, TaxonomyStatus,
        TaxonomyTarget,
    },
};
use serde::Deserialize;

use super::{App, ProfileHttpError, ProfileStore, ProfileStoreError, scope, taxonomy_id};
use crate::{auth, routing::PublicRoutes};

pub const MIN_DISCOVERY_RUNS: u32 = 20;
pub const MAX_DISCOVERY_RUNS: u32 = 500;
pub const MAX_ATTEMPTS: u32 = 3;
pub const LEASE: Duration = Duration::minutes(30);
const NAME_LIMIT: usize = 80;
const TEXT_LIMIT: usize = 600;
const CANCELLED: &str = "Discovery was cancelled because task discovery was turned off";

/// The three stored documents of one agent's enhanced profiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentState {
    pub working: Option<Taxonomy>,
    pub active: Option<Taxonomy>,
    pub settings: Option<ProfileSettings>,
}

/// Documents to replace together; untouched documents stay `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    pub working: Option<Taxonomy>,
    pub active: Option<Taxonomy>,
    pub settings: Option<ProfileSettings>,
}

impl AgentState {
    /// Taxonomies frozen before activation had its own document are still read as active.
    pub fn active(&self) -> Option<&Taxonomy> {
        match &self.active {
            Some(active) => (active.status == TaxonomyStatus::Frozen).then_some(active),
            None => self
                .working
                .as_ref()
                .filter(|working| working.status == TaxonomyStatus::Frozen),
        }
    }

    pub fn under_review(&self) -> Option<&Taxonomy> {
        self.working.as_ref().filter(|working| {
            matches!(
                working.status,
                TaxonomyStatus::Discovering | TaxonomyStatus::Draft | TaxonomyStatus::Failed
            )
        })
    }

    pub fn enabled(&self) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|settings| settings.enabled)
    }

    pub fn models(&self) -> Option<TaxonomyModels> {
        let choice = &self.settings.as_ref()?.models;
        complete_choice(choice).then(|| TaxonomyModels {
            distill: choice.discovery.clone(),
            embedding: choice.embedding.clone(),
            synthesis: choice.discovery.clone(),
            decision: choice.classification.clone(),
        })
    }

    fn generation(&self) -> u32 {
        [&self.working, &self.active]
            .into_iter()
            .flatten()
            .map(|taxonomy| taxonomy.version)
            .max()
            .unwrap_or(0)
    }

    fn legacy(&self) -> bool {
        self.active.is_none() && self.active().is_some()
    }
}

pub trait TaxonomyStore: Send + Sync {
    type Heads: Send;

    fn load(
        &self,
        taxonomy_id: &str,
    ) -> impl Future<Output = Result<(AgentState, Self::Heads), ProfileStoreError>> + Send;

    /// Replaces the changed documents only if none of the agent's documents moved since
    /// `heads`, including unchanged ones a transition depended on; otherwise `Conflict`.
    fn save(
        &self,
        taxonomy_id: &str,
        heads: Self::Heads,
        changes: Changes,
    ) -> impl Future<Output = Result<(), ProfileStoreError>> + Send;
}

pub trait DiscoveryCatalog: Send + Sync {
    fn options(&self) -> impl Future<Output = DiscoveryModelOptions> + Send;

    fn preview(
        &self,
        request: DiscoveryRequest,
    ) -> impl Future<Output = Result<DiscoveryPreview, ProfileStoreError>> + Send;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelRole {
    Discovery,
    Embedding,
    Classification,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum TransitionError {
    #[error("Discovery is already running for this agent")]
    Running,
    #[error("Task discovery changed since it was loaded; reload and try again")]
    Stale,
    #[error("Only a draft taxonomy can be edited or activated")]
    NotDraft,
    #[error("A draft needs at least one task before it can be activated")]
    Empty,
    #[error("Task not found in this draft")]
    MissingTask,
    #[error("Task names must be 1-80 characters and other fields at most 600")]
    InvalidEdit,
    #[error("Discovery reads between 20 and 500 runs from a non-empty range")]
    InvalidSize,
    #[error("Turn on task discovery for this agent first")]
    Disabled,
    #[error("Choose a model for each step before turning on task discovery")]
    Unconfigured,
    #[error("A chosen model is not available on the gateway")]
    UnavailableModel(ModelRole),
    #[error("This agent has no active taxonomy")]
    NotActive,
    #[error("Classification is already running for new runs")]
    AlreadyClassifying,
    #[error("This draft has no discovery range to classify")]
    NoRange,
}

fn complete_choice(choice: &DiscoveryModelChoice) -> bool {
    [&choice.discovery, &choice.embedding, &choice.classification]
        .iter()
        .all(|alias| !alias.trim().is_empty())
}

fn trimmed(choice: &DiscoveryModelChoice) -> DiscoveryModelChoice {
    DiscoveryModelChoice {
        discovery: choice.discovery.trim().to_owned(),
        embedding: choice.embedding.trim().to_owned(),
        classification: choice.classification.trim().to_owned(),
    }
}

pub fn validate_models(
    choice: &DiscoveryModelChoice,
    options: &DiscoveryModelOptions,
) -> Result<(), TransitionError> {
    if !complete_choice(choice) {
        return Err(TransitionError::Unconfigured);
    }
    [
        (ModelRole::Discovery, &choice.discovery, &options.discovery),
        (ModelRole::Embedding, &choice.embedding, &options.embedding),
        (
            ModelRole::Classification,
            &choice.classification,
            &options.classification,
        ),
    ]
    .into_iter()
    .find(|(_, alias, available)| !available.contains(alias))
    .map_or(Ok(()), |(role, _, _)| {
        Err(TransitionError::UnavailableModel(role))
    })
}

fn active_job(taxonomy: &Taxonomy, now: DateTime<Utc>) -> bool {
    taxonomy.status == TaxonomyStatus::Discovering
        && taxonomy.job.as_ref().is_some_and(|job| {
            job.attempts < MAX_ATTEMPTS && job.lease_until.is_none_or(|lease| lease > now)
        })
}

fn next(taxonomy: Taxonomy) -> Taxonomy {
    Taxonomy {
        revision: taxonomy.revision.saturating_add(1),
        ..taxonomy
    }
}

fn cleared(taxonomy: &Taxonomy) -> Taxonomy {
    next(Taxonomy {
        status: TaxonomyStatus::Cleared,
        tasks: Vec::new(),
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        windows: Vec::new(),
        ..taxonomy.clone()
    })
}

fn open(taxonomy: &Taxonomy) -> bool {
    taxonomy
        .windows
        .iter()
        .any(|window| window.end_ms.is_none())
}

/// Writes a changed active taxonomy, moving one still stored the old way into its own document.
fn with_active(state: &AgentState, active: Taxonomy) -> Changes {
    Changes {
        working: state
            .legacy()
            .then(|| state.working.as_ref().map(cleared))
            .flatten(),
        active: Some(active),
        settings: None,
    }
}

pub fn start_discovery(
    state: &AgentState,
    taxonomy_id: &str,
    request: &DiscoveryRequest,
    job_id: String,
    now: DateTime<Utc>,
) -> Result<Changes, TransitionError> {
    if !(MIN_DISCOVERY_RUNS..=MAX_DISCOVERY_RUNS).contains(&request.max_runs)
        || request.end_ms <= request.start_ms
    {
        return Err(TransitionError::InvalidSize);
    }
    if !state.enabled() {
        return Err(TransitionError::Disabled);
    }
    let models = state.models().ok_or(TransitionError::Unconfigured)?;
    if state
        .working
        .as_ref()
        .is_some_and(|working| active_job(working, now))
    {
        return Err(TransitionError::Running);
    }
    let working = Taxonomy {
        taxonomy_id: taxonomy_id.to_owned(),
        agent: request.agent.clone(),
        version: state.generation().saturating_add(1),
        revision: state
            .working
            .as_ref()
            .map_or(1, |working| working.revision.saturating_add(1)),
        status: TaxonomyStatus::Discovering,
        sample: false,
        tasks: Vec::new(),
        points: Vec::new(),
        job: Some(DiscoveryJob {
            job_id,
            requested_at: now,
            lease_until: None,
            attempts: 0,
            max_runs: request.max_runs,
            start_ms: request.start_ms,
            end_ms: request.end_ms,
        }),
        discovered_at: None,
        frozen_at: None,
        error: None,
        models,
        discovery: None,
        windows: Vec::new(),
    };
    Ok(Changes {
        working: Some(working),
        active: state.legacy().then(|| state.active().cloned()).flatten(),
        settings: None,
    })
}

fn draft(state: &AgentState, revision: u32) -> Result<Taxonomy, TransitionError> {
    let taxonomy = state
        .working
        .clone()
        .filter(|taxonomy| taxonomy.revision == revision)
        .ok_or(TransitionError::Stale)?;
    if taxonomy.status != TaxonomyStatus::Draft {
        return Err(TransitionError::NotDraft);
    }
    Ok(taxonomy)
}

pub fn edit_task(
    state: &AgentState,
    task_id: &str,
    edit: &TaskEdit,
) -> Result<Changes, TransitionError> {
    let name = edit.name.trim();
    if name.is_empty()
        || name.chars().count() > NAME_LIMIT
        || [
            &edit.description,
            &edit.inclusion_criteria,
            &edit.exclusion_criteria,
        ]
        .iter()
        .any(|text| text.chars().count() > TEXT_LIMIT)
    {
        return Err(TransitionError::InvalidEdit);
    }
    let taxonomy = draft(state, edit.revision)?;
    if !taxonomy.tasks.iter().any(|task| task.task_id == task_id) {
        return Err(TransitionError::MissingTask);
    }
    let tasks = taxonomy
        .tasks
        .iter()
        .map(|task| {
            if task.task_id != task_id {
                return task.clone();
            }
            TaskDefinition {
                task_id: task.task_id.clone(),
                name: name.to_owned(),
                description: edit.description.trim().to_owned(),
                inclusion_criteria: edit.inclusion_criteria.trim().to_owned(),
                exclusion_criteria: edit.exclusion_criteria.trim().to_owned(),
                required_capabilities: edit.required_capabilities.clone(),
                example_trace_ids: task.example_trace_ids.clone(),
            }
        })
        .collect();
    Ok(Changes {
        working: Some(next(Taxonomy { tasks, ..taxonomy })),
        ..Changes::default()
    })
}

/// Publishes the reviewed draft. Live taxonomies classify runs that start from now on, plus the
/// draft's own discovery range when `backfill` is set; sample taxonomies never classify.
pub fn activate(
    state: &AgentState,
    request: &ActivationRequest,
    now: DateTime<Utc>,
) -> Result<Changes, TransitionError> {
    let taxonomy = draft(state, request.revision)?;
    if taxonomy.tasks.is_empty() {
        return Err(TransitionError::Empty);
    }
    if !taxonomy.sample && !state.enabled() {
        return Err(TransitionError::Disabled);
    }
    let backfill = match (request.backfill, &taxonomy.discovery) {
        (false, _) => None,
        (true, Some(run)) => Some(ClassificationWindow {
            start_ms: run.start_ms,
            end_ms: Some(run.end_ms),
        }),
        (true, None) => return Err(TransitionError::NoRange),
    };
    let windows = if taxonomy.sample {
        Vec::new()
    } else {
        backfill
            .into_iter()
            .chain([ClassificationWindow {
                start_ms: now.timestamp_millis(),
                end_ms: None,
            }])
            .collect()
    };
    let revision = state
        .active()
        .map_or(taxonomy.revision, |active| {
            active.revision.max(taxonomy.revision)
        })
        .saturating_add(1);
    Ok(Changes {
        working: Some(cleared(&taxonomy)),
        active: Some(Taxonomy {
            status: TaxonomyStatus::Frozen,
            frozen_at: Some(now),
            job: None,
            windows,
            revision,
            ..taxonomy
        }),
        settings: None,
    })
}

pub fn resume(
    state: &AgentState,
    request: &TaxonomyRevisionRequest,
    now: DateTime<Utc>,
) -> Result<Changes, TransitionError> {
    if !state.enabled() {
        return Err(TransitionError::Disabled);
    }
    let active = state
        .active()
        .filter(|active| !active.sample)
        .ok_or(TransitionError::NotActive)?;
    if active.revision != request.revision {
        return Err(TransitionError::Stale);
    }
    if open(active) {
        return Err(TransitionError::AlreadyClassifying);
    }
    let windows = active
        .windows
        .iter()
        .cloned()
        .chain([ClassificationWindow {
            start_ms: now.timestamp_millis(),
            end_ms: None,
        }])
        .collect();
    Ok(with_active(
        state,
        next(Taxonomy {
            windows,
            ..active.clone()
        }),
    ))
}

pub fn reset(state: &AgentState, request: &ResetRequest) -> Result<Changes, TransitionError> {
    match request.target {
        TaxonomyTarget::Draft => {
            let working = state
                .under_review()
                .filter(|working| working.revision == request.revision)
                .ok_or(TransitionError::Stale)?;
            Ok(Changes {
                working: Some(cleared(working)),
                ..Changes::default()
            })
        }
        TaxonomyTarget::Active => {
            let active = state
                .active()
                .filter(|active| active.revision == request.revision)
                .ok_or(TransitionError::Stale)?;
            Ok(with_active(state, cleared(active)))
        }
    }
}

/// Saving never starts model calls. Turning discovery off stops classification of new runs and
/// cancels a running discovery; turning it on again leaves both stopped until asked.
pub fn update_settings(
    state: &AgentState,
    update: &ProfileSettingsUpdate,
    options: &DiscoveryModelOptions,
    now: DateTime<Utc>,
) -> Result<Changes, TransitionError> {
    let current = state.settings.clone().unwrap_or_default();
    if current.revision != update.revision {
        return Err(TransitionError::Stale);
    }
    let models = trimmed(&update.models);
    if update.enabled {
        validate_models(&models, options)?;
    }
    let settings = ProfileSettings {
        agent: update.agent.clone(),
        enabled: update.enabled,
        models,
        revision: current.revision.saturating_add(1),
        updated_at: Some(now),
    };
    if update.enabled {
        return Ok(Changes {
            settings: Some(settings),
            ..Changes::default()
        });
    }
    let closed = state.active().filter(|active| open(active)).map(|active| {
        next(Taxonomy {
            windows: active
                .windows
                .iter()
                .map(|window| ClassificationWindow {
                    start_ms: window.start_ms,
                    end_ms: Some(window.end_ms.unwrap_or_else(|| now.timestamp_millis())),
                })
                .collect(),
            ..active.clone()
        })
    });
    let cancelled = state
        .working
        .as_ref()
        .filter(|working| working.status == TaxonomyStatus::Discovering)
        .map(|working| {
            next(Taxonomy {
                status: TaxonomyStatus::Failed,
                job: None,
                error: Some(CANCELLED.into()),
                ..working.clone()
            })
        });
    let moved = closed.map(|active| with_active(state, active));
    Ok(Changes {
        working: cancelled.or_else(|| moved.as_ref().and_then(|changes| changes.working.clone())),
        active: moved.and_then(|changes| changes.active),
        settings: Some(settings),
    })
}

/// The worker's lease on a queued or abandoned discovery job. `None` when there is nothing to
/// claim; a job past its attempt budget is marked failed instead.
pub fn claim(current: &Taxonomy, now: DateTime<Utc>) -> Option<Taxonomy> {
    let job = current.job.as_ref()?;
    if current.status != TaxonomyStatus::Discovering
        || job.lease_until.is_some_and(|lease| lease > now)
    {
        return None;
    }
    if job.attempts >= MAX_ATTEMPTS {
        return Some(next(Taxonomy {
            status: TaxonomyStatus::Failed,
            error: Some("Discovery stopped after repeated failures".into()),
            job: None,
            ..current.clone()
        }));
    }
    Some(next(Taxonomy {
        job: Some(DiscoveryJob {
            lease_until: Some(now + LEASE),
            attempts: job.attempts + 1,
            ..job.clone()
        }),
        ..current.clone()
    }))
}

/// Stores a finished discovery only if the attempt that ran it still holds the job; a reclaimed,
/// replaced or cancelled job's result is dropped.
pub fn complete(
    current: Option<Taxonomy>,
    job_id: &str,
    attempt: u32,
    outcome: Result<(Vec<TaskDefinition>, Vec<DiscoveryPoint>), String>,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current.ok_or(TransitionError::Stale)?;
    let job = taxonomy
        .job
        .clone()
        .filter(|job| {
            taxonomy.status == TaxonomyStatus::Discovering
                && job.job_id == job_id
                && job.attempts == attempt
        })
        .ok_or(TransitionError::Stale)?;
    Ok(next(match outcome {
        Ok((tasks, points)) => Taxonomy {
            status: TaxonomyStatus::Draft,
            discovery: Some(DiscoveryRun {
                start_ms: job.start_ms,
                end_ms: job.end_ms,
                requested_runs: job.max_runs,
                sampled_runs: u32::try_from(points.len()).unwrap_or(u32::MAX),
            }),
            tasks,
            points,
            job: None,
            discovered_at: Some(now),
            error: None,
            ..taxonomy
        },
        Err(error) => Taxonomy {
            status: TaxonomyStatus::Failed,
            job: None,
            error: Some(error),
            ..taxonomy
        },
    }))
}

pub fn can_write(identity: &Identity) -> bool {
    identity.user_role == Role::ProxyAdmin
}

fn can_read(identity: &Identity) -> bool {
    matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    )
}

impl From<TransitionError> for ProfileHttpError {
    fn from(error: TransitionError) -> Self {
        match error {
            TransitionError::Running | TransitionError::Stale => {
                ProfileHttpError::Store(ProfileStoreError::Conflict)
            }
            TransitionError::NotDraft => {
                ProfileHttpError::Invalid("Only a draft taxonomy can be edited or activated")
            }
            TransitionError::Empty => ProfileHttpError::Invalid(
                "A draft needs at least one task before it can be activated",
            ),
            TransitionError::MissingTask | TransitionError::NotActive => ProfileHttpError::NotFound,
            TransitionError::InvalidEdit => ProfileHttpError::Invalid(
                "Task names must be 1-80 characters and other fields at most 600",
            ),
            TransitionError::InvalidSize => ProfileHttpError::Invalid(
                "Discovery reads between 20 and 500 runs from a non-empty range",
            ),
            TransitionError::Disabled => {
                ProfileHttpError::Invalid("Turn on task discovery for this agent first")
            }
            TransitionError::Unconfigured => ProfileHttpError::Invalid(
                "Choose a model for each step before turning on task discovery",
            ),
            TransitionError::UnavailableModel(ModelRole::Discovery) => ProfileHttpError::Invalid(
                "The task discovery model is not an available chat model on the gateway",
            ),
            TransitionError::UnavailableModel(ModelRole::Embedding) => ProfileHttpError::Invalid(
                "The embedding model is not an available embedding model on the gateway",
            ),
            TransitionError::UnavailableModel(ModelRole::Classification) => {
                ProfileHttpError::Invalid(
                    "The classification model is not an available Jev decisions model on the gateway",
                )
            }
            TransitionError::AlreadyClassifying => {
                ProfileHttpError::Invalid("Classification is already running for new runs")
            }
            TransitionError::NoRange => {
                ProfileHttpError::Invalid("This draft has no discovery range to classify")
            }
        }
    }
}

type Profiles<R, S> = State<Arc<App<R, S>>>;

pub(super) fn router<
    R: SessionRepository + 'static,
    S: ProfileStore + TaxonomyStore + DiscoveryCatalog + 'static,
>() -> Router<Arc<App<R, S>>> {
    Router::new()
        .public_route(
            "/lens/profiles/settings",
            get(settings_view::<R, S>).put(settings_update::<R, S>),
        )
        .public_route("/lens/profiles/discovery/preview", post(preview::<R, S>))
        .public_route("/lens/profiles/discovery", post(discover::<R, S>))
        .public_route(
            "/lens/profiles/taxonomy/tasks/{task_id}",
            patch(edit::<R, S>),
        )
        .public_route(
            "/lens/profiles/taxonomy/activate",
            post(activate_route::<R, S>),
        )
        .public_route("/lens/profiles/taxonomy/resume", post(resume_route::<R, S>))
        .public_route("/lens/profiles/taxonomy/reset", post(reset_route::<R, S>))
}

const ADMINS_ONLY: &str = "Only proxy admins who can read every team can change task discovery";
const READERS_ONLY: &str = "Only proxy admins who can read every team can view task discovery";

async fn writer<R: SessionRepository, S>(
    app: &App<R, S>,
    headers: &HeaderMap,
    method: &Method,
    agent: &str,
) -> Result<String, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, headers, method).await?;
    if !can_write(&identity) {
        return Err(ProfileHttpError::Forbidden(ADMINS_ONLY));
    }
    taxonomy_id(&scope(&identity)?, agent).ok_or(ProfileHttpError::Forbidden(ADMINS_ONLY))
}

async fn reader<R: SessionRepository, S>(
    app: &App<R, S>,
    headers: &HeaderMap,
    method: &Method,
    agent: &str,
) -> Result<(String, Identity), ProfileHttpError> {
    let identity = auth::identity(&app.authentication, headers, method).await?;
    if !can_read(&identity) {
        return Err(ProfileHttpError::Forbidden(READERS_ONLY));
    }
    let id =
        taxonomy_id(&scope(&identity)?, agent).ok_or(ProfileHttpError::Forbidden(READERS_ONLY))?;
    Ok((id, identity))
}

async fn apply<R, S: TaxonomyStore>(
    app: &App<R, S>,
    id: &str,
    transition: impl FnOnce(&AgentState) -> Result<Changes, TransitionError>,
) -> Result<AgentState, ProfileHttpError> {
    let (state, heads) = app.store.load(id).await?;
    let changes = transition(&state)?;
    app.store.save(id, heads, changes.clone()).await?;
    Ok(AgentState {
        working: changes.working.or(state.working),
        active: changes.active.or(state.active),
        settings: changes.settings.or(state.settings),
    })
}

fn view(state: &AgentState) -> lens_contract::profiles::TaxonomyView {
    lens_contract::profiles::TaxonomyView {
        taxonomy: state.under_review().cloned(),
        active: state.active().cloned(),
        discovery_available: state.enabled() && state.models().is_some(),
        can_write: true,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentQuery {
    agent: String,
}

async fn settings_view<R: SessionRepository, S: TaxonomyStore + DiscoveryCatalog>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<AgentQuery>,
) -> Result<Json<ProfileSettingsView>, ProfileHttpError> {
    let (id, identity) = reader(&app, &headers, &method, &query.agent).await?;
    let (state, _) = app.store.load(&id).await?;
    Ok(Json(ProfileSettingsView {
        settings: state.settings.unwrap_or_else(|| ProfileSettings {
            agent: query.agent,
            ..ProfileSettings::default()
        }),
        defaults: app.defaults.clone(),
        options: app.store.options().await,
        can_write: can_write(&identity),
    }))
}

async fn settings_update<R: SessionRepository, S: TaxonomyStore + DiscoveryCatalog>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(update): Json<ProfileSettingsUpdate>,
) -> Result<Json<ProfileSettings>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &update.agent).await?;
    let options = if update.enabled {
        app.store.options().await
    } else {
        DiscoveryModelOptions::default()
    };
    let saved = apply(&app, &id, |state| {
        update_settings(state, &update, &options, Utc::now())
    })
    .await?;
    saved
        .settings
        .map(Json)
        .ok_or(ProfileHttpError::Store(ProfileStoreError::Unavailable))
}

async fn preview<R: SessionRepository, S: DiscoveryCatalog>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<DiscoveryRequest>,
) -> Result<Json<DiscoveryPreview>, ProfileHttpError> {
    reader(&app, &headers, &method, &request.agent).await?;
    if !(MIN_DISCOVERY_RUNS..=MAX_DISCOVERY_RUNS).contains(&request.max_runs)
        || request.end_ms <= request.start_ms
    {
        return Err(TransitionError::InvalidSize.into());
    }
    Ok(Json(app.store.preview(request).await?))
}

async fn discover<R: SessionRepository, S: TaxonomyStore + DiscoveryCatalog>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<DiscoveryRequest>,
) -> Result<Json<lens_contract::profiles::TaxonomyView>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let options = app.store.options().await;
    let state = apply(&app, &id, |state| {
        let models = state
            .settings
            .as_ref()
            .filter(|settings| settings.enabled)
            .ok_or(TransitionError::Disabled)?;
        validate_models(&models.models, &options)?;
        start_discovery(
            state,
            &id,
            &request,
            uuid::Uuid::new_v4().to_string(),
            Utc::now(),
        )
    })
    .await?;
    Ok(Json(view(&state)))
}

async fn edit<R: SessionRepository, S: TaxonomyStore>(
    State(app): Profiles<R, S>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<TaskEdit>,
) -> Result<Json<lens_contract::profiles::TaxonomyView>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let state = apply(&app, &id, |state| edit_task(state, &task_id, &request)).await?;
    Ok(Json(view(&state)))
}

async fn activate_route<R: SessionRepository, S: TaxonomyStore>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<ActivationRequest>,
) -> Result<Json<lens_contract::profiles::TaxonomyView>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let state = apply(&app, &id, |state| activate(state, &request, Utc::now())).await?;
    Ok(Json(view(&state)))
}

async fn resume_route<R: SessionRepository, S: TaxonomyStore>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<TaxonomyRevisionRequest>,
) -> Result<Json<lens_contract::profiles::TaxonomyView>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let state = apply(&app, &id, |state| resume(state, &request, Utc::now())).await?;
    Ok(Json(view(&state)))
}

async fn reset_route<R: SessionRepository, S: TaxonomyStore>(
    State(app): Profiles<R, S>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<ResetRequest>,
) -> Result<Json<lens_contract::profiles::TaxonomyView>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let state = apply(&app, &id, |state| reset(state, &request)).await?;
    Ok(Json(view(&state)))
}

pub(super) fn taxonomy_view(
    state: &AgentState,
    identity: &Identity,
) -> lens_contract::profiles::TaxonomyView {
    lens_contract::profiles::TaxonomyView {
        can_write: can_write(identity),
        ..view(state)
    }
}
