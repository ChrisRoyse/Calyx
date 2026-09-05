use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_mincut::EigenPair;
use calyx_paths::AssocGraph;

use crate::{LodestarError, Result};

pub(super) struct KmeansResult {
    pub(super) assignments: Vec<u8>,
    pub(super) iterations: usize,
    pub(super) inertia: f32,
    /// Number of nodes whose normalized-Laplacian embedding row carried no
    /// signal (all-zero / non-finite-positive L2 norm) and were assigned a
    /// deterministic disconnected-component community instead of participating
    /// in the k-means over the main connected component(s) (#1973).
    pub(super) degenerate_node_count: usize,
}

pub(super) fn deterministic_spectral_clusters(
    graph: &AssocGraph,
    eigenmaps: &[EigenPair],
    cluster_count: usize,
    max_iter: usize,
) -> Result<KmeansResult> {
    let (points, degenerate) = spectral_embedding(eigenmaps, cluster_count, graph.node_count())?;
    deterministic_kmeans(graph, &points, &degenerate, cluster_count, max_iter)
}

/// Build the row-normalized Ng-Jordan-Weiss spectral embedding.
///
/// A node whose row has no finite positive L2 norm carries no spectral signal:
/// this happens for nodes in connected components too small to be captured by
/// the bottom-`k` eigenmaps once #1857's `edge_score_threshold` sparsifies the
/// knn graph into disconnected pieces. Rather than fail closed on the first
/// such row (the #1973 regression), those rows are marked `degenerate` and left
/// untouched here; they receive a deterministic per-component community after
/// the main clustering. Well-embedded rows are normalized exactly as before, so
/// a graph with no degenerate rows produces bit-identical numerics to the v2
/// method (the `degenerate` vector is then all-`false` and every downstream
/// code path reduces to the pre-#1973 arithmetic).
fn spectral_embedding(
    eigenmaps: &[EigenPair],
    dimensions: usize,
    node_count: usize,
) -> Result<(Vec<Vec<f32>>, Vec<bool>)> {
    if eigenmaps.len() < dimensions {
        return invalid_params(format!(
            "eigen_k produced {} eigenvectors but {} communities require at least {}",
            eigenmaps.len(),
            dimensions,
            dimensions
        ));
    }
    let mut rows = vec![Vec::with_capacity(dimensions); node_count];
    for pair in eigenmaps.iter().take(dimensions) {
        if pair.eigenvector.len() != node_count {
            return invalid_params("spectral eigenvector length must match graph node count");
        }
        for (row, value) in rows.iter_mut().zip(&pair.eigenvector) {
            if !value.is_finite() {
                return invalid_params("spectral embedding contains a non-finite coordinate");
            }
            row.push(*value);
        }
    }
    let mut degenerate = vec![false; node_count];
    for (index, row) in rows.iter_mut().enumerate() {
        let norm = squared_norm(row).sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            // Disconnected / isolated node: no spectral signal. Mark degenerate
            // and leave the raw (all-zero) row untouched; it is excluded from
            // the main k-means and assigned a component-local community later.
            degenerate[index] = true;
            continue;
        }
        for value in row {
            *value /= norm;
        }
    }
    Ok((rows, degenerate))
}

fn deterministic_kmeans(
    graph: &AssocGraph,
    points: &[Vec<f32>],
    degenerate: &[bool],
    cluster_count: usize,
    max_iter: usize,
) -> Result<KmeansResult> {
    if points.len() != graph.node_count() || points.is_empty() {
        return invalid_params("spectral point count must match the non-empty graph");
    }
    let dimensions = points[0].len();
    if dimensions == 0 || points.iter().any(|point| point.len() != dimensions) {
        return invalid_params("spectral embedding dimensions are inconsistent");
    }

    // Nodes with a finite positive embedding row, in ascending index order. When
    // no node is degenerate this is `0..node_count`, so every loop below is
    // arithmetically identical to the pre-#1973 method.
    let well: Vec<usize> = (0..points.len())
        .filter(|index| !degenerate[*index])
        .collect();
    if well.len() < cluster_count {
        return invalid_params(format!(
            "spectral clustering has {} well-embedded nodes but {cluster_count} communities were requested; \
             the graph is too fragmented for the requested community_count",
            well.len()
        ));
    }

    let seed_indices = farthest_first_seeds(graph, points, &well, cluster_count)?;
    let mut centroids = seed_indices
        .iter()
        .map(|index| points[*index].clone())
        .collect::<Vec<_>>();
    let mut assignments = vec![u8::MAX; points.len()];

    for iteration in 1..=max_iter {
        let next = assign_points(points, &well, &centroids);
        if next == assignments {
            let inertia = clustering_inertia(points, &well, &assignments, &centroids);
            let mut canonical =
                canonicalize_assignments(graph, &assignments, &well, cluster_count)?;
            let degenerate_node_count =
                assign_degenerate_communities(graph, degenerate, cluster_count, &mut canonical)?;
            return Ok(KmeansResult {
                inertia,
                assignments: canonical,
                iterations: iteration - 1,
                degenerate_node_count,
            });
        }
        assignments = next;
        centroids = recompute_centroids(points, &well, &assignments, cluster_count, dimensions)?;
    }
    invalid_params(format!(
        "deterministic spectral clustering did not converge after {max_iter} iterations"
    ))
}

