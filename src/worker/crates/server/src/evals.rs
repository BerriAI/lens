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
    AGENT_IO_CONTRACT_VERSION, CONTRACT_HEADER, CONTRACT_VERSION,
    auth::Role,
    eval::{
        CaseResult, CreateEvalRun, EvalDefinition, EvalRun, EvalSpec, ResolvedDataset, RunCase,
        RunCaseSummary, RunStatus, Scorer, ScorerCheck, ToolStep, TrialSteps,
    },
};
use litellm_storage_clickhouse::{
    evals::{EvalStore, RunFilter, StoredCase},
    state::ClickHouseState,
};
use litellm_traces_clickhouse::evals::EvalTraces;
use serde::Deserialize;
use tower::ServiceExt;

use crate::{
    ApiError, EvalApiError,
    eval_datasets::{Datasets, EvalCases},
    routing::PublicRoutes,
};

struct EvalState<R> {
    authentication: Arc<Authentication<R>>,
    store: EvalStore,
    datasets: Datasets,
    traces: Option<EvalTraces>,
    public_url: String,
}

#[derive(Clone)]
struct Team(String);

#[derive(Clone, Copy)]
enum EvalContract {
    Trace,
    AgentIo,
}

impl EvalContract {
    fn require_agent_io(self, supplied: bool) -> Result<(), EvalApiError> {
        if supplied && matches!(self, Self::Trace) {
            return Err(ApiError::InvalidRequest(
                "Agent I/O and output require X-Lens-Contract: 2",
            )
            .into());
        }
        Ok(())
    }

    fn definition(self, definition: EvalDefinition) -> EvalDefinition {
        match self {
            Self::Trace => EvalDefinition {
                spec: EvalSpec {
                    agent_io: None,
                    ..definition.spec
                },
                ..definition
            },
            Self::AgentIo => definition,
        }
    }
}

pub fn router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
) -> Router {
    let (api, cases) = routers(authentication, state, public_url, None);
    api.merge(cases)
}

pub fn router_with_traces<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: EvalTraces,
) -> Router {
    let (api, cases) = split_router_with_traces(authentication, state, public_url, traces);
    api.merge(cases)
}

/// The eval API and its dataset cases route as separate routers, for hosts that also serve the admin dataset API on the same path
pub fn split_router_with_traces<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: EvalTraces,
) -> (Router, Router) {
    routers(authentication, state, public_url, Some(traces))
}

/// Sends dataset case reads that carry the contract header to `cases`, leaving every other request on `router`
pub fn with_contract_cases(router: Router, cases: Router) -> Router {
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let cases = cases.clone();
        async move {
            if request.headers().contains_key(CONTRACT_HEADER)
                && is_dataset_cases_path(request.uri().path())
            {
                match cases.oneshot(without_extensions(request)).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                }
            } else {
                next.run(request).await
            }
        }
    }))
}

/// The outer router has already matched this path, so its path params must not leak into the inner match
fn without_extensions(request: Request) -> Request {
    let (parts, body) = request.into_parts();
    let mut clean = Request::new(body);
    *clean.method_mut() = parts.method;
    *clean.uri_mut() = parts.uri;
    *clean.version_mut() = parts.version;
    *clean.headers_mut() = parts.headers;
    clean
}

fn is_dataset_cases_path(path: &str) -> bool {
    matches!(
        path.split('/').collect::<Vec<_>>().as_slice(),
        ["", "lens", "datasets", id, "revisions", revision, "cases"]
            if !id.is_empty() && !revision.is_empty()
    )
}

fn routers<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    state: ClickHouseState,
    public_url: String,
    traces: Option<EvalTraces>,
) -> (Router, Router) {
    let state = Arc::new(EvalState {
        authentication,
        store: EvalStore::new(state.clone()),
        datasets: Datasets::new(state, traces.clone()),
        traces,
        public_url,
    });
    let authorized = |router: Router<Arc<EvalState<R>>>| {
        router
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                authorize::<R>,
            ))
            .with_state(state.clone())
    };
    let api = Router::new()
        .public_route("/lens/evals", get(definitions::<R>))
        .public_route(
            "/lens/evals/{name}",
            get(definition::<R>).put(put_definition::<R>),
        )
        .public_route("/lens/evals/runs", post(create::<R>).get(list::<R>))
        .public_route("/lens/evals/runs/{run}", get(read::<R>))
        .public_route(
            "/lens/evals/runs/{run}/results/{case_id}/{trial}",
            put(result::<R>),
        )
        .public_route("/lens/evals/runs/{run}/finish", post(finish::<R>))
        .public_route("/lens/evals/runs/{run}/cases", get(run_cases::<R>))
        .public_route("/lens/evals/runs/{run}/cases/{case_id}", get(run_case::<R>))
        .public_route("/lens/datasets/resolve", get(resolve::<R>));
    let cases = Router::new().public_route(
        "/lens/datasets/{id}/revisions/{revision}/cases",
        get(cases::<R>),
    );
    (authorized(api), authorized(cases))
}

