use std::collections::HashMap;

use lens_contract::profiles::{DiscoveryPoint, MapPoint, Taxonomy};

use super::aggregate::Run;

/// Positions that discovery computed for its sample, for the sampled runs still assigned to the
/// task they were discovered in.
pub fn points(runs: &[Run], taxonomy: Option<&Taxonomy>) -> Vec<MapPoint> {
    let Some(taxonomy) = taxonomy else {
        return Vec::new();
    };
    let sampled: HashMap<(&str, &str, &str), &DiscoveryPoint> = taxonomy
        .points
        .iter()
        .map(|point| {
            (
                (
                    point.team_id.as_str(),
                    point.api_key_hash.as_str(),
                    point.trace_id.as_str(),
                ),
                point,
            )
        })
        .collect();
    runs.iter()
        .filter_map(|run| {
            let task_id = run.profile_id.strip_prefix("task:")?;
            let point = sampled.get(&(
                run.team_id.as_str(),
                run.api_key_hash.as_str(),
                run.trace_id.as_str(),
            ))?;
            (point.task_id.as_deref() == Some(task_id)).then(|| MapPoint {
                trace_id: run.trace_id.clone(),
                profile_id: run.profile_id.clone(),
                x: point.x,
                y: point.y,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::{TaxonomyModels, TaxonomyStatus};
    use rstest::rstest;

    fn run(team_id: &str, trace_id: &str, profile_id: &str) -> Run {
        Run {
            team_id: team_id.into(),
            api_key_hash: "key".into(),
            trace_id: trace_id.into(),
            start_ms: 0,
            end_ms: 0,
            profile_id: profile_id.into(),
            category: String::new(),
            input_preview: String::new(),
            agent_count: 1,
        }
    }

    fn taxonomy() -> Taxonomy {
        Taxonomy {
            taxonomy_id: "tax".into(),
            agent: "agent".into(),
            version: 1,
            revision: 1,
            status: TaxonomyStatus::Frozen,
            tasks: Vec::new(),
            points: vec![DiscoveryPoint {
                team_id: "team".into(),
                api_key_hash: "key".into(),
                trace_id: "sampled".into(),
                task_id: Some("a".into()),
                x: 10.0,
                y: 20.0,
            }],
            job: None,
            discovered_at: None,
            frozen_at: None,
            error: None,
            models: TaxonomyModels::default(),
            discovery: None,
            windows: Vec::new(),
        }
    }

    #[rstest]
    #[case::sampled_and_still_in_its_task(run("team", "sampled", "task:a"), Some((10.0, 20.0)))]
    #[case::reassigned_to_another_task(run("team", "sampled", "task:b"), None)]
    #[case::recorded_category(run("team", "sampled", "recorded:Coding"), None)]
    #[case::not_in_the_sample(run("team", "later", "task:a"), None)]
    #[case::same_trace_in_another_team(run("other", "sampled", "task:a"), None)]
    fn only_runs_placed_by_discovery_get_a_position(
        #[case] listed: Run,
        #[case] expected: Option<(f64, f64)>,
    ) {
        let points = points(&[listed], Some(&taxonomy()));

        assert_eq!(points.first().map(|point| (point.x, point.y)), expected);
    }

    #[rstest]
    fn without_an_active_taxonomy_nothing_is_placed() {
        assert_eq!(
            points(&[run("team", "sampled", "task:a")], None),
            Vec::new()
        );
    }
}
