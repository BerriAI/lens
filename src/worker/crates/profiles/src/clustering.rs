use std::panic::{AssertUnwindSafe, catch_unwind};

use hdbscan::{Hdbscan, HdbscanHyperParams};
use ndarray::Array2;
use rand::{Rng, SeedableRng, rngs::StdRng};
use umap_rs::{
    EuclideanMetric, Optimizer, Umap, UmapConfig,
    config::{GraphParams, ManifoldParams},
    metric::MetricType,
};

use crate::Error;

pub const MIN_POINTS: usize = 20;
pub const MAX_POINTS: usize = 500;
const NEIGHBORS: usize = 15;
const CLUSTER_DIMENSIONS: usize = 10;
const SEED: u64 = 42;
const EPOCHS: usize = 500;
const MAP_SPAN: f64 = 100.0;

/// Cluster labels (`None` for noise) and a 0-100 display position per input.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub labels: Vec<Option<usize>>,
    pub points: Vec<(f64, f64)>,
}

fn normalized(vectors: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, Error> {
    let width = vectors.first().map_or(0, Vec::len);
    vectors
        .iter()
        .map(|vector| {
            let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
            if vector.len() != width
                || width == 0
                || !norm.is_finite()
                || norm == 0.0
                || vector.iter().any(|value| !value.is_finite())
            {
                return Err(Error::InvalidEmbedding);
            }
            Ok(vector.iter().map(|value| value / norm).collect())
        })
        .collect()
}

/// Exact cosine neighbors, each row starting with the point itself as UMAP expects.
fn neighbors(vectors: &[Vec<f32>], count: usize) -> (Array2<u32>, Array2<f32>) {
    let mut indices = Array2::<u32>::zeros((vectors.len(), count));
    let mut distances = Array2::<f32>::zeros((vectors.len(), count));
    for (row, vector) in vectors.iter().enumerate() {
        let mut ranked: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(column, other)| {
                let similarity: f32 = vector.iter().zip(other).map(|(a, b)| a * b).sum();
                (column, if column == row { 0.0 } else { (1.0 - similarity).max(0.0) })
            })
            .collect();
        ranked.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| (left.0 != row).cmp(&(right.0 != row)))
                .then_with(|| left.0.cmp(&right.0))
        });
        for (slot, (column, distance)) in ranked.into_iter().take(count).enumerate() {
            indices[[row, slot]] = u32::try_from(column).unwrap_or(u32::MAX);
            distances[[row, slot]] = distance;
        }
    }
    (indices, distances)
}

fn reduce(
    vectors: &[Vec<f32>],
    indices: &Array2<u32>,
    distances: &Array2<f32>,
    components: usize,
    min_dist: f32,
) -> Result<Array2<f32>, Error> {
    let rows = vectors.len();
    let width = vectors[0].len();
    let data = Array2::from_shape_vec((rows, width), vectors.concat())
        .map_err(|_| Error::InvalidEmbedding)?;
    let mut random = StdRng::seed_from_u64(SEED);
    let init = Array2::from_shape_fn((rows, components), |_| random.gen_range(-10.0f32..10.0));
    let config = UmapConfig {
        n_components: components,
        graph: GraphParams {
            n_neighbors: indices.ncols(),
            ..GraphParams::default()
        },
        manifold: ManifoldParams {
            min_dist,
            ..ManifoldParams::default()
        },
        ..UmapConfig::default()
    };
    // Optimizer::step_epochs reads neighbour positions from a snapshot taken when the call starts,
    // so one epoch per call keeps attraction pointed at where the neighbours are now.
    let embedding = catch_unwind(AssertUnwindSafe(|| {
        let manifold = Umap::new(config.clone()).learn_manifold(data.view(), indices.view(), distances.view());
        let mut optimizer = Optimizer::new(manifold, init, EPOCHS, &config, MetricType::Euclidean);
        while optimizer.remaining_epochs() > 0 {
            optimizer.step_epochs(1, &EuclideanMetric);
        }
        optimizer.embedding().to_owned()
    }))
    .map_err(|_| Error::Clustering)?;
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err(Error::Clustering);
    }
    Ok(embedding)
}

