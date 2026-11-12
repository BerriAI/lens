use std::{future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, Method},
    routing::{patch, post},
};
use chrono::{DateTime, Duration, Utc};
use lens_auth::SessionRepository;
use lens_contract::{
    auth::{Identity, Role},
    profiles::{
        DiscoveryJob, DiscoveryPoint, DiscoveryRequest, TaskDefinition, TaskEdit, Taxonomy,
        TaxonomyModels, TaxonomyRevisionRequest, TaxonomyStatus,
    },
};

use super::{App, ProfileHttpError, ProfileStore, ProfileStoreError, scope, taxonomy_id};
use crate::{auth, routing::PublicRoutes};

pub const MIN_DISCOVERY_RUNS: u32 = 20;
pub const MAX_DISCOVERY_RUNS: u32 = 500;
pub const MAX_ATTEMPTS: u32 = 3;
pub const LEASE: Duration = Duration::minutes(30);
const NAME_LIMIT: usize = 80;
const TEXT_LIMIT: usize = 600;

pub trait TaxonomyStore: Send + Sync {
    type Head: Send;

    fn read(
        &self,
        taxonomy_id: &str,
    ) -> impl Future<Output = Result<(Option<Taxonomy>, Self::Head), ProfileStoreError>> + Send;

    /// Replaces the document only if it is still at `head`; otherwise `Conflict`.
    fn write(
        &self,
        head: Self::Head,
        next: Option<Taxonomy>,
    ) -> impl Future<Output = Result<(), ProfileStoreError>> + Send;
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum TransitionError {
    #[error("Discovery is already running for this agent")]
    Running,
    #[error("The taxonomy changed since it was loaded; reload and try again")]
    Stale,
    #[error("Only a draft taxonomy can be edited or frozen")]
    NotDraft,
    #[error("A draft needs at least one task before it can be frozen")]
    Empty,
    #[error("Task not found in this draft")]
    MissingTask,
    #[error("Task names must be 1-80 characters and other fields at most 600")]
    InvalidEdit,
    #[error("Discovery reads between 20 and 500 runs")]
    InvalidSize,
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

pub fn start_discovery(
    current: Option<&Taxonomy>,
    taxonomy_id: &str,
    request: &DiscoveryRequest,
    models: &TaxonomyModels,
    job_id: String,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    if !(MIN_DISCOVERY_RUNS..=MAX_DISCOVERY_RUNS).contains(&request.max_runs)
        || request.end_ms <= request.start_ms
    {
        return Err(TransitionError::InvalidSize);
    }
    if current.is_some_and(|taxonomy| active_job(taxonomy, now)) {
        return Err(TransitionError::Running);
    }
    Ok(Taxonomy {
        taxonomy_id: taxonomy_id.to_owned(),
        agent: request.agent.clone(),
        version: current.map_or(1, |taxonomy| taxonomy.version.saturating_add(1)),
        revision: current.map_or(1, |taxonomy| taxonomy.revision.saturating_add(1)),
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
        models: models.clone(),
    })
}

fn current_revision(current: Option<Taxonomy>, revision: u32) -> Result<Taxonomy, TransitionError> {
    current
        .filter(|taxonomy| taxonomy.revision == revision)
        .ok_or(TransitionError::Stale)
}

fn draft(current: Option<Taxonomy>, revision: u32) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current_revision(current, revision)?;
    if taxonomy.status != TaxonomyStatus::Draft {
        return Err(TransitionError::NotDraft);
    }
    Ok(taxonomy)
}

pub fn edit_task(
    current: Option<Taxonomy>,
    task_id: &str,
    edit: &TaskEdit,
) -> Result<Taxonomy, TransitionError> {
    let name = edit.name.trim();
    if name.is_empty()
        || name.chars().count() > NAME_LIMIT
        || [&edit.description, &edit.inclusion_criteria, &edit.exclusion_criteria]
            .iter()
            .any(|text| text.chars().count() > TEXT_LIMIT)
    {
        return Err(TransitionError::InvalidEdit);
    }
    let mut taxonomy = draft(current, edit.revision)?;
    let task = taxonomy
        .tasks
        .iter_mut()
        .find(|task| task.task_id == task_id)
        .ok_or(TransitionError::MissingTask)?;
    *task = TaskDefinition {
        task_id: task.task_id.clone(),
        name: name.to_owned(),
        description: edit.description.trim().to_owned(),
        inclusion_criteria: edit.inclusion_criteria.trim().to_owned(),
        exclusion_criteria: edit.exclusion_criteria.trim().to_owned(),
        required_capabilities: edit.required_capabilities.clone(),
        example_trace_ids: task.example_trace_ids.clone(),
    };
    Ok(next(taxonomy))
}

pub fn freeze(
    current: Option<Taxonomy>,
    request: &TaxonomyRevisionRequest,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = draft(current, request.revision)?;
    if taxonomy.tasks.is_empty() {
        return Err(TransitionError::Empty);
    }
    Ok(next(Taxonomy {
        status: TaxonomyStatus::Frozen,
        frozen_at: Some(now),
        ..taxonomy
    }))
}

/// Clears the taxonomy but keeps its generation, so a later discovery never reuses a version
/// whose assignments are still stored.
pub fn reset(
    current: Option<Taxonomy>,
    request: &TaxonomyRevisionRequest,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current_revision(current, request.revision)?;
    Ok(next(Taxonomy {
        status: TaxonomyStatus::Cleared,
        tasks: Vec::new(),
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        ..taxonomy
    }))
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

/// Stores a finished discovery only if the attempt that ran it still holds the job; a reclaimed
/// job's older attempt cannot overwrite the newer one.
pub fn complete(
    current: Option<Taxonomy>,
    job_id: &str,
    attempt: u32,
    outcome: Result<(Vec<TaskDefinition>, Vec<DiscoveryPoint>), String>,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current.ok_or(TransitionError::Stale)?;
    if taxonomy.status != TaxonomyStatus::Discovering
        || taxonomy
            .job
            .as_ref()
            .is_none_or(|job| job.job_id != job_id || job.attempts != attempt)
    {
        return Err(TransitionError::Stale);
    }
    Ok(next(match outcome {
        Ok((tasks, points)) => Taxonomy {
            status: TaxonomyStatus::Draft,
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

impl From<TransitionError> for ProfileHttpError {
    fn from(error: TransitionError) -> Self {
        match error {
            TransitionError::Running | TransitionError::Stale => {
                ProfileHttpError::Store(ProfileStoreError::Conflict)
            }
            TransitionError::NotDraft => ProfileHttpError::Invalid("Only a draft taxonomy can be edited or frozen"),
            TransitionError::Empty => ProfileHttpError::Invalid("A draft needs at least one task before it can be frozen"),
            TransitionError::MissingTask => ProfileHttpError::NotFound,
            TransitionError::InvalidEdit => ProfileHttpError::Invalid("Task names must be 1-80 characters and other fields at most 600"),
            TransitionError::InvalidSize => ProfileHttpError::Invalid("Discovery reads between 20 and 500 runs"),
        }
    }
}

pub(super) fn router<R: SessionRepository + 'static, S: ProfileStore + TaxonomyStore + 'static>()
-> Router<Arc<App<R, S>>> {
    Router::new()
        .public_route("/lens/profiles/discovery", post(discover::<R, S>))
        .public_route(
            "/lens/profiles/taxonomy/tasks/{task_id}",
            patch(edit::<R, S>),
        )
        .public_route("/lens/profiles/taxonomy/freeze", post(freeze_route::<R, S>))
        .public_route("/lens/profiles/taxonomy/reset", post(reset_route::<R, S>))
}

async fn writer<R: SessionRepository, S>(
    app: &App<R, S>,
    headers: &HeaderMap,
    method: &Method,
    agent: &str,
) -> Result<String, ProfileHttpError> {
    let identity = auth::identity(&app.authentication, headers, method).await?;
    if !can_write(&identity) {
        return Err(ProfileHttpError::Forbidden(
            "Only proxy admins can change task profiles",
        ));
    }
    taxonomy_id(&scope(&identity)?, agent)
        .ok_or(ProfileHttpError::Forbidden("Only proxy admins can change task profiles"))
}

async fn discover<R: SessionRepository, S: ProfileStore + TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<DiscoveryRequest>,
) -> Result<Json<Taxonomy>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let models = app
        .discovery
        .clone()
        .ok_or(ProfileHttpError::Invalid("Enhanced discovery is not configured on this Lens deployment"))?;
    let (current, head) = app.store.read(&id).await?;
    let next = start_discovery(
        current.as_ref(),
        &id,
        &request,
        &models,
        uuid::Uuid::new_v4().to_string(),
        Utc::now(),
    )?;
    app.store.write(head, Some(next.clone())).await?;
    Ok(Json(next))
}

async fn edit<R: SessionRepository, S: ProfileStore + TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<TaskEdit>,
) -> Result<Json<Taxonomy>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let (current, head) = app.store.read(&id).await?;
    let next = edit_task(current, &task_id, &request)?;
    app.store.write(head, Some(next.clone())).await?;
    Ok(Json(next))
}

async fn freeze_route<R: SessionRepository, S: ProfileStore + TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<TaxonomyRevisionRequest>,
) -> Result<Json<Taxonomy>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let (current, head) = app.store.read(&id).await?;
    let next = freeze(current, &request, Utc::now())?;
    app.store.write(head, Some(next.clone())).await?;
    Ok(Json(next))
}

async fn reset_route<R: SessionRepository, S: ProfileStore + TaxonomyStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<TaxonomyRevisionRequest>,
) -> Result<Json<Taxonomy>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let (current, head) = app.store.read(&id).await?;
    let next = reset(current, &request)?;
    app.store.write(head, Some(next.clone())).await?;
    Ok(Json(next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("time")
    }

    fn request(max_runs: u32) -> DiscoveryRequest {
        DiscoveryRequest {
            agent: "agent".into(),
            start_ms: 0,
            end_ms: 1,
            max_runs,
        }
    }

    fn task(task_id: &str) -> TaskDefinition {
        TaskDefinition {
            task_id: task_id.into(),
            name: "Debug failing tests".into(),
            description: String::new(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
            example_trace_ids: vec!["trace".into()],
        }
    }

    fn start(current: Option<&Taxonomy>, job_id: &str) -> Result<Taxonomy, TransitionError> {
        start_discovery(current, "tax", &request(100), &TaxonomyModels::default(), job_id.into(), now())
    }

    fn revision(taxonomy: &Taxonomy) -> TaxonomyRevisionRequest {
        TaxonomyRevisionRequest {
            agent: "agent".into(),
            revision: taxonomy.revision,
        }
    }

    fn rename(taxonomy: &Taxonomy, name: &str) -> TaskEdit {
        TaskEdit {
            agent: "agent".into(),
            revision: taxonomy.revision,
            name: name.into(),
            description: String::new(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
        }
    }

    #[fixture]
    fn discovering() -> Taxonomy {
        start(None, "job").expect("start")
    }

    #[fixture]
    fn drafted(discovering: Taxonomy) -> Taxonomy {
        let claimed = claim(&discovering, now()).expect("claim");
        complete(Some(claimed), "job", 1, Ok((vec![task("a")], Vec::new())), now()).expect("draft")
    }

    #[rstest]
    fn a_second_discovery_is_refused_while_one_is_running(discovering: Taxonomy) {
        assert_eq!(start(Some(&discovering), "job-2"), Err(TransitionError::Running));
    }

    #[rstest]
    fn reset_then_rediscovery_never_reuses_a_generation_with_stored_assignments(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &revision(&drafted), now()).expect("freeze");
        let cleared = reset(Some(frozen.clone()), &revision(&frozen)).expect("reset");

        let rediscovered = start(Some(&cleared), "job-2").expect("rediscover");

        assert_eq!(cleared.status, TaxonomyStatus::Cleared);
        assert_eq!(cleared.frozen_at, None);
        assert!(rediscovered.version > frozen.version);
    }

    #[rstest]
    #[case::too_small(19)]
    #[case::too_large(501)]
    fn discovery_size_is_bounded(#[case] max_runs: u32) {
        let result = start_discovery(None, "tax", &request(max_runs), &TaxonomyModels::default(), "job".into(), now());

        assert_eq!(result, Err(TransitionError::InvalidSize));
    }

    #[rstest]
    fn a_running_discovery_cannot_be_frozen(discovering: Taxonomy) {
        assert_eq!(
            freeze(Some(discovering.clone()), &revision(&discovering), now()),
            Err(TransitionError::NotDraft)
        );
    }

    #[rstest]
    fn a_stale_browser_edit_cannot_overwrite_a_newer_one(drafted: Taxonomy) {
        let first = edit_task(Some(drafted.clone()), "a", &rename(&drafted, "Fix flaky tests")).expect("first edit");

        let stale = edit_task(Some(first.clone()), "a", &rename(&drafted, "Something else"));
        let stale_freeze = freeze(Some(first.clone()), &revision(&drafted), now());

        assert_eq!(stale, Err(TransitionError::Stale));
        assert_eq!(stale_freeze, Err(TransitionError::Stale));
        assert_eq!(first.tasks[0].name, "Fix flaky tests");
        assert!(first.revision > drafted.revision);
    }

    #[rstest]
    fn freezing_a_draft_records_when(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &revision(&drafted), now()).expect("freeze");

        assert_eq!(frozen.status, TaxonomyStatus::Frozen);
        assert_eq!(frozen.frozen_at, Some(now()));
        assert_eq!(frozen.version, drafted.version);
    }

    #[rstest]
    fn a_frozen_taxonomy_cannot_be_edited(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &revision(&drafted), now()).expect("freeze");

        assert_eq!(
            edit_task(Some(frozen.clone()), "a", &rename(&frozen, "Renamed")),
            Err(TransitionError::NotDraft)
        );
    }

    #[rstest]
    #[case::blank("  ", Err(TransitionError::InvalidEdit))]
    #[case::renamed(" Fix flaky tests ", Ok("Fix flaky tests"))]
    fn draft_edits_trim_and_validate_names(
        drafted: Taxonomy,
        #[case] name: &str,
        #[case] expected: Result<&str, TransitionError>,
    ) {
        let result = edit_task(Some(drafted.clone()), "a", &rename(&drafted, name))
            .map(|taxonomy| taxonomy.tasks[0].name.clone());

        assert_eq!(result.as_deref().map_err(Clone::clone), expected);
    }

    #[rstest]
    fn an_older_attempt_cannot_finish_a_reclaimed_job(discovering: Taxonomy) {
        let first = claim(&discovering, now()).expect("first claim");
        let second = claim(&first, now() + LEASE + Duration::seconds(1)).expect("reclaim");

        let late = complete(Some(second.clone()), "job", 1, Ok((vec![task("a")], Vec::new())), now());
        let current = complete(Some(second), "job", 2, Ok((vec![task("a")], Vec::new())), now());

        assert_eq!(late, Err(TransitionError::Stale));
        assert_eq!(current.map(|taxonomy| taxonomy.status), Ok(TaxonomyStatus::Draft));
    }

    #[rstest]
    fn a_result_from_a_replaced_job_is_dropped(discovering: Taxonomy) {
        let claimed = claim(&discovering, now()).expect("claim");

        let result = complete(Some(claimed), "older-job", 1, Ok((vec![task("a")], Vec::new())), now());

        assert_eq!(result, Err(TransitionError::Stale));
    }

    #[rstest]
    fn a_held_lease_is_not_reclaimed_and_exhausted_jobs_fail(discovering: Taxonomy) {
        let first = claim(&discovering, now()).expect("first claim");
        let later = |leases: i32| now() + LEASE * leases + Duration::seconds(1);
        let second = claim(&first, later(1)).expect("second");
        let third = claim(&second, later(2)).expect("third");
        let exhausted = claim(&third, later(3)).expect("exhausted");

        assert_eq!(claim(&first, now()), None);
        assert_eq!(exhausted.status, TaxonomyStatus::Failed);
    }
}
