use crate::routing::PublicRoutes;
pub(crate) mod validation;

use std::{future::Future, sync::Arc, time::Duration};

pub use crate::error::{InvestigationAccessError, InvestigationError};
use crate::{auth, datasets::validation as body_validation};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderMap, Method},
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE};
use chrono::Utc;
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::{Identity, Role},
    investigations::{
        FindingImport, FindingImported, FindingSource, FindingUpdate, Lens, LensList, Public,
        ReviewPage, RunRequest, Scope, WatchAllResult, WatchSkipped, Worker,
    },
    worker::{Evidence, Job, LensSettings},
};
use lens_investigations::{LensRepository, RepositoryError, can_access};
use rand::Rng;

pub trait InvestigationAccess: Send + Sync {
    fn verify_finding(
        &self,
        _lens: &Lens,
        _sources: &[FindingSource],
    ) -> impl Future<Output = Result<Option<Vec<Evidence>>, InvestigationAccessError>> + Send {
        async { Ok(None) }
    }
    fn tracing_enabled(&self) -> bool;
    fn workers(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Vec<Worker>, RepositoryError>> + Send;
    fn validate_model(
        &self,
        settings: &LensSettings,
        identity: &Identity,
    ) -> impl Future<Output = Result<(), InvestigationAccessError>> + Send;
    fn validate_workers(
        &self,
        settings: &LensSettings,
        scope: &Scope,
    ) -> impl Future<Output = Result<(), InvestigationAccessError>> + Send;
}

struct App<R, D, A> {
    authentication: Arc<Authentication<R>>,
    repository: D,
    access: Arc<A>,
}

pub fn router<R, D, A>(
    authentication: Arc<Authentication<R>>,
    repository: D,
    access: Arc<A>,
) -> Router
where
    R: SessionRepository + 'static,
    D: LensRepository + 'static,
    A: InvestigationAccess + 'static,
{
    Router::new()
        .public_route("/lens", get(list::<R, D, A>).post(create::<R, D, A>))
        .public_route("/lens/watch-all", post(watch_all::<R, D, A>))
        .public_route(
            "/lens/{lens_id}",
            get(read::<R, D, A>).put(update::<R, D, A>),
        )
        .public_route(
            "/lens/{lens_id}/runs",
            get(runs::<R, D, A>).post(run::<R, D, A>),
        )
        .public_route("/lens/{lens_id}/runs/{job_id}", get(read_run::<R, D, A>))
        .public_route(
            "/lens/{lens_id}/runs/{job_id}/reviews",
            get(reviews::<R, D, A>),
        )
        .public_route("/lens/{lens_id}/cancel", post(cancel::<R, D, A>))
        .public_route(
            "/lens/{lens_id}/findings/import",
            post(import::<R, D, A>).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .public_route(
            "/lens/{lens_id}/findings/{finding_id}",
            patch(feedback::<R, D, A>),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            repository,
            access,
        }))
}

fn scope(identity: &Identity, write: bool) -> Result<Scope, InvestigationError> {
    if write && identity.user_role != Role::ProxyAdmin {
        return Err(InvestigationError::ForbiddenWrite);
    }
    if matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Ok(Scope {
            all_teams: true,
            ..Scope::default()
        });
    }
    Err(InvestigationError::ForbiddenRead)
}

fn validate_selection(settings: &LensSettings) -> Result<(), InvestigationError> {
    for id in &settings.execution_ids {
        let decoded = URL_SAFE
            .decode(id)
            .map_err(|_| InvestigationError::InvalidSelection)?;
        let parts: Vec<String> =
            serde_json::from_slice(&decoded).map_err(|_| InvestigationError::InvalidSelection)?;
        if !matches!(parts.len(), 3 | 4) || !matches!(parts[0].as_str(), "traces" | "requests") {
            return Err(InvestigationError::InvalidSelection);
        }
    }
    Ok(())
}

async fn get_lens<D: LensRepository>(
    repository: &D,
    id: &str,
    scope: &Scope,
) -> Result<Lens, InvestigationError> {
    repository
        .get(id)
        .await?
        .filter(|lens| can_access(scope, &lens.scope))
        .ok_or(InvestigationError::NotFound)
}

