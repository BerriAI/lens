use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{Path, Query, Request, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::Utc;
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    CONTRACT_HEADER, CONTRACT_VERSION,
    auth::Role,
    eval::{CaseResult, CreateEvalRun, EvalRun, ResolvedDataset, RunStatus, Scorer},
};
use litellm_storage_clickhouse::{
    evals::{EvalStore, RunFilter},
    state::ClickHouseState,
};
use litellm_traces_clickhouse::evals::EvalTraces;
use serde::Deserialize;

use crate::{
    ApiError, EvalApiError,
    eval_datasets::{Datasets, EvalCases},
};

struct EvalState<R> {
    authentication: Arc<Authentication<R>>,
    store: EvalStore,
    datasets: Datasets,
    public_url: String,
}

#[derive(Clone)]
struct Team(String);

pub fn router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
) -> Router {
    configured_router(authentication, state, public_url, None, true)
}

pub fn router_with_traces<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: EvalTraces,
) -> Router {
    configured_router(authentication, state, public_url, Some(traces), true)
}

pub fn router_without_cases<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: Option<EvalTraces>,
) -> Router {
    configured_router(authentication, state, public_url, traces, false)
}

fn configured_router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: Option<EvalTraces>,
    include_cases: bool,
) -> Router {
    let state = Arc::new(EvalState {
        authentication,
        store: EvalStore::new(state.clone()),
        datasets: Datasets::new(state, traces),
        public_url,
    });
    let router = Router::new()
        .route("/lens/evals/runs", post(create::<R>).get(list::<R>))
        .route("/lens/evals/runs/{run}", get(read::<R>))
        .route(
            "/lens/evals/runs/{run}/results/{case_id}/{trial}",
            put(result::<R>),
        )
        .route("/lens/evals/runs/{run}/finish", post(finish::<R>))
        .route("/lens/datasets/resolve", get(resolve::<R>));
    let router = if include_cases {
        router.route(
            "/lens/datasets/{dataset_id}/revisions/{revision}/cases",
            get(cases::<R>),
        )
    } else {
        router
    };
    router
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            authorize::<R>,
        ))
        .with_state(state)
}

async fn authorize<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, EvalApiError> {
    validate_contract(request.headers())?;
    let identity =
        crate::auth::identity(&state.authentication, request.headers(), request.method()).await?;
    let scope = identity
        .team_id
        .filter(|team| !team.is_empty())
        .ok_or(EvalApiError::Unauthorized)?;
    if !matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS")
        && matches!(
            identity.user_role,
            Role::ProxyAdminViewer | Role::InternalUserViewer
        )
    {
        return Err(EvalApiError::Authentication(
            lens_auth::Error::Unauthorized("This credential is read-only"),
        ));
    }
    request.extensions_mut().insert(Team(scope));
    Ok(next.run(request).await)
}

pub(crate) fn validate_contract(headers: &HeaderMap) -> Result<(), EvalApiError> {
    let version = headers
        .get(CONTRACT_HEADER)
        .and_then(|value| value.to_str().ok());
    if version != Some(CONTRACT_VERSION.to_string().as_str()) {
        return Err(EvalApiError::ContractVersion);
    }
    Ok(())
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, EvalApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::InvalidRequest("body does not match the eval contract").into())
}

fn validate_create(request: &CreateEvalRun) -> Result<(), EvalApiError> {
    let valid_name = request.eval.bytes().enumerate().all(|(index, character)| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || (index > 0 && matches!(character, b'_' | b'-'))
    });
    if request.eval.is_empty()
        || !valid_name
        || request.agent.trim().is_empty()
        || request.version.trim().is_empty()
        || request.branch.trim().is_empty()
        || request.dataset_id.is_empty()
        || request.revision == 0
        || !(1..=10).contains(&request.trials)
        || request.scorers.is_empty()
        || request.timeout_per_trial_ms == 0
    {
        return Err(
            ApiError::InvalidRequest("run fields are outside the eval contract bounds").into(),
        );
    }
    if request
        .gate
        .pass_rate
        .is_some_and(|value| !(0.0..=1.0).contains(&value))
        || request
            .gate
            .cost_per_case
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        || request.gate.min.values().any(|value| !value.is_finite())
        || request.scorers.iter().any(|scorer| match scorer {
            Scorer::TaskCompleted(_) => false,
            Scorer::CalledBefore(config) => {
                config.first.trim().is_empty() || config.then.trim().is_empty()
            }
            Scorer::Judge(config) => config.prompt.trim().is_empty(),
        })
    {
        return Err(ApiError::InvalidRequest(
            "scorer and gate fields are outside the eval contract bounds",
        )
        .into());
    }
    Ok(())
}

