use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path as RoutePath, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{Error, Result, model::*};

mod traces;
use traces::{Submission, Trace, Wait};

#[derive(Clone)]
pub struct Stub {
    store: Arc<Mutex<Store>>,
    key: String,
}

struct Store {
    dataset: EvalCases,
    runs: BTreeMap<String, EvalRun>,
    specs: BTreeMap<String, CreateEvalRun>,
    results: BTreeMap<String, BTreeMap<(String, usize), Submission>>,
    traces: BTreeMap<(String, String), Trace>,
    keys: BTreeMap<String, (String, String)>,
    verdicts: BTreeMap<String, BTreeMap<String, bool>>,
    completed: Vec<String>,
}

type Fault = (StatusCode, Json<serde_json::Value>);
type Reply<T> = std::result::Result<T, Fault>;
fn fault(status: StatusCode, code: &str) -> Fault {
    (status, Json(json!({"detail":code,"code":code})))
}

pub fn sample_cases(count: usize) -> EvalCases {
    EvalCases {
        dataset_id: "demo".into(),
        revision: 1,
        cases: (0..count)
            .map(|index| DatasetCase {
                id: format!("case-{index}"),
                messages: vec![Message {
                    role: "user".into(),
                    content: format!("Case {index}"),
                }],
                expected: String::new(),
                included: true,
                source: Source {
                    finding_id: "1".into(),
                },
                meta: BTreeMap::from([
                    ("finding_id".into(), "1".into()),
                    (
                        "priority".into(),
                        if index < 2 { "high" } else { "low" }.into(),
                    ),
                ]),
            })
            .collect(),
    }
}

pub fn router(dataset: EvalCases, key: String) -> Router {
    let stub = Stub {
        key,
        store: Arc::new(Mutex::new(Store {
            dataset,
            runs: BTreeMap::new(),
            specs: BTreeMap::new(),
            results: BTreeMap::new(),
            traces: BTreeMap::new(),
            keys: BTreeMap::new(),
            verdicts: BTreeMap::new(),
            completed: Vec::new(),
        })),
    };
    Router::new()
        .route("/lens/datasets/resolve", get(resolve))
        .route(
            "/lens/datasets/{dataset}/revisions/{revision}/cases",
            get(cases),
        )
        .route("/lens/evals/runs", post(create).get(listing))
        .route("/lens/evals/runs/{run}", get(lookup))
        .route("/lens/evals/runs/{run}/results/{case}/{trial}", put(result))
        .route("/lens/evals/runs/{run}/finish", post(finish))
        .route("/_dev/traces", post(trace))
        .layer(middleware::from_fn_with_state(stub.clone(), authorize))
        .with_state(stub)
}