async fn mutate<D, F>(
    repository: &D,
    id: &str,
    scope: &Scope,
    transform: F,
) -> Result<Lens, InvestigationError>
where
    D: LensRepository,
    F: Fn(&Lens) -> Result<Lens, lens_investigations::Error> + Send,
{
    for _ in 0..40 {
        let current = get_lens(repository, id, scope).await?;
        let candidate = transform(&current)?;
        match repository.replace(&current, &candidate).await {
            Ok(updated) => return Ok(updated),
            Err(RepositoryError::Conflict) => {
                let seconds = rand::thread_rng().gen_range(0.0..0.02);
                tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(InvestigationError::Changed)
}

async fn list<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<LensList>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity, false)?;
    let lenses = app
        .repository
        .lenses(&scope)
        .await?
        .iter()
        .filter(|lens| can_access(&scope, &lens.scope))
        .map(lens_investigations::summarized)
        .collect();
    let workers = app
        .access
        .workers(&scope)
        .await?
        .into_iter()
        .filter(|worker| can_access(&scope, &worker.scope))
        .collect();
    Ok(Json(LensList {
        lenses,
        workers,
        tracing_enabled: app.access.tracing_enabled(),
    }))
}

async fn create<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Lens>, InvestigationError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let settings: LensSettings = body_validation::request(
        parsed,
        body_validation::Model::Investigation(validation::Model::Settings),
    )?;
    let scope = scope(&identity, true)?;
    validate_selection(&settings)?;
    app.access.validate_model(&settings, &identity).await?;
    app.access.validate_workers(&settings, &scope).await?;
    let lens = lens_investigations::create_lens(
        settings,
        scope,
        Utc::now(),
        &uuid::Uuid::new_v4().to_string(),
        &uuid::Uuid::new_v4().to_string(),
    )?;
    Ok(Json(app.repository.create(&lens).await?))
}

async fn read<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Lens>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let lens = get_lens(&app.repository, &id, &scope(&identity, false)?).await?;
    Ok(Json(lens_investigations::summarized(&lens)))
}

async fn update<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Lens>, InvestigationError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let settings: LensSettings = body_validation::request(
        parsed,
        body_validation::Model::Investigation(validation::Model::Settings),
    )?;
    let scope = scope(&identity, true)?;
    let lens = get_lens(&app.repository, &id, &scope).await?;
    validate_selection(&settings)?;
    if settings.model != lens.settings.model || (settings.enabled && !lens.settings.enabled) {
        app.access.validate_model(&settings, &identity).await?;
    }
    Ok(Json(
        mutate(&app.repository, &id, &scope, |lens| {
            lens_investigations::update_settings(lens, settings.clone(), Utc::now())
        })
        .await?,
    ))
}

async fn run<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Lens>, InvestigationError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: RunRequest = body_validation::request(
        parsed,
        body_validation::Model::Investigation(validation::Model::Run),
    )?;
    let scope = scope(&identity, true)?;
    let lens = get_lens(&app.repository, &id, &scope).await?;
    let settings = request.settings.as_ref().unwrap_or(&lens.settings);
    validate_selection(settings)?;
    app.access.validate_model(settings, &identity).await?;
    app.access.validate_workers(settings, &lens.scope).await?;
    let now = Utc::now();
    let job_id = uuid::Uuid::new_v4().to_string();
    Ok(Json(
        mutate(&app.repository, &id, &scope, |lens| {
            lens_investigations::manual_run(lens, &request, now, &job_id)
        })
        .await?,
    ))
}

async fn runs<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Public<Vec<Job>>>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let offset = validation::offset(query.as_deref(), "offset")?;
    get_lens(&app.repository, &id, &scope(&identity, false)?).await?;
    let jobs = app
        .repository
        .jobs(&id, offset)
        .await?
        .into_iter()
        .map(|job| Job {
            sample: None,
            findings: None,
            assessments: Vec::new(),
            ..job
        })
        .collect();
    Ok(Json(Public(jobs)))
}

async fn job<D: LensRepository>(
    repository: &D,
    lens_id: &str,
    job_id: &str,
    scope: &Scope,
) -> Result<Job, InvestigationError> {
    get_lens(repository, lens_id, scope).await?;
    repository
        .job(lens_id, job_id)
        .await?
        .ok_or(InvestigationError::RunNotFound)
}