async fn create<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, EvalApiError> {
    let request: CreateEvalRun = parse_body(&body)?;
    validate_create(&request)?;
    let key = headers
        .get("idempotency-key")
        .map(|value| value.to_str())
        .transpose()
        .map_err(|_| ApiError::InvalidRequest("Idempotency-Key must be text"))?;
    let cases = state.datasets.selected_cases(&team.0, &request).await?;
    let stored = state
        .store
        .create(&team.0, request, cases, key, &state.public_url, Utc::now())
        .await?;
    Ok((StatusCode::CREATED, Json(stored.run)).into_response())
}

async fn result<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Path((run, case_id, trial)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<StatusCode, EvalApiError> {
    let trial = trial
        .parse::<u32>()
        .map_err(|_| ApiError::InvalidRequest("trial must be a nonnegative integer"))?;
    let result: CaseResult = parse_body(&body)?;
    result.validate().map_err(|_| {
        ApiError::InvalidRequest("result must contain exactly one valid trace or error")
    })?;
    state
        .store
        .put_result(&team.0, &run, &case_id, trial, result, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn finish<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Path(run): Path<String>,
) -> Result<Response, EvalApiError> {
    let stored = state.store.finish(&team.0, &run, Utc::now()).await?;
    Ok((StatusCode::ACCEPTED, Json(stored.run)).into_response())
}

#[derive(Default, Deserialize)]
struct WaitQuery {
    #[serde(default)]
    wait: u64,
}

async fn read<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Path(run): Path<String>,
    query: Result<Query<WaitQuery>, QueryRejection>,
) -> Result<Json<EvalRun>, EvalApiError> {
    let Query(query) =
        query.map_err(|_| ApiError::InvalidRequest("wait must be a nonnegative integer"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(query.wait.min(30));
    let mut latest = state.store.get(&team.0, &run).await?.run;
    loop {
        if matches!(latest.status, RunStatus::Done | RunStatus::Failed)
            || tokio::time::Instant::now() >= deadline
        {
            return Ok(Json(latest));
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(100)).min(deadline),
        )
        .await;
        latest = match tokio::time::timeout_at(deadline, state.store.get(&team.0, &run)).await {
            Ok(stored) => stored?.run,
            Err(_) => return Ok(Json(latest)),
        };
    }
}

#[derive(Default, Deserialize)]
struct ListQuery {
    eval: Option<String>,
    agent: Option<String>,
    branch: Option<String>,
    limit: Option<u32>,
}

async fn list<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<Vec<EvalRun>>, EvalApiError> {
    let Query(query) = query.map_err(|_| ApiError::InvalidRequest("invalid run filters"))?;
    let filter = RunFilter {
        eval: query.eval,
        agent: query.agent,
        branch: query.branch,
        limit: query.limit.unwrap_or(50),
    };
    Ok(Json(
        state
            .store
            .list(&team.0, &filter)
            .await?
            .into_iter()
            .map(|stored| stored.run)
            .collect(),
    ))
}

#[derive(Deserialize)]
struct ResolveQuery {
    name: String,
    revision: Option<u64>,
}

async fn resolve<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    query: Result<Query<ResolveQuery>, QueryRejection>,
) -> Result<Json<ResolvedDataset>, EvalApiError> {
    let Query(query) = query.map_err(|_| {
        ApiError::InvalidRequest("name and an optional positive revision are required")
    })?;
    Ok(Json(
        state
            .datasets
            .resolve(&team.0, &query.name, query.revision)
            .await?,
    ))
}

async fn cases<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Path((id, revision)): Path<(String, String)>,
) -> Result<Json<EvalCases>, EvalApiError> {
    let revision = revision
        .parse::<u64>()
        .map_err(|_| ApiError::InvalidRequest("revision must be a positive integer"))?;
    Ok(Json(state.datasets.cases(&team.0, &id, revision).await?))
}
