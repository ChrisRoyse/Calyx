use rayon::prelude::*;

use crate::index::distance::l2_sq;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskAnnBuildMetric {
    UnitL2,
    RawL2,
}

/// L2-normalize every vector; a zero vector stays all-zero (dot == 0 with
/// anything, i.e. distance 1, matching cosine's zero-vector convention).
pub(in crate::index::diskann) fn normalize(vectors: &[(u32, Vec<f32>)]) -> Vec<Vec<f32>> {
    vectors
        .par_iter()
        .map(|(_, v)| {
            let mag = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if mag == 0.0 {
                v.clone()
            } else {
                v.iter().map(|x| x / mag).collect()
            }
        })
        .collect()
}

pub(super) fn build_space(
    vectors: &[(u32, Vec<f32>)],
    metric: DiskAnnBuildMetric,
) -> Vec<Vec<f32>> {
    match metric {
        DiskAnnBuildMetric::UnitL2 => normalize(vectors),
        DiskAnnBuildMetric::RawL2 => vectors.iter().map(|(_, vector)| vector.clone()).collect(),
    }
}

pub(super) fn dist(a: &[f32], b: &[f32], metric: DiskAnnBuildMetric) -> f32 {
    match metric {
        DiskAnnBuildMetric::UnitL2 => 0.5 * l2_sq(a, b),
        DiskAnnBuildMetric::RawL2 => l2_sq(a, b),
    }
}
