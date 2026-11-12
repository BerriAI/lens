use std::collections::BTreeMap;

use lens_contract::profiles::{MapPlacement, MapPoint, MapPosition, Taxonomy};
use sha2::{Digest, Sha256};

use super::aggregate::Run;

pub const MAX_POINTS: usize = 500;
const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;

/// Bubble centers. Enhanced tasks sit at the centroid of their discovery sample; every other
/// profile is placed on a spiral by rank, which says nothing about similarity.
pub fn centers(ranked: &[&str], taxonomy: Option<&Taxonomy>) -> BTreeMap<String, MapPosition> {
    let count = ranked.len().max(1) as f64;
    ranked
        .iter()
        .enumerate()
        .map(|(rank, profile_id)| {
            let discovered = profile_id
                .strip_prefix("task:")
                .and_then(|task_id| centroid(taxonomy?, task_id));
            let position = discovered.unwrap_or_else(|| {
                let radius = 34.0 * ((rank as f64 + 0.5) / count).sqrt();
                let angle = rank as f64 * GOLDEN_ANGLE;
                MapPosition {
                    x: 50.0 + radius * angle.cos(),
                    y: 50.0 + radius * angle.sin(),
                }
            });
            ((*profile_id).to_owned(), position)
        })
        .collect()
}

fn centroid(taxonomy: &Taxonomy, task_id: &str) -> Option<MapPosition> {
    let points: Vec<_> = taxonomy
        .points
        .iter()
        .filter(|point| point.task_id.as_deref() == Some(task_id))
        .collect();
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    Some(MapPosition {
        x: points.iter().map(|point| point.x).sum::<f64>() / count,
        y: points.iter().map(|point| point.y).sum::<f64>() / count,
    })
}

/// Discovery samples keep the position the discovery run computed for them; every later run is
/// drawn at a deterministic offset around its profile and is labeled categorical.
pub fn points(
    runs: &[Run],
    centers: &BTreeMap<String, MapPosition>,
    taxonomy: Option<&Taxonomy>,
) -> Vec<MapPoint> {
    runs.iter()
        .take(MAX_POINTS)
        .filter_map(|run| {
            let center = centers.get(&run.profile_id)?;
            let discovered = taxonomy.and_then(|taxonomy| {
                taxonomy.points.iter().find(|point| {
                    point.team_id == run.team_id
                        && point.api_key_hash == run.api_key_hash
                        && point.trace_id == run.trace_id
                        && point.task_id.as_deref()
                            == run.profile_id.strip_prefix("task:")
                })
            });
            Some(match discovered {
                Some(point) => MapPoint {
                    trace_id: run.trace_id.clone(),
                    profile_id: run.profile_id.clone(),
                    x: point.x,
                    y: point.y,
                    placement: MapPlacement::Discovery,
                },
                None => {
                    let (angle, distance) = offset(run);
                    MapPoint {
                        trace_id: run.trace_id.clone(),
                        profile_id: run.profile_id.clone(),
                        x: center.x + distance * angle.cos(),
                        y: center.y + distance * angle.sin(),
                        placement: MapPlacement::Categorical,
                    }
                }
            })
        })
        .collect()
}

fn offset(run: &Run) -> (f64, f64) {
    let digest = Sha256::digest(
        [
            run.team_id.as_bytes(),
            run.api_key_hash.as_bytes(),
            run.trace_id.as_bytes(),
        ]
        .join(&0),
    );
    let unit = |index: usize| f64::from(u16::from_be_bytes([digest[index], digest[index + 1]])) / 65_535.0;
    (unit(0) * std::f64::consts::TAU, 1.5 + unit(2) * 5.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::{DiscoveryPoint, TaxonomyModels, TaxonomyStatus};
    use rstest::rstest;

    fn run(trace_id: &str, profile_id: &str) -> Run {
        Run {
            team_id: "team".into(),
            api_key_hash: "key".into(),
            trace_id: trace_id.into(),
            start_ms: 0,
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
            status: TaxonomyStatus::Frozen,
            sample: false,
            tasks: Vec::new(),
            points: vec![
                DiscoveryPoint {
                    team_id: "team".into(),
                    api_key_hash: "key".into(),
                    trace_id: "sampled".into(),
                    task_id: Some("a".into()),
                    x: 10.0,
                    y: 20.0,
                },
                DiscoveryPoint {
                    team_id: "team".into(),
                    api_key_hash: "key".into(),
                    trace_id: "other".into(),
                    task_id: Some("a".into()),
                    x: 30.0,
                    y: 40.0,
                },
            ],
            job: None,
            discovered_at: None,
            frozen_at: None,
            error: None,
            models: TaxonomyModels::default(),
        }
    }

    #[rstest]
    fn enhanced_tasks_center_on_their_discovery_sample() {
        let taxonomy = taxonomy();

        let centers = centers(&["task:a", "recorded:Coding"], Some(&taxonomy));

        assert_eq!(centers["task:a"], MapPosition { x: 20.0, y: 30.0 });
    }

    #[rstest]
    fn discovery_samples_keep_their_position_and_later_runs_are_categorical() {
        let taxonomy = taxonomy();
        let runs = [run("sampled", "task:a"), run("later", "task:a")];
        let centers = centers(&["task:a"], Some(&taxonomy));

        let points = points(&runs, &centers, Some(&taxonomy));

        assert_eq!(points[0].placement, MapPlacement::Discovery);
        assert_eq!((points[0].x, points[0].y), (10.0, 20.0));
        assert_eq!(points[1].placement, MapPlacement::Categorical);
        let distance = (points[1].x - 20.0).hypot(points[1].y - 30.0);
        assert!((1.5..=6.5).contains(&distance), "{distance}");
    }

    #[rstest]
    fn categorical_offsets_are_stable_between_reads() {
        let runs = [run("t1", "cluster:a")];
        let centers = centers(&["cluster:a"], None);

        assert_eq!(
            points(&runs, &centers, None),
            points(&runs, &centers, None)
        );
    }
}
