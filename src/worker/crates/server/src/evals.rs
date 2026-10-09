mod details;
mod validation;

use crate::{auth, error::EvalError, routing::PublicRoutes};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    routing::{get, post, put},
};
use chrono::Utc;
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::Role,
    eval::{CaseResult, EvalRun, RunStatus},
};
use lens_datasets::DatasetRepository;
use lens_evals::{RunRepository, StoredRun};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc};

pub struct EvalConfig {
    pub public_url: url::Url,
}

struct App<R, D, E> {
    authentication: Arc<Authentication<R>>,
    datasets: D,
    runs: E,
    config: EvalConfig,
}

pub fn router<R, D, E>(
    authentication: Arc<Authentication<R>>,
    datasets: D,
    runs: E,
    config: EvalConfig,
) -> Router
where
    R: SessionRepository + 'static,
    D: DatasetRepository + 'static,
    E: RunRepository + 'static,
{
    Router::new()
        .public_route(
            "/lens/evals/runs",
            get(list::<R, D, E>).post(create::<R, D, E>),
        )
        .public_route("/lens/evals/runs/details", get(detail_list::<R, D, E>))
        .public_route("/lens/evals/runs/{run}", get(lookup::<R, D, E>))
        .public_route("/lens/evals/runs/{run}/details", get(detail::<R, D, E>))
        .public_route(
            "/lens/evals/runs/{run}/results/{case}/{trial}",
            put(submit::<R, D, E>),
        )
        .public_route("/lens/evals/runs/{run}/finish", post(finish::<R, D, E>))
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            datasets,
            runs,
            config,
        }))
}

async fn authorize<R: SessionRepository>(
    authentication: &Authentication<R>,
    headers: &HeaderMap,
    method: &Method,
    write: bool,
) -> Result<(), EvalError> {
    let identity = auth::identity(authentication, headers, method).await?;
    if identity.user_role != Role::ProxyAdmin
        && (write || identity.user_role != Role::ProxyAdminViewer)
    {
        return Err(EvalError::Forbidden);
    }
    if headers
        .get("x-lens-contract")
        .and_then(|value| value.to_str().ok())
        != Some("1")
    {
        return Err(EvalError::Contract);
    }
    Ok(())
}

async fn create<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<EvalRun>), EvalError> {
    authorize(&app.authentication, &headers, &method, true).await?;
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or(EvalError::Idempotency)?;
    let spec = validation::create(&body)?;
    let revision = i64::try_from(spec.revision).map_err(|_| EvalError::RevisionNotFound)?;
    let current = app
        .datasets
        .get(&spec.dataset_id, None)
        .await?
        .ok_or(EvalError::DatasetNotFound)?;
    let dataset = if current.revision == revision {
        current
    } else {
        app.datasets
            .get(&spec.dataset_id, Some(revision))
            .await?
            .ok_or(EvalError::RevisionNotFound)?
    };
    let cases: Vec<_> = dataset
        .cases
        .into_iter()
        .filter(|case| {
            case.included
                && spec
                    .case_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&case.id))
        })
        .collect();
    let ids: BTreeSet<_> = cases.iter().map(|case| &case.id).collect();
    if ids.is_empty()
        || spec
            .case_ids
            .as_ref()
            .is_some_and(|requested| requested.iter().collect::<BTreeSet<_>>() != ids)
    {
        return Err(EvalError::Run(lens_evals::RunError::UnknownCase));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut url = app.config.public_url.clone();
    url.set_path("/ui/");
    url.query_pairs_mut()
        .append_pair("tab", "datasets")
        .append_pair("dataset", &spec.dataset_id)
        .append_pair("dataset_tab", "runs")
        .append_pair("eval_run", &id);
    let run = EvalRun {
        id,
        status: RunStatus::Running,
        eval: spec.eval.clone(),
        agent: spec.agent.clone(),
        version: spec.version.clone(),
        branch: spec.branch.clone(),
        pr: spec.pr,
        url: url.into(),
        expected_trials: cases.len() as u64 * u64::from(spec.trials),
        received_trials: 0,
        summary: None,
        failure: String::new(),
    };
    let candidate = StoredRun {
        run,
        spec,
        cases,
        team_id: dataset.team_id,
        version: 0,
        created_at: Utc::now(),
        completed_at: None,
        submissions: Vec::new(),
        verdicts: Default::default(),
        lease: None,
        resolved_traces: Default::default(),
    };
    let created = app.runs.create(key, &candidate).await?;
    Ok((StatusCode::CREATED, Json(created.run)))
}

