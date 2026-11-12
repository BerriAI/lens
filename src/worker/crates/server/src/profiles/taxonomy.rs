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
        TaxonomyModels, TaxonomyStatus, TaxonomyVersionRequest,
    },
};

use super::{App, ProfileHttpError, ProfileStore, ProfileStoreError, scope, taxonomy_id};
use crate::{auth, routing::PublicRoutes};

pub const MIN_DISCOVERY_RUNS: u32 = 20;
pub const MAX_DISCOVERY_RUNS: u32 = 500;
pub const MAX_ATTEMPTS: u32 = 3;
pub const LEASE: Duration = Duration::minutes(10);
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

fn draft(current: Option<Taxonomy>, version: u32) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current.ok_or(TransitionError::Stale)?;
    if taxonomy.version != version {
        return Err(TransitionError::Stale);
    }
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
    let mut taxonomy = draft(current, edit.version)?;
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
    Ok(taxonomy)
}

pub fn freeze(
    current: Option<Taxonomy>,
    request: &TaxonomyVersionRequest,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = draft(current, request.version)?;
    if taxonomy.tasks.is_empty() {
        return Err(TransitionError::Empty);
    }
    Ok(Taxonomy {
        status: TaxonomyStatus::Frozen,
        frozen_at: Some(now),
        ..taxonomy
    })
}

pub fn reset(
    current: Option<&Taxonomy>,
    request: &TaxonomyVersionRequest,
) -> Result<(), TransitionError> {
    match current {
        Some(taxonomy) if taxonomy.version == request.version => Ok(()),
        _ => Err(TransitionError::Stale),
    }
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
        return Some(Taxonomy {
            status: TaxonomyStatus::Failed,
            error: Some("Discovery stopped after repeated failures".into()),
            job: None,
            ..current.clone()
        });
    }
    Some(Taxonomy {
        job: Some(DiscoveryJob {
            lease_until: Some(now + LEASE),
            attempts: job.attempts + 1,
            ..job.clone()
        }),
        ..current.clone()
    })
}