async fn authorize(
    State(stub): State<Stub>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(format!("Bearer {}", stub.key).as_str())
    {
        return fault(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if !matches!(
        headers
            .get("X-Lens-Contract")
            .and_then(|value| value.to_str().ok()),
        Some("1" | "2")
    ) {
        return fault(StatusCode::CONFLICT, "contract_version").into_response();
    }
    next.run(request).await
}

#[derive(Deserialize)]
struct Resolve {
    name: String,
    revision: Option<u64>,
}
async fn resolve(
    State(stub): State<Stub>,
    Query(query): Query<Resolve>,
) -> Reply<Json<ResolvedDataset>> {
    let store = stub.store.lock().expect("stub lock");
    if query.name != "demo" {
        return Err(fault(StatusCode::NOT_FOUND, "dataset_not_found"));
    }
    if query
        .revision
        .is_some_and(|revision| revision != store.dataset.revision)
    {
        return Err(fault(StatusCode::NOT_FOUND, "revision_not_found"));
    }
    Ok(Json(ResolvedDataset {
        id: store.dataset.dataset_id.clone(),
        name: query.name,
        revision: store.dataset.revision,
    }))
}

async fn cases(
    State(stub): State<Stub>,
    RoutePath((dataset, revision)): RoutePath<(String, u64)>,
) -> Reply<Json<EvalCases>> {
    let store = stub.store.lock().expect("stub lock");
    if store.dataset.dataset_id != dataset {
        return Err(fault(StatusCode::NOT_FOUND, "dataset_not_found"));
    }
    if store.dataset.revision != revision {
        return Err(fault(StatusCode::NOT_FOUND, "revision_not_found"));
    }
    Ok(Json(store.dataset.clone()))
}

async fn create(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(spec): Json<CreateEvalRun>,
) -> Reply<(StatusCode, Json<EvalRun>)> {
    let key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| fault(StatusCode::UNPROCESSABLE_ENTITY, "idempotency_key"))?;
    let body = serde_json::to_string(&spec).expect("run serializes");
    let mut store = stub.store.lock().expect("stub lock");
    if let Some((original, id)) = store.keys.get(key) {
        if original != &body {
            return Err(fault(StatusCode::CONFLICT, "idempotency_key"));
        }
        return Ok((StatusCode::CREATED, Json(store.runs[id].clone())));
    }
    if store.dataset.dataset_id != spec.dataset_id {
        return Err(fault(StatusCode::NOT_FOUND, "dataset_not_found"));
    }
    if store.dataset.revision != spec.revision {
        return Err(fault(StatusCode::NOT_FOUND, "revision_not_found"));
    }
    let ids = store
        .dataset
        .cases
        .iter()
        .filter(|case| {
            case.included
                && spec
                    .case_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&case.id))
        })
        .map(|case| case.id.clone())
        .collect::<BTreeSet<_>>();
    if ids.is_empty()
        || spec
            .case_ids
            .as_ref()
            .is_some_and(|requested| requested.iter().cloned().collect::<BTreeSet<_>>() != ids)
    {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "unknown_case"));
    }
    let check = EvalSpec {
        name: spec.eval.clone(),
        data: "demo".into(),
        scores: spec.scorers.clone(),
        baseline: "main".into(),
        trials: spec.trials,
        gate: spec.gate.clone(),
        concurrency: 1,
        timeout_seconds: 1.0,
        finding: None,
        case_ids: None,
    };
    if check.validate().is_err()
        || spec.timeout_per_trial_ms == 0
        || Instant::now()
            .checked_add(Duration::from_millis(spec.timeout_per_trial_ms))
            .is_none()
        || spec.agent.is_empty()
        || spec.version.is_empty()
        || spec.branch.is_empty()
    {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"));
    }
    if spec.baseline_run_id.is_some() && baseline_id(&store, &spec).is_none() {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"));
    }
    let id = Uuid::new_v4().simple().to_string();
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1:8765");
    let run = EvalRun {
        id: id.clone(),
        status: RunStatus::Running,
        eval: spec.eval.clone(),
        agent: spec.agent.clone(),
        version: spec.version.clone(),
        branch: spec.branch.clone(),
        pr: spec.pr,
        ci_url: spec.ci_url.clone(),
        url: format!("http://{host}/lens/evals/runs/{id}"),
        expected_trials: ids.len() * spec.trials,
        received_trials: 0,
        summary: None,
        failure: String::new(),
    };
    store.runs.insert(id.clone(), run.clone());
    store.specs.insert(id.clone(), spec);
    store.results.insert(id.clone(), BTreeMap::new());
    store.keys.insert(key.into(), (body, id));
    Ok((StatusCode::CREATED, Json(run)))
}

async fn lookup(
    State(stub): State<Stub>,
    RoutePath(run): RoutePath<String>,
) -> Reply<Json<EvalRun>> {
    let mut store = stub.store.lock().expect("stub lock");
    advance(&mut store, &run);
    store
        .runs
        .get(&run)
        .cloned()
        .map(Json)
        .ok_or_else(|| fault(StatusCode::NOT_FOUND, "run_not_found"))
}