fn scaled(display: &Array2<f32>) -> Vec<(f64, f64)> {
    let axis = |column: usize| {
        let values: Vec<f64> = display.column(column).iter().map(|value| f64::from(*value)).collect();
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let span = if high > low { high - low } else { 1.0 };
        values
            .into_iter()
            .map(|value| (value - low) / span * MAP_SPAN * 0.9 + 5.0)
            .collect::<Vec<_>>()
    };
    axis(0).into_iter().zip(axis(1)).collect()
}

/// UMAP reduction tuned for density, then HDBSCAN, plus a separate 2D UMAP for display, as the
/// UMAP clustering guide advises (https://umap-learn.readthedocs.io/en/latest/clustering.html).
/// Coordinates come from one stochastic optimization and are stored, never recomputed.
pub fn cluster(vectors: &[Vec<f32>]) -> Result<Layout, Error> {
    if !(MIN_POINTS..=MAX_POINTS).contains(&vectors.len()) {
        return Err(Error::InvalidClusters);
    }
    let vectors = normalized(vectors)?;
    let count = NEIGHBORS.min(vectors.len() - 1);
    let (indices, distances) = neighbors(&vectors, count);
    let width = vectors[0].len();
    let dimensions = CLUSTER_DIMENSIONS.min(vectors.len() - 2).min(width).max(2);
    let reduced = reduce(&vectors, &indices, &distances, dimensions, 0.0)?;
    let display = reduce(&vectors, &indices, &distances, 2, 0.3)?;
    let rows: Vec<Vec<f32>> = reduced.rows().into_iter().map(|row| row.to_vec()).collect();
    let min_cluster_size = (vectors.len() / 25).max(5);
    let labels = Hdbscan::new(
        &rows,
        HdbscanHyperParams::builder()
            .min_cluster_size(min_cluster_size)
            .build(),
    )
    .cluster()
    .map_err(|_| Error::Clustering)?;
    Ok(Layout {
        labels: labels
            .into_iter()
            .map(|label| usize::try_from(label).ok())
            .collect(),
        points: scaled(&display),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn group(axis: usize, count: usize) -> Vec<Vec<f32>> {
        (0..count)
            .map(|index| {
                let mut vector = vec![0.02 * (index % 7) as f32; 8];
                vector[axis] = 1.0;
                vector
            })
            .collect()
    }

    #[rstest]
    fn well_separated_groups_land_in_separate_clusters() {
        let vectors: Vec<Vec<f32>> = [group(0, 15), group(4, 15)].concat();

        let layout = cluster(&vectors).expect("layout");
        let first = layout.labels[0];
        let second = layout.labels[15];

        assert!(first.is_some() && second.is_some());
        assert_ne!(first, second);
        assert!(layout.labels[..15].iter().all(|label| *label == first));
        assert!(layout.labels[15..].iter().all(|label| *label == second));
        assert!(layout
            .points
            .iter()
            .all(|(x, y)| (5.0..=95.0).contains(x) && (5.0..=95.0).contains(y)));
    }

    #[rstest]
    #[case::too_few(vec![vec![1.0, 0.0]; 19])]
    #[case::ragged([vec![vec![1.0, 0.0]; 19], vec![vec![1.0]]].concat())]
    #[case::zero([vec![vec![1.0, 0.0]; 19], vec![vec![0.0, 0.0]]].concat())]
    #[case::not_finite([vec![vec![1.0, 0.0]; 19], vec![vec![f32::NAN, 0.0]]].concat())]
    fn invalid_inputs_are_rejected_before_umap_can_panic(#[case] vectors: Vec<Vec<f32>>) {
        assert!(cluster(&vectors).is_err());
    }

    #[rstest]
    fn each_point_is_its_own_first_neighbor() {
        let vectors = normalized(&group(0, 4)).expect("normalized");

        let (indices, distances) = neighbors(&vectors, 3);

        assert!((0..4).all(|row| indices[[row, 0]] as usize == row && distances[[row, 0]] == 0.0));
    }
}
