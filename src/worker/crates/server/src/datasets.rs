mod json_diagnostics;
mod validation;

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, RawQuery, Request, State},
    http::{HeaderMap, Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::{Identity, Role},
    datasets::{BuildRequest, BuildResult, Dataset, DatasetCreate, DatasetSummary, RevisionSave},
};
use lens_datasets::{
    DatasetReader, DatasetRepository, Limits, Scope, build_cases, can_access, export_jsonl,
    revision_cases, revision_problem,
};

use crate::{auth, error::DatasetError};

#[derive(Clone, Copy, Debug)]
pub struct DatasetConfig {
    pub limits: Limits,
    pub trace_retry_after_seconds: i64,
}

impl Default for DatasetConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            trace_retry_after_seconds: 2,
        }
    }
}

struct App<R, D, B> {
    authentication: Arc<Authentication<R>>,
    datasets: D,
    reader: B,
    config: DatasetConfig,
}

pub fn router<R, D, B>(
    authentication: Arc<Authentication<R>>,
    datasets: D,
    reader: B,
    config: DatasetConfig,
) -> Router
where
    R: SessionRepository + 'static,
    D: DatasetRepository + 'static,
    B: DatasetReader + 'static,
{
    Router::new()
        .route(
            "/lens/datasets",
            get(list::<R, D, B>).post(create::<R, D, B>),
        )
        .route("/lens/datasets/build", post(build::<R, D, B>))
        .route("/lens/datasets/{dataset_id}", get(read::<R, D, B>))
        .route(
            "/lens/datasets/{dataset_id}/revisions",
            post(save::<R, D, B>),
        )
        .route("/lens/datasets/{dataset_id}/export", get(export::<R, D, B>))
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn(redirect_trailing_slash))
        .with_state(Arc::new(App {
            authentication,
            datasets,
            reader,
            config,
        }))
}

async fn redirect_trailing_slash(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let normalized = path.trim_end_matches('/');
    let parts: Vec<_> = normalized.split('/').collect();
    let known = matches!(
        parts.as_slice(),
        ["", "lens", "datasets"]
            | ["", "lens", "datasets", _]
            | ["", "lens", "datasets", _, "revisions" | "export"]
            | ["", "lens", "datasets", _, "revisions", _, "cases"]
    );
    if path != normalized && known {
        let query = request
            .uri()
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|host| host.to_str().ok());
        let location = if let Some(host) = host {
            format!(
                "{}://{host}{normalized}{query}",
                request.uri().scheme_str().unwrap_or("http")
            )
        } else {
            format!("{normalized}{query}")
        };
        return (
            axum::http::StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, location)],
        )
            .into_response();
    }
    next.run(request).await
}

fn user_scope(identity: &Identity, write: bool) -> Result<Scope, DatasetError> {
    if write && identity.user_role != Role::ProxyAdmin {
        return Err(DatasetError::ForbiddenWrite);
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
    Err(DatasetError::ForbiddenRead)
}

async fn get_dataset<D: DatasetRepository>(
    datasets: &D,
    id: &str,
    scope: &Scope,
    revision: Option<i64>,
) -> Result<Dataset, DatasetError> {
    datasets
        .get(id, revision)
        .await?
        .filter(|dataset| {
            can_access(
                scope,
                &Scope {
                    team_id: dataset.team_id.clone(),
                    ..Scope::default()
                },
            )
        })
        .ok_or(DatasetError::NotFound)
}

async fn list<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<DatasetSummary>>, DatasetError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = user_scope(&identity, false)?;
    let summaries = app
        .datasets
        .summaries()
        .await?
        .into_iter()
        .filter(|entry| {
            can_access(
                &scope,
                &Scope {
                    team_id: entry.team_id.clone(),
                    ..Scope::default()
                },
            )
        })
        .map(|entry| entry.summary)
        .collect();
    Ok(Json(summaries))
}

async fn create<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Dataset>, DatasetError> {
    let parsed = validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: DatasetCreate = validation::request(parsed, validation::Model::Create)?;
    user_scope(&identity, true)?;
    let now = Utc::now();
    let dataset = Dataset {
        id: uuid::Uuid::new_v4().to_string(),
        name: request.name,
        agent_name: request.agent_name,
        team_id: identity.team_id.unwrap_or_default(),
        created_at: now,
        revision: 0,
        created_by: identity.user_id.unwrap_or_default(),
        cases: Vec::new(),
    };
    if !app.datasets.insert(&dataset, now).await? {
        return Err(DatasetError::AlreadyExists);
    }
    Ok(Json(dataset))
}

async fn build<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<BuildResult>, DatasetError> {
    let parsed = validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: BuildRequest = validation::request(parsed, validation::Model::Build)?;
    let scope = user_scope(&identity, false)?;
    let existing = if request.dataset_id.is_empty() {
        Vec::new()
    } else {
        get_dataset(&app.datasets, &request.dataset_id, &scope, None)
            .await?
            .cases
    };
    let result = build_cases(&request, &app.reader, &existing, &scope, app.config.limits)
        .await
        .map_err(|source| DatasetError::Read {
            source,
            retry_after_seconds: app.config.trace_retry_after_seconds,
        })?;
    Ok(Json(result))
}

async fn read<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Dataset>, DatasetError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let revision = validation::revision_query(query.as_deref())?;
    let scope = user_scope(&identity, false)?;
    let revision = revision
        .map(|revision| revision.exact().ok_or(DatasetError::NotFound))
        .transpose()?;
    Ok(Json(
        get_dataset(&app.datasets, &id, &scope, revision).await?,
    ))
}

async fn save<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Dataset>, DatasetError> {
    let parsed = validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let (request, base_revision): (RevisionSave, _) = validation::revision_request(parsed)?;
    let latest = get_dataset(&app.datasets, &id, &user_scope(&identity, true)?, None).await?;
    if base_revision.exact() != Some(latest.revision) {
        return Err(DatasetError::Changed);
    }
    let cases = revision_cases(&request.cases);
    if let Some(problem) = revision_problem(&cases, app.config.limits) {
        return Err(problem.into());
    }
    let saved = Dataset {
        revision: latest
            .revision
            .checked_add(1)
            .ok_or(DatasetError::Changed)?,
        cases,
        created_by: identity.user_id.unwrap_or_default(),
        ..latest
    };
    if !app.datasets.insert(&saved, Utc::now()).await? {
        return Err(DatasetError::Changed);
    }
    Ok(Json(saved))
}

async fn export<R: SessionRepository, D: DatasetRepository, B: DatasetReader>(
    State(app): State<Arc<App<R, D, B>>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Response, DatasetError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let revision = validation::revision_query(query.as_deref())?;
    let scope = user_scope(&identity, false)?;
    let revision = revision
        .map(|revision| revision.exact().ok_or(DatasetError::NotFound))
        .transpose()?;
    let dataset = get_dataset(&app.datasets, &id, &scope, revision).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"dataset-{}-r{}.jsonl\"",
                    dataset.id, dataset.revision
                ),
            ),
        ],
        export_jsonl(&dataset.cases),
    )
        .into_response())
}