async fn authorize<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    mut request: Request,
    next: Next,
) -> Result<Response, EvalApiError> {
    let version = request
        .headers()
        .get(CONTRACT_HEADER)
        .and_then(|value| value.to_str().ok());
    if request.headers().get_all(CONTRACT_HEADER).iter().count() != 1 {
        return Err(EvalApiError::ContractVersion);
    }
    let contract = if version == Some(CONTRACT_VERSION.to_string().as_str()) {
        EvalContract::Trace
    } else if version == Some(AGENT_IO_CONTRACT_VERSION.to_string().as_str()) {
        EvalContract::AgentIo
    } else {
        return Err(EvalApiError::ContractVersion);
    };
    let session = crate::auth::session_cookie(request.headers());
    let identity = state
        .authentication
        .authenticate(
            crate::auth::credentials(request.headers(), request.method(), session.as_deref())?,
            Utc::now(),
        )
        .await?;
    let scope = match identity.team_id.filter(|team| !team.is_empty()) {
        Some(team) => team,
        None if matches!(
            identity.user_role,
            Role::ProxyAdmin | Role::ProxyAdminViewer
        ) =>
        {
            String::new()
        }
        None => return Err(EvalApiError::Unauthorized),
    };
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
    request.extensions_mut().insert(contract);
    Ok(next.run(request).await)
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, EvalApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::InvalidRequest("body does not match the eval contract").into())
}

pub(crate) fn valid_eval_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(index, character)| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || (index > 0 && matches!(character, b'_' | b'-'))
        })
}

fn validate_create(request: &CreateEvalRun) -> Result<(), EvalApiError> {
    if let Some(agent_io) = &request.agent_io {
        agent_io
            .validate()
            .map_err(|_| ApiError::InvalidRequest("invalid agent I/O contract"))?;
        if agent_io.trace.is_none()
            && request
                .scorers
                .iter()
                .any(|scorer| !matches!(scorer, Scorer::Judge(_)))
        {
            return Err(ApiError::InvalidRequest(
                "Trace scorers require a trace mapping in the agent I/O contract",
            )
            .into());
        }
        if agent_io.trace.is_none() && request.gate.cost_per_case.is_some() {
            return Err(ApiError::InvalidRequest(
                "Cost gates require a trace mapping so Lens can resolve incurred spend",
            )
            .into());
        }
    }
    if !valid_eval_name(&request.eval)
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
        || request.gate.min.iter().any(|(name, value)| {
            !lens_contract::eval::scorer_names(&request.scorers).contains(name)
                || !(0.0..=1.0).contains(value)
        })
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
    let timeout = i64::try_from(request.timeout_per_trial_ms)
        .ok()
        .and_then(chrono::TimeDelta::try_milliseconds)
        .and_then(|duration| Utc::now().checked_add_signed(duration));
    if timeout.is_none() {
        return Err(
            ApiError::InvalidRequest("trial timeout exceeds the supported calendar range").into(),
        );
    }
    Ok(())
}

async fn create<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, EvalApiError> {
    let request: CreateEvalRun = parse_body(&body)?;
    contract.require_agent_io(request.agent_io.is_some())?;
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
    Ok((StatusCode::CREATED, Json(without_ci(stored.run))).into_response())
}

fn without_ci(run: EvalRun) -> EvalRun {
    EvalRun {
        ci_url: String::new(),
        ..run
    }
}

async fn definitions<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
) -> Result<Json<Vec<EvalDefinition>>, EvalApiError> {
    Ok(Json(
        state
            .store
            .definitions(&team.0)
            .await?
            .into_iter()
            .map(|definition| contract.definition(definition))
            .collect(),
    ))
}

async fn definition<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
    Path(name): Path<String>,
) -> Result<Json<EvalDefinition>, EvalApiError> {
    Ok(Json(
        contract.definition(state.store.definition(&team.0, &name).await?),
    ))
}