#[derive(Default, Deserialize)]
struct Filter {
    eval: Option<String>,
    agent: Option<String>,
    branch: Option<String>,
}

impl Filter {
    fn accepts(&self, run: &EvalRun) -> bool {
        self.eval.as_ref().is_none_or(|value| *value == run.eval)
            && self.agent.as_ref().is_none_or(|value| *value == run.agent)
            && self
                .branch
                .as_ref()
                .is_none_or(|value| *value == run.branch)
    }
}

async fn detail_list<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Query(filter): Query<Filter>,
) -> Result<Json<Vec<lens_contract::eval::EvalRunDetails>>, EvalError> {
    authorize(&app.authentication, &headers, &method, false).await?;
    let mut runs = app.runs.list().await?;
    runs.sort_by_key(|run| std::cmp::Reverse((run.created_at, run.run.id.clone())));
    Ok(Json(
        runs.iter()
            .filter(|run| filter.accepts(&run.run))
            .map(|run| details::details(run.clone(), &runs))
            .collect(),
    ))
}

async fn detail<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<Json<lens_contract::eval::EvalRunDetails>, EvalError> {
    authorize(&app.authentication, &headers, &method, false).await?;
    let record = app
        .runs
        .get(&id)
        .await?
        .ok_or(lens_evals::RunError::NotFound)?;
    let runs = app.runs.list().await?;
    Ok(Json(details::details(record, &runs)))
}

async fn list<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Query(filter): Query<Filter>,
) -> Result<Json<Vec<EvalRun>>, EvalError> {
    authorize(&app.authentication, &headers, &method, false).await?;
    let mut runs = app.runs.list().await?;
    runs.sort_by_key(|run| std::cmp::Reverse((run.created_at, run.run.id.clone())));
    Ok(Json(
        runs.into_iter()
            .map(|record| record.run)
            .filter(|run| filter.accepts(run))
            .collect(),
    ))
}

async fn lookup<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<Json<EvalRun>, EvalError> {
    authorize(&app.authentication, &headers, &method, false).await?;
    Ok(Json(
        app.runs
            .get(&id)
            .await?
            .ok_or(lens_evals::RunError::NotFound)?
            .run,
    ))
}

async fn submit<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Path((id, case, trial)): Path<(String, String, String)>,
    body: axum::body::Bytes,
) -> Result<StatusCode, EvalError> {
    authorize(&app.authentication, &headers, &method, true).await?;
    let result: CaseResult = serde_json::from_slice(&body).map_err(EvalError::InvalidResult)?;
    result
        .validate()
        .map_err(|_| lens_evals::RunError::InvalidResult)?;
    let trial = trial
        .parse()
        .map_err(|_| lens_evals::RunError::InvalidTrial)?;
    app.runs
        .submit(&id, &case, trial, &result, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn finish<R: SessionRepository, D: DatasetRepository, E: RunRepository>(
    State(app): State<Arc<App<R, D, E>>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<EvalRun>), EvalError> {
    authorize(&app.authentication, &headers, &method, true).await?;
    Ok((StatusCode::ACCEPTED, Json(app.runs.finish(&id).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::unfiltered(None, None, None, true)]
    #[case::eval(Some("quality"), None, None, true)]
    #[case::different_eval(Some("other"), None, None, false)]
    #[case::agent(None, Some("support"), None, true)]
    #[case::different_agent(None, Some("other"), None, false)]
    #[case::branch(None, None, Some("main"), true)]
    #[case::different_branch(None, None, Some("feature"), false)]
    #[case::all(Some("quality"), Some("support"), Some("main"), true)]
    #[case::one_mismatch(Some("quality"), Some("other"), Some("main"), false)]
    fn run_filters_combine_every_supplied_field(
        #[case] eval: Option<&str>,
        #[case] agent: Option<&str>,
        #[case] branch: Option<&str>,
        #[case] expected: bool,
    ) {
        let filter = Filter {
            eval: eval.map(Into::into),
            agent: agent.map(Into::into),
            branch: branch.map(Into::into),
        };
        let run = EvalRun {
            id: "run".into(),
            status: RunStatus::Running,
            eval: "quality".into(),
            agent: "support".into(),
            version: "version".into(),
            branch: "main".into(),
            pr: None,
            url: String::new(),
            expected_trials: 1,
            received_trials: 0,
            summary: None,
            failure: String::new(),
        };
        assert_eq!(filter.accepts(&run), expected);
    }
}
