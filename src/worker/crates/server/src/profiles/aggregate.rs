use std::collections::{BTreeMap, BTreeSet};

use lens_contract::profiles::{
    MatrixCell, ProfileCoverage, ProfileRunRef, ProfileSource, ProfileTotals, Taxonomy,
};
use litellm_traces::LedgerRequest;

/// A run in the selected agent's view, with the profile the shared SQL resolver gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    pub start_ms: i64,
    pub profile_id: String,
    pub category: String,
    pub input_preview: String,
    pub agent_count: u64,
}

/// One leaf model call, already priced by the shared trace resolver.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub span_id: String,
    pub agent: String,
    pub duration_ms: f64,
    pub spend: Option<f64>,
    pub requests: Vec<LedgerRequest>,
    pub column: Column,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Column {
    Deployment {
        model_id: String,
        model_group: String,
        provider: String,
    },
    UnknownDeployment {
        model_group: String,
    },
    Unmatched {
        model: String,
    },
}

impl Column {
    pub fn key(&self) -> String {
        match self {
            Self::Deployment { model_id, .. } => format!("id:{model_id}"),
            Self::UnknownDeployment { model_group } => format!("group:{model_group}"),
            Self::Unmatched { model } => format!("unmatched:{model}"),
        }
    }
}

/// Resolved model calls per run; runs absent from the map were not resolved.
pub type RunCalls = BTreeMap<(String, String, String), Vec<Call>>;

pub fn run_key(run: &Run) -> (String, String, String) {
    (
        run.team_id.clone(),
        run.api_key_hash.clone(),
        run.trace_id.clone(),
    )
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RequestIdentity {
    Ledger(Vec<LedgerRequest>),
    Span {
        team_id: String,
        api_key_hash: String,
        trace_id: String,
        span_id: String,
    },
}

/// A deduplicated request: two exported spans for one ledger request count once.
#[derive(Clone, Debug)]
struct Request<'a> {
    run: &'a Run,
    call: &'a Call,
}

fn identity(run: &Run, call: &Call) -> RequestIdentity {
    if call.requests.is_empty() {
        return RequestIdentity::Span {
            team_id: run.team_id.clone(),
            api_key_hash: run.api_key_hash.clone(),
            trace_id: run.trace_id.clone(),
            span_id: call.span_id.clone(),
        };
    }
    let mut ledger = call.requests.clone();
    ledger.sort();
    RequestIdentity::Ledger(ledger)
}

fn attributed(run: &Run, call: &Call, agent: &str) -> bool {
    call.agent == agent || (call.agent.is_empty() && run.agent_count <= 1)
}

pub struct View<'a> {
    pub runs: &'a [Run],
    pub calls: &'a RunCalls,
    pub agent: &'a str,
    pub truncated: bool,
}

