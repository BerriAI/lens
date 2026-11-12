use std::collections::{BTreeMap, BTreeSet, HashMap};

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

/// One gateway ledger row strict matching tied to a call, with the deployment it recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Charge {
    pub identity: LedgerRequest,
    pub spend: Option<f64>,
    pub column: Column,
}

/// One leaf model call. `charges` is empty when strict matching did not settle on ledger rows,
/// and then `fallback` names the column from the span alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub span_id: String,
    pub agent: String,
    pub duration_ms: f64,
    pub charges: Vec<Charge>,
    pub fallback: Column,
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

pub const MULTIPLE_DEPLOYMENTS: &str = "multiple deployments";

/// Resolved model calls per run; runs absent from the map were not resolved.
pub type RunCalls = BTreeMap<(String, String, String), Vec<Call>>;

pub fn run_key(run: &Run) -> (String, String, String) {
    (
        run.team_id.clone(),
        run.api_key_hash.clone(),
        run.trace_id.clone(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Attribution {
    Own,
    Other,
    Shared,
}

/// One request: every exported call that shares a ledger row with another is the same request,
/// so overlapping or duplicated exports never count a charge twice.
struct Request<'a> {
    run: &'a Run,
    calls: Vec<(&'a Run, &'a Call)>,
    charges: BTreeMap<&'a LedgerRequest, &'a Charge>,
    attribution: Attribution,
}

impl Request<'_> {
    fn spend(&self) -> Option<f64> {
        if self.charges.is_empty() {
            return None;
        }
        finite_sum(self.charges.values().map(|charge| charge.spend))
    }

    /// A retry that crossed deployments is a whole-request cost, not one deployment's price.
    fn column(&self) -> Column {
        let columns: BTreeSet<String> = self.charges.values().map(|charge| charge.column.key()).collect();
        if columns.len() > 1 {
            return Column::UnknownDeployment {
                model_group: MULTIPLE_DEPLOYMENTS.into(),
            };
        }
        self.charges
            .values()
            .next()
            .map_or(&self.calls[0].1.fallback, |charge| &charge.column)
            .clone()
    }

    fn duration_ms(&self) -> f64 {
        self.calls
            .iter()
            .map(|(_, call)| call.duration_ms)
            .fold(0.0, f64::max)
    }
}

fn finite_sum(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values
        .try_fold(0.0, |total, value| {
            let value = value.filter(|value| value.is_finite())?;
            Some(total + value)
        })
        .filter(|total: &f64| total.is_finite())
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
    fn requests(&self) -> Vec<Request<'a>> {
        let mut groups: Vec<Option<Request<'a>>> = Vec::new();
        let mut by_charge: HashMap<&'a LedgerRequest, usize> = HashMap::new();
        let mut spans: BTreeSet<(&'a str, &'a str, &'a str, &'a str)> = BTreeSet::new();
        for run in self.runs {
            for call in self.calls.get(&run_key(run)).into_iter().flatten() {
                if call.charges.is_empty() {
                    if spans.insert((
                        run.team_id.as_str(),
                        run.api_key_hash.as_str(),
                        run.trace_id.as_str(),
                        call.span_id.as_str(),
                    )) {
                        groups.push(Some(Request {
                            run,
                            calls: vec![(run, call)],
                            charges: BTreeMap::new(),
                            attribution: Attribution::Own,
                        }));
                    }
                    continue;
                }
                let linked: BTreeSet<usize> = call
                    .charges
                    .iter()
                    .filter_map(|charge| by_charge.get(&charge.identity).copied())
                    .collect();
                let target = linked.first().copied().unwrap_or_else(|| {
                    groups.push(Some(Request {
                        run,
                        calls: Vec::new(),
                        charges: BTreeMap::new(),
                        attribution: Attribution::Own,
                    }));
                    groups.len() - 1
                });
                for other in linked.iter().skip(1) {
                    if let Some(merged) = groups[*other].take() {
                        for identity in merged.charges.keys() {
                            by_charge.insert(identity, target);
                        }
                        if let Some(group) = groups[target].as_mut() {
                            group.calls.extend(merged.calls);
                            group.charges.extend(merged.charges);
                        }
                    }
                }
                if let Some(group) = groups[target].as_mut() {
                    group.calls.push((run, call));
                    for charge in &call.charges {
                        group.charges.entry(&charge.identity).or_insert(charge);
                        by_charge.insert(&charge.identity, target);
                    }
                }
            }
        }
        groups
            .into_iter()
            .flatten()
            .map(|mut request| {
                let runs: BTreeSet<(&str, &str, &str)> = request
                    .calls
                    .iter()
                    .map(|(run, _)| (run.team_id.as_str(), run.api_key_hash.as_str(), run.trace_id.as_str()))
                    .collect();
                let own = request
                    .calls
                    .iter()
                    .filter(|(run, call)| attributed(run, call, self.agent))
                    .count();
                request.attribution = match own {
                    _ if runs.len() > 1 => Attribution::Shared,
                    0 => Attribution::Other,
                    own if own == request.calls.len() => Attribution::Own,
                    _ => Attribution::Shared,
                };
                request
            })
            .collect()
    }

    fn own(&self) -> Vec<Request<'a>> {
        self.requests()
            .into_iter()
            .filter(|request| request.attribution == Attribution::Own)
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
        let requests = self.requests();
        let count = |attribution| {
            requests
                .iter()
                .filter(|request| request.attribution == attribution)
                .count() as u64
        };
        let own: Vec<&Request> = requests
            .iter()
            .filter(|request| request.attribution == Attribution::Own)
            .collect();
        let priced: Vec<f64> = own.iter().filter_map(|request| request.spend()).collect();
        ProfileTotals {
            runs: self.runs.len() as u64,
            requests: own.len() as u64,
            matched_requests: own
                .iter()
                .filter(|request| !request.charges.is_empty())
                .count() as u64,
            cost_observations: priced.len() as u64,
            spend_usd: (!priced.is_empty())
                .then(|| finite_sum(priced.iter().copied().map(Some)))
                .flatten(),
            other_agent_requests: count(Attribution::Other),
            shared_requests: count(Attribution::Shared),
        }
    }

    pub fn by_profile(&self) -> BTreeMap<&'a str, ProfileFigures> {
        let mut figures: BTreeMap<&str, ProfileFigures> = BTreeMap::new();
        for run in self.runs {
            figures.entry(run.profile_id.as_str()).or_default().runs += 1;
        }
        let mut priced: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for request in self.own() {
            let profile = request.run.profile_id.as_str();
            figures.entry(profile).or_default().requests += 1;
            if let Some(spend) = request.spend() {
                priced.entry(profile).or_default().push(spend);
            }
        }
        for (profile, values) in priced {
            let entry = figures.entry(profile).or_default();
            entry.cost_observations = values.len() as u64;
            entry.spend = finite_sum(values.into_iter().map(Some));
        }
        figures
    }

    /// Cells keyed by profile and canonical column key; a deployment keeps one cell even when
    /// its display labels changed between ledger rows.
    pub fn cells(&self, profile: Option<&str>) -> Vec<(Column, MatrixCell)> {
        let mut grouped: BTreeMap<(String, String), (Column, Vec<(Option<f64>, f64)>)> =
            BTreeMap::new();
        for request in self
            .own()
            .iter()
            .filter(|request| profile.is_none_or(|profile| request.run.profile_id == profile))
        {
            let column = request.column();
            grouped
                .entry((request.run.profile_id.clone(), column.key()))
                .or_insert_with(|| (column, Vec::new()))
                .1
                .push((request.spend(), request.duration_ms()));
        }
        grouped
            .into_iter()
            .map(|((profile_id, column_key), (column, requests))| {
                let priced: Vec<f64> = requests.iter().filter_map(|(spend, _)| *spend).collect();
                let latencies: Vec<f64> = requests
                    .iter()
                    .map(|(_, duration)| *duration)
                    .filter(|duration| duration.is_finite() && *duration > 0.0)
                    .collect();
                (
                    column,
                    MatrixCell {
                        profile_id,
                        column_key,
                        request_count: requests.len() as u64,
                        avg_cost_usd: finite_sum(priced.iter().copied().map(Some))
                            .filter(|_| !priced.is_empty())
                            .map(|total| total / priced.len() as f64),
                        cost_observations: priced.len() as u64,
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
        let own = self.own();
        self.runs
            .iter()
            .filter(|run| run.profile_id == profile_id)
            .take(limit)
            .map(|run| {
                let requests: Vec<&Request> = own
                    .iter()
                    .filter(|request| std::ptr::eq(request.run, run))
                    .collect();
                let spend = finite_sum(requests.iter().map(|request| request.spend()))
                    .filter(|_| !requests.is_empty());
                ProfileRunRef {
                    trace_id: run.trace_id.clone(),
                    start_ms: run.start_ms,
                    input_preview: run.input_preview.clone(),
                    request_count: requests.len() as u64,
                    spend_usd: spend,
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
    use rstest::rstest;

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

    fn deployment(model_id: &str, model_group: &str) -> Column {
        Column::Deployment {
            model_id: model_id.into(),
            model_group: model_group.into(),
            provider: "anthropic".into(),
        }
    }

    fn charge(request_id: &str, spend: Option<f64>) -> Charge {
        Charge {
            identity: LedgerRequest {
                team_id: "team".into(),
                start_ms: 1,
                request_id: request_id.into(),
            },
            spend,
            column: deployment("dep-1", "sonnet"),
        }
    }

    fn call(span_id: &str, agent: &str, charges: Vec<Charge>) -> Call {
        Call {
            span_id: span_id.into(),
            agent: agent.into(),
            duration_ms: 100.0,
            charges,
            fallback: Column::Unmatched {
                model: "claude".into(),
            },
        }
    }

    fn view<'a>(runs: &'a [Run], calls: &'a RunCalls, agent: &'a str) -> View<'a> {
        View {
            runs,
            calls,
            agent,
            truncated: false,
        }
    }

    fn resolved(run: &Run, calls: Vec<Call>) -> RunCalls {
        [(run_key(run), calls)].into_iter().collect()
    }

    #[rstest]
    fn another_agents_calls_in_a_shared_run_are_excluded_and_counted_apart() {
        let runs = vec![run("t1", "recorded:Coding", 2)];
        let calls = resolved(
            &runs[0],
            vec![
                call("s1", "planner", vec![charge("r1", Some(0.5))]),
                call("s2", "coder", vec![charge("r2", Some(2.0))]),
            ],
        );

        let totals = view(&runs, &calls, "planner").totals();

        assert_eq!(totals.runs, 1);
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.spend_usd, Some(0.5));
        assert_eq!(totals.other_agent_requests, 1);
    }

    #[rstest]
    fn overlapping_exports_of_one_request_charge_it_once() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let calls = resolved(
            &runs[0],
            vec![
                call("wrapper", "agent", vec![charge("a", Some(1.0)), charge("b", Some(2.0))]),
                call("attempt", "agent", vec![charge("a", Some(1.0))]),
            ],
        );

        let totals = view(&runs, &calls, "agent").totals();

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.spend_usd, Some(3.0));
    }

    #[rstest]
    fn a_request_exported_by_two_agents_is_shared_and_charged_to_neither() {
        let runs = vec![run("t1", "cluster:a", 2)];
        let calls = resolved(
            &runs[0],
            vec![
                call("s1", "planner", vec![charge("a", Some(1.0))]),
                call("s2", "coder", vec![charge("a", Some(1.0))]),
            ],
        );

        let totals = view(&runs, &calls, "planner").totals();

        assert_eq!(totals.requests, 0);
        assert_eq!(totals.shared_requests, 1);
        assert_eq!(totals.spend_usd, None);
    }

    #[rstest]
    fn a_request_with_an_unpriced_ledger_row_has_unknown_cost() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let calls = resolved(
            &runs[0],
            vec![call("s1", "agent", vec![charge("a", Some(1.0)), charge("b", None)])],
        );

        let view = view(&runs, &calls, "agent");
        let totals = view.totals();
        let cells = view.cells(None);

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.cost_observations, 0);
        assert_eq!(totals.spend_usd, None);
        assert_eq!(cells[0].1.avg_cost_usd, None);
    }

    #[rstest]
    fn unmatched_calls_count_as_requests_without_cost() {
        let runs = vec![run("t1", "unclassified", 1)];
        let calls = resolved(&runs[0], vec![call("s1", "agent", vec![]), call("s1", "agent", vec![])]);

        let view = view(&runs, &calls, "agent");
        let totals = view.totals();
        let cells = view.cells(None);

        assert_eq!(totals.requests, 1);
        assert_eq!(totals.matched_requests, 0);
        assert_eq!(totals.spend_usd, None);
        assert_eq!(cells[0].1.column_key, "unmatched:claude");
    }

    #[rstest]
    fn one_deployment_with_changing_labels_keeps_one_cell() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let relabeled = Charge {
            column: deployment("dep-1", "sonnet-renamed"),
            ..charge("b", Some(3.0))
        };
        let calls = resolved(
            &runs[0],
            vec![
                call("s1", "agent", vec![charge("a", Some(1.0))]),
                call("s2", "agent", vec![relabeled]),
            ],
        );

        let cells = view(&runs, &calls, "agent").cells(None);

        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].1.request_count, 2);
        assert_eq!(cells[0].1.avg_cost_usd, Some(2.0));
    }

    #[rstest]
    fn non_finite_totals_become_unknown() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let calls = resolved(
            &runs[0],
            vec![
                call("s1", "agent", vec![charge("a", Some(f64::MAX))]),
                call("s2", "agent", vec![charge("b", Some(f64::MAX))]),
            ],
        );

        let view = view(&runs, &calls, "agent");

        assert_eq!(view.totals().spend_usd, None);
        assert_eq!(view.by_profile()["cluster:a"].spend, None);
    }

    #[rstest]
    fn unresolved_runs_are_reported_in_coverage_not_as_zero_requests() {
        let runs = vec![run("t1", "cluster:a", 1), run("t2", "cluster:a", 1)];
        let calls = resolved(&runs[0], vec![call("s1", "agent", vec![charge("a", Some(1.0))])]);
        let view = View {
            runs: &runs,
            calls: &calls,
            agent: "agent",
            truncated: true,
        };

        let coverage = view.coverage();

        assert_eq!(coverage.runs_listed, 2);
        assert_eq!(coverage.runs_resolved, 1);
        assert!(coverage.truncated);
    }

    #[rstest]
    fn cells_take_the_nearest_rank_p95_and_leave_evals_empty() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let mut slow = call("s3", "agent", vec![charge("c", Some(1.0))]);
        slow.duration_ms = 1000.0;
        let calls = resolved(
            &runs[0],
            vec![
                call("s1", "agent", vec![charge("a", Some(1.0))]),
                call("s2", "agent", vec![charge("b", Some(4.0))]),
                slow,
            ],
        );

        let cells = view(&runs, &calls, "agent").cells(Some("cluster:a"));

        assert_eq!(cells[0].1.request_count, 3);
        assert_eq!(cells[0].1.avg_cost_usd, Some(2.0));
        assert_eq!(cells[0].1.p95_latency_ms, Some(1000.0));
        assert_eq!(cells[0].1.eval_success_rate, None);
        assert_eq!(cells[0].1.eval_sample_count, 0);
    }

    #[rstest]
    fn a_ledger_row_matched_from_two_runs_is_shared_not_given_to_the_first() {
        let runs = vec![run("t1", "cluster:a", 1), run("t2", "cluster:b", 1)];
        let calls: RunCalls = [
            (run_key(&runs[0]), vec![call("s1", "agent", vec![charge("a", Some(1.0))])]),
            (run_key(&runs[1]), vec![call("s2", "agent", vec![charge("a", Some(1.0))])]),
        ]
        .into_iter()
        .collect();

        let view = view(&runs, &calls, "agent");

        assert_eq!(view.totals().requests, 0);
        assert_eq!(view.totals().shared_requests, 1);
        assert!(view.cells(None).is_empty());
    }

    #[rstest]
    fn a_retry_across_deployments_is_not_priced_as_either_deployment() {
        let runs = vec![run("t1", "cluster:a", 1)];
        let fallback_attempt = Charge {
            column: deployment("dep-2", "haiku"),
            ..charge("b", Some(2.0))
        };
        let calls = resolved(
            &runs[0],
            vec![call("s1", "agent", vec![charge("a", Some(1.0)), fallback_attempt])],
        );

        let cells = view(&runs, &calls, "agent").cells(None);

        assert_eq!(cells.len(), 1);
        assert_eq!(
            cells[0].0,
            Column::UnknownDeployment {
                model_group: MULTIPLE_DEPLOYMENTS.into()
            }
        );
        assert_eq!(cells[0].1.avg_cost_usd, Some(3.0));
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