async fn listing(State(stub): State<Stub>) -> Json<Vec<EvalRun>> {
    Json(
        stub.store
            .lock()
            .expect("stub lock")
            .runs
            .values()
            .cloned()
            .collect(),
    )
}

async fn result(
    State(stub): State<Stub>,
    RoutePath((run_id, case_id, trial)): RoutePath<(String, String, usize)>,
    Json(result): Json<CaseResult>,
) -> Reply<StatusCode> {
    let mut store = stub.store.lock().expect("stub lock");
    let run = store
        .runs
        .get(&run_id)
        .ok_or_else(|| fault(StatusCode::NOT_FOUND, "run_not_found"))?;
    if !matches!(run.status, RunStatus::Running) {
        return Err(fault(StatusCode::CONFLICT, "run_closed"));
    }
    let spec = &store.specs[&run_id];
    if !store
        .dataset
        .cases
        .iter()
        .any(|case| case.id == case_id && case.included)
        || spec
            .case_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&case_id))
    {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "unknown_case"));
    }
    if trial >= spec.trials || result.validate().is_err() {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "invalid_result"));
    }
    let results = store.results.get_mut(&run_id).expect("run results");
    let key = (case_id, trial);
    let received = results
        .get(&key)
        .map_or_else(Instant::now, |previous| previous.received);
    results.insert(key, Submission { result, received });
    let count = results.len();
    store
        .runs
        .get_mut(&run_id)
        .expect("run exists")
        .received_trials = count;
    Ok(StatusCode::NO_CONTENT)
}

fn passes(result: &CaseResult) -> bool {
    result.error.is_none()
        && result
            .trace
            .as_ref()
            .is_some_and(|trace| trace.value.contains("pass"))
}

fn baseline_id<'a>(store: &'a Store, spec: &CreateEvalRun) -> Option<&'a String> {
    store.completed.iter().rev().find(|prior| {
        let previous = &store.specs[*prior];
        spec.baseline_run_id.as_ref().is_none_or(|id| id == *prior)
            && previous.branch == "main"
            && previous.eval == spec.eval
            && previous.agent == spec.agent
            && previous.dataset_id == spec.dataset_id
            && previous.revision == spec.revision
            && previous.scorers == spec.scorers
            && previous.case_ids == spec.case_ids
            && previous.trials == spec.trials
            && previous.agent_io == spec.agent_io
    })
}