async fn put_definition<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<EvalDefinition>, EvalApiError> {
    if !valid_eval_name(&name) || name == "runs" {
        return Err(ApiError::InvalidRequest("eval name must match ^[a-z0-9][a-z0-9_-]*$").into());
    }
    let spec: EvalSpec = parse_body(&body)?;
    contract.require_agent_io(spec.agent_io.is_some())?;
    validate_create(&CreateEvalRun {
        eval: name.clone(),
        agent: spec.agent.clone(),
        dataset_id: spec.dataset_id.clone(),
        revision: spec.revision.unwrap_or(1),
        case_ids: None,
        version: "definition".into(),
        branch: spec.baseline.clone(),
        pr: None,
        ci_url: String::new(),
        trials: spec.trials,
        scorers: spec.scorers.clone(),
        gate: spec.gate.clone(),
        timeout_per_trial_ms: spec.timeout_per_trial_ms,
        agent_io: spec.agent_io.clone(),
    })?;
    let definition = match contract {
        EvalContract::Trace => {
            state
                .store
                .put_legacy_definition(&team.0, &name, spec, Utc::now())
                .await?
        }
        EvalContract::AgentIo => {
            state
                .store
                .put_definition(&team.0, &name, spec, Utc::now())
                .await?
        }
    };
    Ok(Json(definition))
}

async fn result<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
    Path((run, case_id, trial)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<StatusCode, EvalApiError> {
    let trial = trial
        .parse::<u32>()
        .map_err(|_| ApiError::InvalidRequest("trial must be a nonnegative integer"))?;
    let result: CaseResult = parse_body(&body)?;
    contract.require_agent_io(result.output.is_some())?;
    result.validate().map_err(|_| {
        ApiError::InvalidRequest("result must contain output and/or a trace, or an exclusive error")
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
    Ok((StatusCode::ACCEPTED, Json(without_ci(stored.run))).into_response())
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
            return Ok(Json(without_ci(latest)));
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(100)).min(deadline),
        )
        .await;
        latest = match tokio::time::timeout_at(deadline, state.store.get(&team.0, &run)).await {
            Ok(stored) => stored?.run,
            Err(_) => return Ok(Json(without_ci(latest))),
        };
    }
}