impl<'a> View<'a> {
    fn requests(&self, own: bool) -> Vec<Request<'a>> {
        let mut seen = BTreeSet::new();
        self.runs
            .iter()
            .flat_map(|run| {
                self.calls
                    .get(&run_key(run))
                    .into_iter()
                    .flatten()
                    .map(move |call| Request { run, call })
            })
            .filter(|request| attributed(request.run, request.call, self.agent) == own)
            .filter(|request| seen.insert(identity(request.run, request.call)))
            .collect()
    }

    pub fn coverage(&self) -> ProfileCoverage {
        ProfileCoverage {
            runs_listed: self.runs.len() as u64,
            runs_resolved: self
                .runs
                .iter()
                .filter(|run| self.calls.contains_key(&run_key(run)))
                .count() as u64,
            truncated: self.truncated,
        }
    }

    pub fn totals(&self) -> ProfileTotals {
        let requests = self.requests(true);
        let figures = Figures::of(requests.iter().map(|request| request.call));
        let ledger: BTreeSet<&LedgerRequest> = requests
            .iter()
            .flat_map(|request| &request.call.requests)
            .collect();
        ProfileTotals {
            runs: self.runs.len() as u64,
            requests: requests.len() as u64,
            matched_requests: requests
                .iter()
                .filter(|request| !request.call.requests.is_empty())
                .count() as u64,
            deliveries: (!ledger.is_empty()).then_some(ledger.len() as u64),
            cost_observations: figures.cost_observations,
            spend_usd: figures.spend,
            other_agent_requests: self.requests(false).len() as u64,
        }
    }

    pub fn by_profile(&self) -> BTreeMap<&'a str, ProfileFigures> {
        let requests = self.requests(true);
        let mut figures: BTreeMap<&str, ProfileFigures> = BTreeMap::new();
        for run in self.runs {
            figures.entry(run.profile_id.as_str()).or_default().runs += 1;
        }
        for request in &requests {
            let entry = figures
                .entry(request.run.profile_id.as_str())
                .or_default();
            entry.requests += 1;
            if let Some(spend) = request.call.spend {
                entry.cost_observations += 1;
                entry.spend = Some(entry.spend.unwrap_or(0.0) + spend);
            }
        }
        figures
    }

    pub fn cells(&self, profile: Option<&str>) -> Vec<(Column, MatrixCell)> {
        let requests = self.requests(true);
        let mut grouped: BTreeMap<(&str, &Column), Vec<&Call>> = BTreeMap::new();
        for request in requests
            .iter()
            .filter(|request| profile.is_none_or(|profile| request.run.profile_id == profile))
        {
            grouped
                .entry((request.run.profile_id.as_str(), &request.call.column))
                .or_default()
                .push(request.call);
        }
        grouped
            .into_iter()
            .map(|((profile_id, column), calls)| {
                let figures = Figures::of(calls.iter().copied());
                let latencies: Vec<f64> = calls
                    .iter()
                    .map(|call| call.duration_ms)
                    .filter(|duration| duration.is_finite() && *duration > 0.0)
                    .collect();
                (
                    column.clone(),
                    MatrixCell {
                        profile_id: profile_id.to_owned(),
                        column_key: column.key(),
                        request_count: calls.len() as u64,
                        avg_cost_usd: figures.spend.map(|spend| {
                            spend / figures.cost_observations as f64
                        }),
                        cost_observations: figures.cost_observations,
                        p95_latency_ms: p95(&latencies),
                        latency_observations: latencies.len() as u64,
                        eval_success_rate: None,
                        eval_sample_count: 0,
                    },
                )
            })
            .collect()
    }

    pub fn runs_for(&self, profile_id: &str, limit: usize) -> Vec<ProfileRunRef> {
        let requests = self.requests(true);
        self.runs
            .iter()
            .filter(|run| run.profile_id == profile_id)
            .take(limit)
            .map(|run| {
                let own: Vec<&Call> = requests
                    .iter()
                    .filter(|request| std::ptr::eq(request.run, run))
                    .map(|request| request.call)
                    .collect();
                let figures = Figures::of(own.iter().copied());
                let complete = figures.cost_observations == own.len() as u64;
                ProfileRunRef {
                    trace_id: run.trace_id.clone(),
                    start_ms: run.start_ms,
                    input_preview: run.input_preview.clone(),
                    request_count: own.len() as u64,
                    spend_usd: figures.spend.filter(|_| complete),
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileFigures {
    pub runs: u64,
    pub requests: u64,
    pub cost_observations: u64,
    pub spend: Option<f64>,
}

struct Figures {
    cost_observations: u64,
    spend: Option<f64>,
}

impl Figures {
    fn of<'c>(calls: impl Iterator<Item = &'c Call>) -> Self {
        let priced: Vec<f64> = calls
            .filter_map(|call| call.spend)
            .filter(|spend| spend.is_finite())
            .collect();
        Self {
            cost_observations: priced.len() as u64,
            spend: (!priced.is_empty()).then(|| priced.iter().sum()),
        }
    }
}

/// Nearest-rank 95th percentile.
pub fn p95(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (0.95 * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.saturating_sub(1)).copied()
}

pub fn source(profile_id: &str) -> ProfileSource {
    match profile_id.split_once(':').map_or(profile_id, |(kind, _)| kind) {
        "recorded" => ProfileSource::Recorded,
        "task" => ProfileSource::Enhanced,
        "cluster" => ProfileSource::Local,
        "pending" => ProfileSource::Pending,
        "unknown" => ProfileSource::Unknown,
        "uncertain" => ProfileSource::Uncertain,
        _ => ProfileSource::Unclassified,
    }
}

pub fn display_name(profile_id: &str, category: &str, taxonomy: Option<&Taxonomy>) -> String {
    profile_id
        .strip_prefix("task:")
        .and_then(|task_id| {
            taxonomy?
                .tasks
                .iter()
                .find(|task| task.task_id == task_id)
                .map(|task| task.name.clone())
        })
        .unwrap_or_else(|| {
            if category.is_empty() {
                "Unnamed task".into()
            } else {
                category.to_owned()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};

    fn run(trace_id: &str, profile_id: &str, agent_count: u64) -> Run {
        Run {
            team_id: "team".into(),
            api_key_hash: "key".into(),
            trace_id: trace_id.into(),
            start_ms: 1,
            profile_id: profile_id.into(),
            category: profile_id.into(),
            input_preview: String::new(),
            agent_count,
        }
    }

    fn ledger(request_id: &str) -> LedgerRequest {
        LedgerRequest {
            team_id: "team".into(),
            start_ms: 1,
            request_id: request_id.into(),
        }
    }

    fn call(span_id: &str, agent: &str, spend: Option<f64>, ledger: Vec<LedgerRequest>) -> Call {
        Call {
            span_id: span_id.into(),
            agent: agent.into(),
            duration_ms: 100.0,
            spend,
            requests: ledger,
            column: Column::Deployment {
                model_id: "dep-1".into(),
                model_group: "sonnet".into(),
                provider: "anthropic".into(),
            },
        }
    }

    fn calls(entries: Vec<(&Run, Vec<Call>)>) -> RunCalls {
        entries
            .into_iter()
            .map(|(run, calls)| (run_key(run), calls))
            .collect()
    }

    #[fixture]
    fn multi_agent() -> (Vec<Run>, RunCalls) {
        let runs = vec![run("t1", "recorded:Coding", 2)];
        let resolved = calls(vec![(
            &runs[0],
            vec![
                call("s1", "planner", Some(0.5), vec![ledger("r1")]),
                call("s2", "coder", Some(2.0), vec![ledger("r2")]),
            ],
        )]);
        (runs, resolved)
    }

    #[rstest]
    fn another_agents_calls_in_a_shared_run_are_excluded_and_counted_apart(
        multi_agent: (Vec<Run>, RunCalls),
    ) {
        let (runs, resolved) = multi_agent;
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "planner",
            truncated: false,
        };

        let totals = view.totals();

        assert_eq!(totals.runs, 1);
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.spend_usd, Some(0.5));
        assert_eq!(totals.other_agent_requests, 1);
    }

    #[rstest]
    fn a_request_exported_twice_counts_once() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let resolved = calls(vec![(
            &runs[0],
            vec![
                call("s1", "agent", Some(1.0), vec![ledger("r1")]),
                call("s1-copy", "agent", Some(1.0), vec![ledger("r1")]),
            ],
        )]);
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "agent",
            truncated: false,
        };

        let totals = view.totals();

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.spend_usd, Some(1.0));
        assert_eq!(totals.deliveries, Some(1));
    }

    #[rstest]
    fn retries_count_one_request_with_one_delivery_per_ledger_attempt() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let resolved = calls(vec![(
            &runs[0],
            vec![call(
                "s1",
                "agent",
                Some(0.3),
                vec![ledger("attempt-1"), ledger("attempt-2")],
            )],
        )]);
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "agent",
            truncated: false,
        };

        let totals = view.totals();

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.matched_requests, 1);
        assert_eq!(totals.deliveries, Some(2));
    }

    #[rstest]
    fn unmatched_calls_keep_cost_and_deliveries_unknown_instead_of_zero() {
        let runs = vec![run("t1", "unclassified", 1)];
        let resolved = calls(vec![(&runs[0], vec![call("s1", "agent", None, vec![])])]);
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "agent",
            truncated: false,
        };

        let totals = view.totals();
        let cells = view.cells(None);

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.spend_usd, None);
        assert_eq!(totals.deliveries, None);
        assert_eq!(cells[0].1.avg_cost_usd, None);
        assert_eq!(cells[0].1.cost_observations, 0);
    }

    #[rstest]
    fn unresolved_runs_are_reported_in_coverage_not_as_zero_requests() {
        let runs = vec![run("t1", "cluster:a", 1), run("t2", "cluster:a", 1)];
        let resolved = calls(vec![(
            &runs[0],
            vec![call("s1", "agent", Some(1.0), vec![ledger("r1")])],
        )]);
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "agent",
            truncated: true,
        };

        let coverage = view.coverage();

        assert_eq!(coverage.runs_listed, 2);
        assert_eq!(coverage.runs_resolved, 1);
        assert!(coverage.truncated);
    }

    #[rstest]
    fn cells_average_only_priced_requests_and_take_the_nearest_rank_p95() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let mut slow = call("s3", "agent", None, vec![]);
        slow.duration_ms = 1000.0;
        let resolved = calls(vec![(
            &runs[0],
            vec![
                call("s1", "agent", Some(1.0), vec![ledger("r1")]),
                call("s2", "agent", Some(3.0), vec![ledger("r2")]),
                slow,
            ],
        )]);
        let view = View {
            runs: &runs,
            calls: &resolved,
            agent: "agent",
            truncated: false,
        };

        let cells = view.cells(Some("cluster:a"));
        let deployment = cells
            .iter()
            .find(|(column, _)| matches!(column, Column::Deployment { .. }))
            .map(|(_, cell)| cell)
            .expect("deployment cell");

        assert_eq!(deployment.request_count, 3);
        assert_eq!(deployment.avg_cost_usd, Some(2.0));
        assert_eq!(deployment.cost_observations, 2);
        assert_eq!(deployment.p95_latency_ms, Some(1000.0));
        assert_eq!(deployment.eval_success_rate, None);
        assert_eq!(deployment.eval_sample_count, 0);
    }

    #[rstest]
    #[case::empty(&[], None)]
    #[case::single(&[4.0], Some(4.0))]
    #[case::twenty(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0], Some(19.0))]
    fn p95_uses_the_nearest_rank(#[case] values: &[f64], #[case] expected: Option<f64>) {
        assert_eq!(p95(values), expected);
    }

    #[rstest]
    #[case::recorded("recorded:Coding", ProfileSource::Recorded)]
    #[case::enhanced("task:abc", ProfileSource::Enhanced)]
    #[case::local("cluster:abc", ProfileSource::Local)]
    #[case::pending("pending", ProfileSource::Pending)]
    #[case::unknown("unknown", ProfileSource::Unknown)]
    #[case::uncertain("uncertain", ProfileSource::Uncertain)]
    #[case::unclassified("unclassified", ProfileSource::Unclassified)]
    fn profile_ids_map_to_their_source(#[case] profile_id: &str, #[case] expected: ProfileSource) {
        assert_eq!(source(profile_id), expected);
    }
}
