//! Partitioned-vault manifest types and closure-assignment telemetry (#1129).

use serde::{Deserialize, Serialize};

use super::{DiskAnnBuildBackend, PartitionDistanceMetric};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMeta {
    pub id: u32,
    pub count: usize,
    pub graph_rel: String,
    pub ids_rel: String,
}

/// Build-time closure-assignment telemetry (#1129). SPTAG logs the equivalent
/// "RNG failed count" and a replica-count histogram at build time; without
/// these counters a `max_replication` request that the RNG rule prunes to
/// nothing is indistinguishable from working boundary replication.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClosureAssignmentStats {
    /// Rows routed through bounded closure assignment (equals n_cx).
    pub rows: u64,
    /// Replica copies stored beyond each row's primary region.
    pub replicas_stored: u64,
    /// Replica candidates rejected by the (1 + epsilon) closure threshold.
    pub epsilon_filtered: u64,
    /// Replica candidates rejected by the RNG rule.
    pub rng_skipped: u64,
    /// Replica candidates rejected by the region cap or per-region duplicate cap.
    pub cap_skipped: u64,
    /// Rows whose replication stopped early on the global duplicate budget.
    pub budget_stopped_rows: u64,
    /// `replica_histogram[i]` = rows stored in exactly `i + 1` regions.
    pub replica_histogram: Vec<u64>,
}

impl ClosureAssignmentStats {
    /// Stored copies per row (1.0 = no replication happened).
    pub fn replication_factor(&self) -> f64 {
        if self.rows == 0 {
            return 1.0;
        }
        1.0 + self.replicas_stored as f64 / self.rows as f64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionedManifest {
    pub format: String,
    pub n_cx: u64,
    pub dim: usize,
    pub n_regions: usize,
    pub seed: u64,
    pub m_max: usize,
    pub ef_construction: usize,
    #[serde(default)]
    pub distance_metric: PartitionDistanceMetric,
    #[serde(default)]
    pub region_build_parallelism: usize,
    #[serde(default = "default_graph_build_backend")]
    pub graph_build_backend: DiskAnnBuildBackend,
    #[serde(default)]
    pub provisional_assignment_routing: String,
    #[serde(default)]
    pub final_assignment_routing: String,
    #[serde(default)]
    pub final_assignment_probe: usize,
    #[serde(default)]
    pub final_assignment_cap: Option<usize>,
    #[serde(default)]
    pub final_assignment_boundary_epsilon: f32,
    #[serde(default)]
    pub final_assignment_max_replication: usize,
    #[serde(default)]
    pub final_assignment_rng_rule: bool,
    /// SPTAG `RNGFactor` parity: relaxes the RNG rule on the squared-distance
    /// scale. Manifests written before #1129 default to the strict paper rule.
    #[serde(default = "default_rng_factor")]
    pub final_assignment_rng_factor: f32,
    /// Closure telemetry; `None` for vaults built before #1129.
    #[serde(default)]
    pub final_assignment_closure: Option<ClosureAssignmentStats>,
    #[serde(default)]
    pub region_balance_cap: usize,
    #[serde(default)]
    pub stored_region_members: usize,
    pub centroids_rel: String,
    pub root_graph_rel: String,
    pub regions: Vec<RegionMeta>,
}

fn default_graph_build_backend() -> DiskAnnBuildBackend {
    DiskAnnBuildBackend::CpuVamana
}

pub(super) fn default_rng_factor() -> f32 {
    1.0
}