#[derive(Default, Deserialize)]
struct ListQuery {
    eval: Option<String>,
    agent: Option<String>,
    branch: Option<String>,
    dataset: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
    #[serde(default)]
    include_ci: bool,
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
        dataset: query.dataset,
        after: query.cursor,
        limit: query.limit.unwrap_or(50),
    };
    Ok(Json(
        state
            .store
            .list(&team.0, &filter)
            .await?
            .into_iter()
            .map(|stored| {
                if query.include_ci {
                    EvalRun {
                        ci_url: stored.request.ci_url,
                        ..stored.run
                    }
                } else {
                    without_ci(stored.run)
                }
            })
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

async fn run_cases<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Path(run): Path<String>,
) -> Result<Json<Vec<RunCaseSummary>>, EvalApiError> {
    let stored = state.store.get(&team.0, &run).await?;
    Ok(Json(
        stored
            .cases
            .iter()
            .map(|case| RunCaseSummary {
                case_id: case.id.clone(),
                title: case_title(case),
                critical: case.critical,
                passed: stored.verdicts.get(&case.id).copied(),
            })
            .collect(),
    ))
}

async fn run_case<R: SessionRepository>(
    State(state): State<Arc<EvalState<R>>>,
    Extension(team): Extension<Team>,
    Extension(contract): Extension<EvalContract>,
    Path((run, case_id)): Path<(String, String)>,
) -> Result<Json<RunCase>, EvalApiError> {
    let stored = state.store.get(&team.0, &run).await?;
    let case = stored
        .cases
        .iter()
        .find(|case| case.id == case_id)
        .ok_or(ApiError::NotFound)?;
    let now = Utc::now().timestamp_millis();
    let mut trials = Vec::new();
    for trial in stored
        .trials
        .iter()
        .filter(|trial| trial.case_id == case_id)
    {
        let (spans, traces) = match (&state.traces, &trial.result.trace) {
            (Some(traces), Some(reference)) => traces
                .read(&team.0, reference, now)
                .await
                .map_err(|error| ApiError::Internal(Box::new(error)))?
                .map(|trace| (trace.spans, trace.traces))
                .unwrap_or_default(),
            _ => (Vec::new(), Vec::new()),
        };
        let checks = if spans.is_empty() {
            Vec::new()
        } else {
            scorer_checks(&stored.request.scorers, &spans)
        };
        let steps = tool_steps(spans);
        trials.push(TrialSteps {
            trial: trial.trial,
            traces,
            output: match contract {
                EvalContract::Trace => None,
                EvalContract::AgentIo => trial.result.output.clone(),
            },
            error: trial
                .result
                .error
                .as_ref()
                .map(|error| error.message.clone()),
            checks,
            steps,
        });
    }
    trials.sort_by_key(|trial| trial.trial);
    Ok(Json(RunCase {
        case_id: case.id.clone(),
        title: case_title(case),
        critical: case.critical,
        passed: stored.verdicts.get(&case.id).copied(),
        trials,
    }))
}

fn case_title(case: &StoredCase) -> String {
    if !case.title.trim().is_empty() && case.title != case.id {
        return case.title.clone();
    }
    let input = case.input.split_whitespace().collect::<Vec<_>>().join(" ");
    if input.is_empty() {
        return case.id.clone();
    }
    let title: String = input.chars().take(120).collect();
    if input.chars().count() > 120 {
        format!("{title}…")
    } else {
        title
    }
}

fn scorer_checks(
    scorers: &[Scorer],
    spans: &[litellm_traces_clickhouse::evals::EvalSpan],
) -> Vec<ScorerCheck> {
    let spans: Vec<lens_evals::EvalSpan> = spans.iter().map(scoring_span).collect();
    lens_contract::eval::scorer_names(scorers)
        .into_iter()
        .zip(scorers)
        .filter_map(|(name, scorer)| {
            let passed = match scorer {
                Scorer::TaskCompleted(_) => lens_evals::scorer::task_completed(&spans),
                Scorer::CalledBefore(rule) => {
                    lens_evals::scorer::called_before(&spans, &rule.first, &rule.then)
                }
                Scorer::Judge(_) => return None,
            };
            Some(ScorerCheck {
                scorer: name,
                passed,
            })
        })
        .collect()
}

fn scoring_span(span: &litellm_traces_clickhouse::evals::EvalSpan) -> lens_evals::EvalSpan {
    lens_evals::EvalSpan {
        span_id: span.span_id.clone(),
        parent_span_id: span.parent_span_id.clone(),
        name: span.name.clone(),
        start_ns: span.start_ns,
        status: match span.status {
            litellm_traces::SpanStatus::Ok => lens_evals::SpanStatus::Ok,
            litellm_traces::SpanStatus::Error => lens_evals::SpanStatus::Error,
            litellm_traces::SpanStatus::Unset => lens_evals::SpanStatus::Unset,
        },
        tool_name: span.attributes.get("gen_ai.tool.name").cloned(),
        input: span.input.clone(),
        output: span.output.clone(),
    }
}

fn tool_steps(spans: Vec<litellm_traces_clickhouse::evals::EvalSpan>) -> Vec<ToolStep> {
    let mut steps: Vec<ToolStep> = spans
        .into_iter()
        .filter_map(|span| {
            let tool_name = span.attributes.get("gen_ai.tool.name")?.clone();
            Some(ToolStep {
                ok: !matches!(span.status, litellm_traces::SpanStatus::Error),
                name: span.name,
                tool_name,
                start_ns: span.start_ns,
                end_ns: span.end_ns,
            })
        })
        .collect();
    steps.sort_by_key(|step| step.start_ns);
    steps
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

#[cfg(test)]
mod tests {
    use super::case_title;
    use litellm_storage_clickhouse::evals::StoredCase;
    use rstest::rstest;

    #[rstest]
    #[case::named("Run tests", "Fix the bug", "Run tests")]
    #[case::hash("case-id", "Fix\n the  bug", "Fix the bug")]
    #[case::blank("", "Fix the bug", "Fix the bug")]
    #[case::no_input("case-id", "  ", "case-id")]
    fn should_use_the_input_when_a_case_has_no_readable_title(
        #[case] title: &str,
        #[case] input: &str,
        #[case] expected: &str,
    ) {
        let case = StoredCase {
            id: "case-id".into(),
            title: title.into(),
            critical: false,
            input: input.into(),
            followups: Vec::new(),
            expected: String::new(),
        };
        assert_eq!(case_title(&case), expected);
    }
}