fn summary(
    store: &Store,
    id: &str,
    results: &BTreeMap<(String, usize), CaseResult>,
) -> (Summary, BTreeMap<String, bool>) {
    let spec = &store.specs[id];
    let cases = store
        .dataset
        .cases
        .iter()
        .filter(|case| {
            case.included
                && spec
                    .case_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&case.id))
        })
        .collect::<Vec<_>>();
    let baseline = baseline_id(store, spec);
    let verdicts = cases
        .iter()
        .map(|case| {
            (
                case.id.clone(),
                (0..spec.trials)
                    .filter(|trial| results.get(&(case.id.clone(), *trial)).is_some_and(passes))
                    .count()
                    * 2
                    > spec.trials,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let prior = baseline.and_then(|id| store.verdicts.get(id));
    let diff = |case: &DatasetCase| CaseDiff {
        case_id: case.id.clone(),
        title: case.id.clone(),
        critical: case
            .meta
            .get("priority")
            .is_some_and(|value| value == "high"),
        baseline_url: baseline
            .map(|id| case_url(&store.runs[id].url, &case.id))
            .unwrap_or_default(),
        candidate_url: case_url(&store.runs[id].url, &case.id),
    };
    let regressions = cases
        .iter()
        .filter(|case| {
            prior.and_then(|prior| prior.get(&case.id)) == Some(&true) && !verdicts[&case.id]
        })
        .map(|case| diff(case))
        .collect::<Vec<_>>();
    let fixed = cases
        .iter()
        .filter(|case| {
            prior.and_then(|prior| prior.get(&case.id)) == Some(&false) && verdicts[&case.id]
        })
        .map(|case| diff(case))
        .collect();
    let passed = verdicts.values().filter(|passed| **passed).count();
    let errors = cases
        .iter()
        .map(|case| {
            (0..spec.trials)
                .filter(|trial| {
                    results
                        .get(&(case.id.clone(), *trial))
                        .is_none_or(|result| result.error.is_some())
                })
                .count()
        })
        .sum();
    let cost = results
        .values()
        .map(|result| result.cost_usd.unwrap_or(0.0))
        .sum::<f64>()
        / cases.len() as f64;
    let score = results.values().filter(|result| passes(result)).count() as f64
        / (cases.len() * spec.trials) as f64;
    let scores = scorer_names(&spec.scorers)
        .into_iter()
        .map(|name| (name, score))
        .collect::<BTreeMap<_, _>>();
    let critical = regressions.iter().filter(|case| case.critical).count();
    let rate = passed as f64 / cases.len() as f64;
    let failures = [
        (
            baseline.is_some()
                && spec
                    .gate
                    .regressions
                    .is_some_and(|max| regressions.len() as u64 > max),
            format!(
                "{} regressions (max {})",
                regressions.len(),
                spec.gate.regressions.unwrap_or(0)
            ),
        ),
        (
            baseline.is_some() && spec.gate.critical.is_some_and(|max| critical as u64 > max),
            format!(
                "{critical} critical regressions (max {})",
                spec.gate.critical.unwrap_or(0)
            ),
        ),
        (
            spec.gate.pass_rate.is_some_and(|min| rate < min),
            "Pass rate below minimum".into(),
        ),
        (
            spec.gate.cost_per_case.is_some_and(|max| cost > max),
            "Cost per case above maximum".into(),
        ),
    ]
    .into_iter()
    .filter_map(|(failed, reason)| failed.then_some(reason))
    .chain(
        spec.gate
            .min
            .iter()
            .filter(|(name, min)| scores.get(*name).is_none_or(|score| score < *min))
            .map(|(name, min)| format!("{name} below minimum {min}")),
    )
    .collect::<Vec<_>>();
    let gate = GateResult {
        passed: failures.is_empty(),
        reasons: failures
            .into_iter()
            .chain(
                baseline
                    .is_none()
                    .then(|| format!("no baseline on main for rev {}", spec.revision)),
            )
            .collect(),
    };
    (
        Summary {
            passed,
            total: cases.len(),
            pass_rate: rate,
            cost_per_case: cost,
            scores,
            errors,
            baseline_run_id: baseline.cloned(),
            baseline_version: baseline.map(|id| store.runs[id].version.clone()),
            regressions,
            fixed,
            gate,
        },
        verdicts,
    )
}

fn case_url(base: &str, case: &str) -> String {
    let Ok(mut url) = url::Url::parse(base) else {
        return base.to_owned();
    };
    url.query_pairs_mut().append_pair("eval_case", case);
    url.into()
}

async fn finish(
    State(stub): State<Stub>,
    RoutePath(id): RoutePath<String>,
) -> Reply<(StatusCode, Json<EvalRun>)> {
    let mut store = stub.store.lock().expect("stub lock");
    let run = store
        .runs
        .get(&id)
        .ok_or_else(|| fault(StatusCode::NOT_FOUND, "run_not_found"))?;
    if !matches!(run.status, RunStatus::Running) {
        return Err(fault(StatusCode::CONFLICT, "run_closed"));
    }
    let pending = EvalRun {
        status: RunStatus::Scoring,
        ..run.clone()
    };
    store.runs.insert(id.clone(), pending.clone());
    advance(&mut store, &id);
    Ok((StatusCode::ACCEPTED, Json(pending)))
}

fn advance(store: &mut Store, id: &str) {
    if !store
        .runs
        .get(id)
        .is_some_and(|run| matches!(run.status, RunStatus::Scoring))
    {
        return;
    }
    let now = Instant::now();
    let wait = Wait {
        timeout: Duration::from_millis(store.specs[id].timeout_per_trial_ms),
        ..Wait::default()
    };
    let results: Option<BTreeMap<_, _>> = store.results[id]
        .iter()
        .map(|(key, value)| {
            wait.resolve(value, &store.traces, &store.specs[id].version, now)
                .map(|result| (key.clone(), result))
        })
        .collect();
    let Some(results) = results else {
        return;
    };
    let (summary, verdicts) = summary(store, id, &results);
    store.runs.insert(
        id.to_owned(),
        EvalRun {
            status: RunStatus::Done,
            summary: Some(summary),
            ..store.runs[id].clone()
        },
    );
    store.verdicts.insert(id.to_owned(), verdicts);
    store.completed.push(id.to_owned());
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceUpdate {
    trace: TraceRef,
    agent_version: String,
    #[serde(default)]
    root_ended: bool,
}

async fn trace(State(stub): State<Stub>, Json(update): Json<TraceUpdate>) -> Reply<StatusCode> {
    let result = CaseResult {
        trace: Some(update.trace.clone()),
        ..CaseResult::default()
    };
    if result.validate().is_err() || update.agent_version.is_empty() {
        return Err(fault(StatusCode::UNPROCESSABLE_ENTITY, "invalid_trace"));
    }
    let mut store = stub.store.lock().expect("stub lock");
    let key = (update.trace.attribute, update.trace.value);
    if let Some(previous) = store.traces.get(&key) {
        if previous.version != update.agent_version {
            return Err(fault(StatusCode::CONFLICT, "trace_version_conflict"));
        }
        if previous.ended {
            return Ok(StatusCode::NO_CONTENT);
        }
    }
    store.traces.insert(
        key,
        Trace {
            version: update.agent_version,
            ended: update.root_ended,
            updated: Instant::now(),
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn serve(host: &str, port: u16, dataset: Option<&Path>) -> Result<()> {
    let ip: IpAddr = match host {
        "127.0.0.1" | "localhost" => std::net::Ipv4Addr::LOCALHOST.into(),
        "::1" => std::net::Ipv6Addr::LOCALHOST.into(),
        _ => {
            return Err(Error::Configuration(
                "The development server binds only to loopback",
            ));
        }
    };
    let data = match dataset {
        Some(path) => {
            serde_json::from_str(&std::fs::read_to_string(path)?).map_err(Error::Response)?
        }
        None => sample_cases(36),
    };
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip, port)).await?;
    println!(
        "Development server at http://{}: synthetic scoring only",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        router(
            data,
            crate::setup::env_value("LENS_API_KEY").unwrap_or_else(|| "lens-dev".into()),
        ),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::case_url;
    use rstest::rstest;

    #[rstest]
    #[case::plain("http://lens/runs/run")]
    #[case::query_and_fragment("http://lens/ui/?eval_run=run&tab=evals#results")]
    fn case_links_encode_the_case_without_losing_the_run_location(#[case] base: &str) {
        let case = "case / with&symbols";
        let original = url::Url::parse(base).unwrap();
        let actual = url::Url::parse(&case_url(base, case)).unwrap();
        assert_eq!(actual.path(), original.path());
        assert_eq!(actual.fragment(), original.fragment());
        assert_eq!(
            actual
                .query_pairs()
                .filter(|(key, _)| key == "eval_case")
                .collect::<Vec<_>>(),
            vec![("eval_case".into(), case.into())]
        );
        assert_eq!(
            actual
                .query_pairs()
                .filter(|(key, _)| key != "eval_case")
                .collect::<Vec<_>>(),
            original.query_pairs().collect::<Vec<_>>()
        );
    }

    #[rstest]
    fn invalid_base_is_returned_without_inventing_a_destination() {
        assert_eq!(case_url("/relative-run", "case"), "/relative-run");
    }
}