fn farthest_first_seeds(
    graph: &AssocGraph,
    points: &[Vec<f32>],
    well: &[usize],
    cluster_count: usize,
) -> Result<Vec<usize>> {
    let mut seeds = Vec::with_capacity(cluster_count);
    let first = well
        .iter()
        .copied()
        .max_by(|left, right| {
            squared_norm(&points[*left])
                .total_cmp(&squared_norm(&points[*right]))
                .then_with(|| node_id_cmp(graph, *right, *left))
        })
        .ok_or_else(|| LodestarError::KernelInvalidParams {
            detail: "spectral embedding contains no seed candidates".to_string(),
        })?;
    seeds.push(first);
    while seeds.len() < cluster_count {
        let next = well
            .iter()
            .copied()
            .filter(|index| !seeds.contains(index))
            .max_by(|left, right| {
                min_seed_distance(&points[*left], points, &seeds)
                    .total_cmp(&min_seed_distance(&points[*right], points, &seeds))
                    .then_with(|| node_id_cmp(graph, *right, *left))
            })
            .ok_or_else(|| LodestarError::KernelInvalidParams {
                detail: format!(
                    "community_count {cluster_count} exceeds distinct graph seed candidates"
                ),
            })?;
        seeds.push(next);
    }
    Ok(seeds)
}

fn node_id_cmp(graph: &AssocGraph, left: usize, right: usize) -> std::cmp::Ordering {
    graph
        .node_id(left)
        .expect("spectral node id")
        .as_bytes()
        .cmp(graph.node_id(right).expect("spectral node id").as_bytes())
}

fn min_seed_distance(point: &[f32], points: &[Vec<f32>], seeds: &[usize]) -> f32 {
    seeds
        .iter()
        .map(|index| squared_distance(point, &points[*index]))
        .fold(f32::INFINITY, f32::min)
}

fn assign_points(points: &[Vec<f32>], well: &[usize], centroids: &[Vec<f32>]) -> Vec<u8> {
    let mut assignments = vec![u8::MAX; points.len()];
    for &index in well {
        assignments[index] = centroids
            .iter()
            .enumerate()
            .min_by(|(left_index, left), (right_index, right)| {
                squared_distance(&points[index], left)
                    .total_cmp(&squared_distance(&points[index], right))
                    .then_with(|| left_index.cmp(right_index))
            })
            .map(|(centroid_index, _)| centroid_index as u8)
            .expect("spectral clustering has at least two centroids");
    }
    assignments
}

fn recompute_centroids(
    points: &[Vec<f32>],
    well: &[usize],
    assignments: &[u8],
    cluster_count: usize,
    dimensions: usize,
) -> Result<Vec<Vec<f32>>> {
    let mut sums = vec![vec![0.0_f32; dimensions]; cluster_count];
    let mut counts = vec![0_usize; cluster_count];
    for &index in well {
        let cluster = usize::from(assignments[index]);
        counts[cluster] += 1;
        for (sum, value) in sums[cluster].iter_mut().zip(&points[index]) {
            *sum += *value;
        }
    }
    if let Some(empty) = counts.iter().position(|count| *count == 0) {
        return invalid_params(format!(
            "deterministic spectral clustering produced empty community {empty}"
        ));
    }
    for (centroid, count) in sums.iter_mut().zip(counts) {
        for value in centroid {
            *value /= count as f32;
        }
    }
    Ok(sums)
}

fn canonicalize_assignments(
    graph: &AssocGraph,
    assignments: &[u8],
    well: &[usize],
    cluster_count: usize,
) -> Result<Vec<u8>> {
    let mut minima = vec![None::<CxId>; cluster_count];
    for &index in well {
        let id = graph.node_id(index).ok_or_else(|| LodestarError::Graph {
            code: "CALYX_GRAPH_UNKNOWN_NODE",
            message: format!("graph node index {index} is missing"),
        })?;
        let slot = &mut minima[usize::from(assignments[index])];
        if slot
            .as_ref()
            .is_none_or(|current| id.as_bytes() < current.as_bytes())
        {
            *slot = Some(id);
        }
    }
    if minima.iter().any(Option::is_none) {
        return invalid_params("spectral community canonicalization found an empty community");
    }
    let mut order = (0..cluster_count).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        minima[*left]
            .expect("community minimum")
            .as_bytes()
            .cmp(minima[*right].expect("community minimum").as_bytes())
    });
    let mut remap = vec![0_u8; cluster_count];
    for (canonical, original) in order.into_iter().enumerate() {
        remap[original] = canonical as u8;
    }
    // Well nodes are remapped into canonical community ids; degenerate nodes keep
    // their `u8::MAX` sentinel here and are filled in by
    // `assign_degenerate_communities`.
    let mut canonical = assignments.to_vec();
    for &index in well {
        canonical[index] = remap[usize::from(assignments[index])];
    }
    Ok(canonical)
}

