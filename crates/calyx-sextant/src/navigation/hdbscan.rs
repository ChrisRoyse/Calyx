//! Deterministic HDBSCAN* condensed-tree clustering (skill discovery core).
//!
//! Implements the HDBSCAN* pipeline: core distances → mutual reachability →
//! Prim MST → single-linkage dendrogram → condensed tree (`min_cluster_size`)
//! → excess-of-mass stability selection. Every comparison uses `total_cmp`
//! plus index tie-breaks, so identical inputs produce identical trees —
//! reference implementations leave MST/stability ties unspecified; here they
//! are total-ordered on purpose (issue #600 requires seeded determinism).

use calyx_core::Result;

use crate::error::{CALYX_SEXTANT_SKILL_PARAMS, sextant_error};

/// Guards `1/distance` when two points coincide (distance 0).
const MIN_SPLIT_DISTANCE: f64 = 1e-12;

/// A node of the condensed cluster tree (cluster 0 is the root).
#[derive(Clone, Debug)]
pub(crate) struct CondensedCluster {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// λ = 1/distance at which this cluster appeared.
    pub birth_lambda: f64,
    /// λ at which this cluster split or fully evaporated (∞ if never).
    pub death_lambda: f64,
    /// Point indexes that belonged to this cluster when it was born.
    pub members_at_birth: Vec<usize>,
    /// Σ over points of (λ_exit − λ_birth): the excess-of-mass stability.
    pub stability: f64,
    /// Flat-clustering selection flag (excess-of-mass rule).
    pub selected: bool,
}

/// Symmetric distance matrix stored as the strict upper triangle.
pub(crate) struct DistanceMatrix {
    n: usize,
    upper: Vec<f64>,
}

impl DistanceMatrix {
    pub(crate) fn new(n: usize, upper: Vec<f64>) -> Result<Self> {
        let expected = n * n.saturating_sub(1) / 2;
        if upper.len() != expected {
            return Err(sextant_error(
                CALYX_SEXTANT_SKILL_PARAMS,
                format!(
                    "distance triangle has {} entries, expected {expected} for n={n}",
                    upper.len()
                ),
            ));
        }
        Ok(Self { n, upper })
    }

    pub(crate) fn get(&self, i: usize, j: usize) -> f64 {
        debug_assert!(i != j && i < self.n && j < self.n);
        let (lo, hi) = if i < j { (i, j) } else { (j, i) };
        self.upper[lo * self.n - lo * (lo + 1) / 2 + (hi - lo - 1)]
    }
}

/// Runs deterministic HDBSCAN* and returns the condensed cluster tree.
///
/// `n == 0` returns an empty tree; `n == 1` returns a root holding the single
/// point with no sub-structure.
pub(crate) fn condensed_tree(
    dist: &DistanceMatrix,
    min_samples: usize,
    min_cluster_size: usize,
    allow_single_cluster: bool,
) -> Result<Vec<CondensedCluster>> {
    if min_cluster_size < 2 || min_samples < 1 {
        return Err(sextant_error(
            CALYX_SEXTANT_SKILL_PARAMS,
            format!(
                "min_cluster_size {min_cluster_size} must be >= 2 and min_samples {min_samples} >= 1"
            ),
        ));
    }
    let n = dist.n;
    if n == 0 {
        return Ok(Vec::new());
    }
    let root = CondensedCluster {
        parent: None,
        children: Vec::new(),
        birth_lambda: 0.0,
        death_lambda: f64::INFINITY,
        members_at_birth: (0..n).collect(),
        stability: 0.0,
        selected: false,
    };
    if n == 1 {
        return Ok(vec![root]);
    }

    let core = core_distances(dist, min_samples);
    let mst = prim_mst(dist, &core);
    let dendrogram = single_linkage(n, &mst);
    let mut clusters = vec![root];
    condense(&dendrogram, n, min_cluster_size, &mut clusters);
    select_clusters(&mut clusters, allow_single_cluster);
    Ok(clusters)
}

/// Distance to the k-th nearest other point, k = min(min_samples, n-1).
fn core_distances(dist: &DistanceMatrix, min_samples: usize) -> Vec<f64> {
    let n = dist.n;
    let k = min_samples.min(n - 1);
    (0..n)
        .map(|i| {
            let mut row: Vec<f64> = (0..n).filter(|j| *j != i).map(|j| dist.get(i, j)).collect();
            row.sort_by(f64::total_cmp);
            row[k - 1]
        })
        .collect()
}

fn mutual_reachability(dist: &DistanceMatrix, core: &[f64], i: usize, j: usize) -> f64 {
    dist.get(i, j).max(core[i]).max(core[j])
}