async fn read_run<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path((id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Public<Job>>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    Ok(Json(Public(
        job(&app.repository, &id, &job_id, &scope(&identity, false)?).await?,
    )))
}

async fn reviews<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path((id, job_id)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ReviewPage>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let after = validation::offset(query.as_deref(), "after")?;
    let job = job(&app.repository, &id, &job_id, &scope(&identity, false)?).await?;
    Ok(Json(lens_investigations::reviews_after(
        &job,
        i64::try_from(after).unwrap_or(i64::MAX),
    )))
}

async fn cancel<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Lens>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity, true)?;
    let now = Utc::now();
    Ok(Json(
        mutate(&app.repository, &id, &scope, |lens| {
            lens_investigations::cancel_job(lens, now)
        })
        .await?,
    ))
}

async fn feedback<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path((id, finding_id)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Lens>, InvestigationError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: FindingUpdate = body_validation::request(
        parsed,
        body_validation::Model::Investigation(validation::Model::Finding),
    )?;
    let scope = scope(&identity, true)?;
    Ok(Json(
        mutate(&app.repository, &id, &scope, |lens| {
            Ok(lens_investigations::update_finding_status(
                lens,
                &finding_id,
                &request,
            ))
        })
        .await?,
    ))
}

async fn import<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    Json(request): Json<FindingImport>,
) -> Result<Json<FindingImported>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity, true)?;
    let lens = get_lens(&app.repository, &id, &scope).await?;
    lens_investigations::validate_import(&lens, &request)?;
    let evidence = app
        .access
        .verify_finding(&lens, &request.evidence)
        .await?
        .ok_or(lens_investigations::Error::InvalidImport)?;
    let now = Utc::now();
    let updated = mutate(&app.repository, &id, &scope, |current| {
        if current.revision != lens.revision || current.scope != lens.scope {
            return Err(lens_investigations::Error::ImportScope);
        }
        lens_investigations::import_finding(current, &request, evidence.clone(), now)
    })
    .await?;
    Ok(Json(FindingImported {
        lens_id: updated.id,
        finding_id: updated.findings[0].id.clone(),
    }))
}

async fn watch_all<R: SessionRepository, D: LensRepository, A: InvestigationAccess>(
    State(app): State<Arc<App<R, D, A>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<WatchAllResult>, InvestigationError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity, true)?;
    let paused = app.repository.lenses(&scope).await?;
    let mut ready = Vec::new();
    let mut skipped = Vec::new();
    for lens in paused
        .into_iter()
        .filter(|lens| can_access(&scope, &lens.scope) && !lens.settings.enabled)
    {
        let settings = LensSettings {
            enabled: true,
            ..lens.settings.clone()
        };
        let check = match validate_selection(&settings) {
            Ok(()) => app
                .access
                .validate_model(&settings, &identity)
                .await
                .map_err(InvestigationError::from),
            Err(error) => Err(error),
        };
        match check {
            Ok(()) => ready.push(lens.id),
            Err(error) => skipped.push(WatchSkipped {
                id: lens.id,
                name: lens.settings.name.to_string(),
                reason: error.to_string(),
            }),
        }
    }
    let mut watching = Vec::new();
    for id in ready {
        match mutate(&app.repository, &id, &scope, |lens| {
            if lens.settings.enabled {
                return Ok(lens.clone());
            }
            Ok(Lens {
                settings: LensSettings {
                    enabled: true,
                    ..lens.settings.clone()
                },
                revision: lens
                    .revision
                    .checked_add(1)
                    .ok_or(lens_investigations::Error::RevisionCount)?,
                ..lens.clone()
            })
        })
        .await
        {
            Ok(lens) => watching.push(lens.id),
            Err(InvestigationError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(Json(WatchAllResult { watching, skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::trace(json!(["traces","alpha","t","ref"]),true)]
    #[case::legacy_request(json!(["requests","alpha","t"]),true)]
    #[case::too_short(json!(["traces","alpha"]),false)]
    #[case::too_long(json!(["traces","alpha","t","ref","extra"]),false)]
    #[case::wrong_source(json!(["logs","alpha","t"]),false)]
    #[case::wrong_type(json!(["traces","alpha",1]),false)]
    fn execution_ids_require_the_source_reader_contract(
        #[case] parts: serde_json::Value,
        #[case] valid: bool,
    ) {
        let id = URL_SAFE.encode(serde_json::to_vec(&parts).unwrap());
        let settings: LensSettings = serde_json::from_value(
            json!({"name":"Research","model":"analysis","execution_ids":[id]}),
        )
        .unwrap();
        assert_eq!(validate_selection(&settings).is_ok(), valid);
    }
}
