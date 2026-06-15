use std::path::Path;

use super::operational_h13_14::{probe_h13_hot_shard, probe_h14_lock_contention};
use super::operational_h15_16::{probe_h15_cache_stampede, probe_h16_slow_lens_hol};
use super::resource::{HazardResult, ProbeResult, run_probe};

pub fn run_hazards_13_16(root: &Path) -> Vec<HazardResult> {
    [
        (
            13,
            "hot-shard tenant skew",
            probe_h13_hot_shard as fn(&Path) -> ProbeResult,
        ),
        (14, "lock contention / deadlock", probe_h14_lock_contention),
        (15, "cache stampede single-flight", probe_h15_cache_stampede),
        (16, "slow-lens head-of-line", probe_h16_slow_lens_hol),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}