/// Prim MST over the mutual-reachability graph; ties broken by node index.
fn prim_mst(dist: &DistanceMatrix, core: &[f64]) -> Vec<(usize, usize, f64)> {
    let n = dist.n;
    let mut in_tree = vec![false; n];
    let mut key = vec![f64::INFINITY; n];
    let mut parent = vec![usize::MAX; n];
    key[0] = 0.0;
    let mut edges = Vec::with_capacity(n - 1);
    for _ in 0..n {
        let u = (0..n)
            .filter(|i| !in_tree[*i])
            .min_by(|a, b| key[*a].total_cmp(&key[*b]).then_with(|| a.cmp(b)))
            .expect("prim always has a next node");
        in_tree[u] = true;
        if parent[u] != usize::MAX {
            edges.push((parent[u].min(u), parent[u].max(u), key[u]));
        }
        for v in 0..n {
            if !in_tree[v] {
                let w = mutual_reachability(dist, core, u, v);
                if w < key[v] {
                    key[v] = w;
                    parent[v] = u;
                }
            }
        }
    }
    edges.sort_by(|a, b| {
        a.2.total_cmp(&b.2)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    edges
}

/// A binary dendrogram node; leaves are 0..n, internals are n..2n-1.
struct DendroNode {
    left: usize,
    right: usize,
    distance: f64,
    size: usize,
}

fn single_linkage(n: usize, sorted_edges: &[(usize, usize, f64)]) -> Vec<DendroNode> {
    let mut representative: Vec<usize> = (0..2 * n - 1).collect();
    let mut nodes = Vec::with_capacity(n - 1);
    for (a, b, distance) in sorted_edges {
        let left = find(&mut representative, *a);
        let right = find(&mut representative, *b);
        let id = n + nodes.len();
        let size = dendro_size(&nodes, n, left) + dendro_size(&nodes, n, right);
        nodes.push(DendroNode {
            left,
            right,
            distance: *distance,
            size,
        });
        representative[left] = id;
        representative[right] = id;
        representative[id] = id;
    }
    nodes
}

fn find(representative: &mut [usize], mut node: usize) -> usize {
    while representative[node] != node {
        representative[node] = representative[representative[node]];
        node = representative[node];
    }
    node
}

fn dendro_size(nodes: &[DendroNode], n: usize, id: usize) -> usize {
    if id < n { 1 } else { nodes[id - n].size }
}

fn leaves_under(nodes: &[DendroNode], n: usize, id: usize) -> Vec<usize> {
    let mut leaves = Vec::new();
    let mut stack = vec![id];
    while let Some(node) = stack.pop() {
        if node < n {
            leaves.push(node);
        } else {
            stack.push(nodes[node - n].left);
            stack.push(nodes[node - n].right);
        }
    }
    leaves.sort_unstable();
    leaves
}

/// Walks the dendrogram top-down, condensing it per `min_cluster_size`.
fn condense(
    dendrogram: &[DendroNode],
    n: usize,
    min_cluster_size: usize,
    clusters: &mut Vec<CondensedCluster>,
) {
    let root_id = n + dendrogram.len() - 1;
    let mut stack = vec![(root_id, 0usize)];
    while let Some((node_id, cluster_id)) = stack.pop() {
        if node_id < n {
            // A bare leaf inside a live cluster: it exits when distance hits 0.
            let lambda = 1.0 / MIN_SPLIT_DISTANCE;
            record_exits(clusters, cluster_id, &[node_id], lambda);
            continue;
        }
        let node = &dendrogram[node_id - n];
        let lambda = 1.0 / node.distance.max(MIN_SPLIT_DISTANCE);
        let left_size = dendro_size(dendrogram, n, node.left);
        let right_size = dendro_size(dendrogram, n, node.right);
        let left_big = left_size >= min_cluster_size;
        let right_big = right_size >= min_cluster_size;
        if left_big && right_big {
            // True split: the parent dies here, two children are born.
            let exiting = leaves_under(dendrogram, n, node_id);
            record_exits(clusters, cluster_id, &exiting, lambda);
            clusters[cluster_id].death_lambda = lambda;
            for child_root in [node.left, node.right] {
                let child_id = clusters.len();
                clusters.push(CondensedCluster {
                    parent: Some(cluster_id),
                    children: Vec::new(),
                    birth_lambda: lambda,
                    death_lambda: f64::INFINITY,
                    members_at_birth: leaves_under(dendrogram, n, child_root),
                    stability: 0.0,
                    selected: false,
                });
                clusters[cluster_id].children.push(child_id);
                stack.push((child_root, child_id));
            }
        } else if left_big || right_big {
            // Points fall out of the cluster; the cluster itself survives.
            let (keep, fall) = if left_big {
                (node.left, node.right)
            } else {
                (node.right, node.left)
            };
            let fallen = leaves_under(dendrogram, n, fall);
            record_exits(clusters, cluster_id, &fallen, lambda);
            stack.push((keep, cluster_id));
        } else {
            // The whole remainder evaporates: cluster ends here.
            let fallen = leaves_under(dendrogram, n, node_id);
            record_exits(clusters, cluster_id, &fallen, lambda);
            clusters[cluster_id].death_lambda = lambda;
        }
    }
}

fn record_exits(
    clusters: &mut [CondensedCluster],
    cluster_id: usize,
    points: &[usize],
    lambda: f64,
) {
    let cluster = &mut clusters[cluster_id];
    cluster.stability += points.len() as f64 * (lambda - cluster.birth_lambda);
}

/// Excess-of-mass selection: pick the most stable antichain of clusters.
fn select_clusters(clusters: &mut [CondensedCluster], allow_single_cluster: bool) {
    let mut subtree_stability = vec![0.0_f64; clusters.len()];
    for id in (0..clusters.len()).rev() {
        let children = clusters[id].children.clone();
        let is_root = clusters[id].parent.is_none();
        if children.is_empty() {
            clusters[id].selected = !is_root || allow_single_cluster;
            subtree_stability[id] = clusters[id].stability;
            continue;
        }
        let child_sum: f64 = children.iter().map(|c| subtree_stability[*c]).sum();
        if (is_root && !allow_single_cluster) || child_sum > clusters[id].stability {
            clusters[id].selected = false;
            subtree_stability[id] = child_sum;
        } else {
            clusters[id].selected = true;
            subtree_stability[id] = clusters[id].stability;
            deselect_descendants(clusters, id);
        }
    }
}

fn deselect_descendants(clusters: &mut [CondensedCluster], id: usize) {
    let mut stack = clusters[id].children.clone();
    while let Some(child) = stack.pop() {
        clusters[child].selected = false;
        stack.extend(clusters[child].children.clone());
    }
}