/// Assign every degenerate (disconnected) node a deterministic community.
///
/// Standard practice for spectral clustering after edge sparsification is to
/// take each connected component as its own community (e.g. the divisive
/// spectral method of Cheng & Zhao, arXiv:1506.08354: "After sparsification,
/// the network itself might become disconnected. We take each connected
/// component as a community."). We follow that: degenerate nodes are grouped by
/// the connected components they form *among themselves* (union-find over edges
/// whose endpoints are both degenerate), so isolated opinions that still
/// associate with one another share one honest community, while a truly
/// isolated opinion becomes its own singleton. New communities are numbered
/// `cluster_count, cluster_count+1, ...` in ascending order of each component's
/// minimum `cx_id`, so the labeling is a pure function of the input graph.
///
/// Returns the number of degenerate nodes assigned (0 leaves `assignments`
/// untouched, keeping the no-degenerate path bit-identical to v2).
fn assign_degenerate_communities(
    graph: &AssocGraph,
    degenerate: &[bool],
    cluster_count: usize,
    assignments: &mut [u8],
) -> Result<usize> {
    let node_count = degenerate.len();
    let degenerate_indices: Vec<usize> =
        (0..node_count).filter(|index| degenerate[*index]).collect();
    let degenerate_node_count = degenerate_indices.len();
    if degenerate_node_count == 0 {
        return Ok(0);
    }

    // Union-find over the degenerate sub-graph. Union-by-min-index keeps the
    // final partition independent of edge iteration order.
    let mut parent: Vec<usize> = (0..node_count).collect();
    for edge in graph.edges() {
        if degenerate[edge.src] && degenerate[edge.dst] {
            let left = uf_find(&mut parent, edge.src);
            let right = uf_find(&mut parent, edge.dst);
            if left != right {
                parent[left.max(right)] = left.min(right);
            }
        }
    }

    // Group degenerate nodes by component root, recording each component's
    // minimum cx_id (the stable key we order communities by).
    let mut root_min_id: BTreeMap<usize, CxId> = BTreeMap::new();
    for &index in &degenerate_indices {
        let root = uf_find(&mut parent, index);
        let id = graph.node_id(index).ok_or_else(|| LodestarError::Graph {
            code: "CALYX_GRAPH_UNKNOWN_NODE",
            message: format!("graph node index {index} is missing"),
        })?;
        root_min_id
            .entry(root)
            .and_modify(|current| {
                if id.as_bytes() < current.as_bytes() {
                    *current = id;
                }
            })
            .or_insert(id);
    }

    let total_communities = cluster_count + root_min_id.len();
    if total_communities > usize::from(u8::MAX) + 1 {
        return invalid_params(format!(
            "spectral clustering would require {total_communities} communities \
             ({cluster_count} spectral + {} disconnected components), exceeding the {} community-id limit",
            root_min_id.len(),
            usize::from(u8::MAX) + 1
        ));
    }

    // Order components by minimum cx_id and assign community ids
    // `cluster_count, cluster_count + 1, ...` in that order.
    let mut roots_by_min_id: Vec<(usize, CxId)> = root_min_id.into_iter().collect();
    roots_by_min_id.sort_by(|left, right| left.1.as_bytes().cmp(right.1.as_bytes()));
    let community_of_root: BTreeMap<usize, u8> = roots_by_min_id
        .into_iter()
        .enumerate()
        .map(|(offset, (root, _))| (root, (cluster_count + offset) as u8))
        .collect();

    for &index in &degenerate_indices {
        let root = uf_find(&mut parent, index);
        assignments[index] = community_of_root[&root];
    }

    Ok(degenerate_node_count)
}

fn uf_find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn clustering_inertia(
    points: &[Vec<f32>],
    well: &[usize],
    assignments: &[u8],
    centroids: &[Vec<f32>],
) -> f32 {
    well.iter()
        .map(|&index| squared_distance(&points[index], &centroids[usize::from(assignments[index])]))
        .sum()
}

fn squared_norm(point: &[f32]) -> f32 {
    point.iter().map(|value| value * value).sum()
}

fn squared_distance(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| {
            let delta = left - right;
            delta * delta
        })
        .sum()
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