/// Stores a finished discovery only if the job is still the one this version started.
pub fn complete(
    current: Option<Taxonomy>,
    job_id: &str,
    outcome: Result<(Vec<TaskDefinition>, Vec<DiscoveryPoint>), String>,
    now: DateTime<Utc>,
) -> Result<Taxonomy, TransitionError> {
    let taxonomy = current.ok_or(TransitionError::Stale)?;
    if taxonomy.status != TaxonomyStatus::Discovering
        || taxonomy.job.as_ref().is_none_or(|job| job.job_id != job_id)
    {
        return Err(TransitionError::Stale);
    }
    Ok(match outcome {
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
    })
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
    Json(request): Json<TaxonomyVersionRequest>,
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
    Json(request): Json<TaxonomyVersionRequest>,
) -> Result<Json<serde_json::Value>, ProfileHttpError> {
    let id = writer(&app, &headers, &method, &request.agent).await?;
    let (current, head) = app.store.read(&id).await?;
    reset(current.as_ref(), &request)?;
    app.store.write(head, None).await?;
    Ok(Json(serde_json::json!({"reset": true})))
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

    #[fixture]
    fn discovering() -> Taxonomy {
        start_discovery(None, "tax", &request(100), &TaxonomyModels::default(), "job".into(), now())
            .expect("start")
    }

    #[fixture]
    fn drafted(discovering: Taxonomy) -> Taxonomy {
        complete(Some(discovering), "job", Ok((vec![task("a")], Vec::new())), now()).expect("draft")
    }

    fn version(taxonomy: &Taxonomy) -> TaxonomyVersionRequest {
        TaxonomyVersionRequest {
            agent: "agent".into(),
            version: taxonomy.version,
        }
    }

    #[rstest]
    fn a_second_discovery_is_refused_while_one_is_running(discovering: Taxonomy) {
        let result = start_discovery(
            Some(&discovering),
            "tax",
            &request(100),
            &TaxonomyModels::default(),
            "job-2".into(),
            now(),
        );

        assert_eq!(result, Err(TransitionError::Running));
    }

    #[rstest]
    fn rediscovery_bumps_the_version_so_old_assignments_stop_applying(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &version(&drafted), now()).expect("freeze");

        let next = start_discovery(
            Some(&frozen),
            "tax",
            &request(100),
            &TaxonomyModels::default(),
            "job-2".into(),
            now(),
        )
        .expect("rediscover");

        assert_eq!(next.version, frozen.version + 1);
        assert_eq!(next.frozen_at, None);
    }

    #[rstest]
    #[case::too_small(19)]
    #[case::too_large(501)]
    fn discovery_size_is_bounded(#[case] max_runs: u32) {
        let result = start_discovery(
            None,
            "tax",
            &request(max_runs),
            &TaxonomyModels::default(),
            "job".into(),
            now(),
        );

        assert_eq!(result, Err(TransitionError::InvalidSize));
    }

    #[rstest]
    fn a_running_discovery_cannot_be_frozen(discovering: Taxonomy) {
        let result = freeze(Some(discovering.clone()), &version(&discovering), now());

        assert_eq!(result, Err(TransitionError::NotDraft));
    }

    #[rstest]
    fn freezing_an_older_version_is_rejected_as_stale(drafted: Taxonomy) {
        let stale = TaxonomyVersionRequest {
            agent: "agent".into(),
            version: drafted.version + 1,
        };

        assert_eq!(freeze(Some(drafted), &stale, now()), Err(TransitionError::Stale));
    }

    #[rstest]
    fn freezing_a_draft_records_when(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &version(&drafted), now()).expect("freeze");

        assert_eq!(frozen.status, TaxonomyStatus::Frozen);
        assert_eq!(frozen.frozen_at, Some(now()));
    }

    #[rstest]
    fn a_frozen_taxonomy_cannot_be_edited(drafted: Taxonomy) {
        let frozen = freeze(Some(drafted.clone()), &version(&drafted), now()).expect("freeze");
        let edit = TaskEdit {
            agent: "agent".into(),
            version: frozen.version,
            name: "Renamed".into(),
            description: String::new(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
        };

        assert_eq!(edit_task(Some(frozen), "a", &edit), Err(TransitionError::NotDraft));
    }

    #[rstest]
    #[case::blank("  ", Err(TransitionError::InvalidEdit))]
    #[case::renamed(" Fix flaky tests ", Ok("Fix flaky tests"))]
    fn draft_edits_trim_and_validate_names(
        drafted: Taxonomy,
        #[case] name: &str,
        #[case] expected: Result<&str, TransitionError>,
    ) {
        let edit = TaskEdit {
            agent: "agent".into(),
            version: drafted.version,
            name: name.into(),
            description: String::new(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
        };

        let result = edit_task(Some(drafted), "a", &edit).map(|taxonomy| taxonomy.tasks[0].name.clone());

        assert_eq!(result.as_deref().map_err(Clone::clone), expected);
    }

    #[rstest]
    fn a_stale_job_result_does_not_overwrite_a_newer_discovery(discovering: Taxonomy) {
        let result = complete(Some(discovering), "older-job", Ok((vec![task("a")], Vec::new())), now());

        assert_eq!(result, Err(TransitionError::Stale));
    }

    #[rstest]
    fn an_abandoned_job_is_reclaimed_until_its_attempts_run_out(discovering: Taxonomy) {
        let first = claim(&discovering, now()).expect("first claim");
        let held = claim(&first, now());
        let expired = now() + LEASE + Duration::seconds(1);
        let second = claim(&first, expired).expect("reclaim");
        let third = claim(&second, expired + LEASE + Duration::seconds(1)).expect("third");
        let exhausted = claim(&third, expired + LEASE * 3).expect("exhausted");

        assert_eq!(held, None);
        assert_eq!(second.job.as_ref().map(|job| job.attempts), Some(2));
        assert_eq!(exhausted.status, TaxonomyStatus::Failed);
    }
}
