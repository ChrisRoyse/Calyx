use std::path::Path;

use super::operational_h13_14::{probe_h13_hot_shard, probe_h14_lock_contention};
use super::operational_h15_16::{probe_h15_cache_stampede, probe_h16_slow_lens_hol};
use super::operational_h17_19::{
    probe_h17_disk_pressure, probe_h18_arc_read_thrash, probe_h19_clock_skew,
};
use super::operational_h20_21::{probe_h20_anneal_thrashing, probe_h21_panel_explosion};
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

pub fn run_hazards_17_21(root: &Path) -> Vec<HazardResult> {
    [
        (
            17,
            "disk pressure fail-closed",
            probe_h17_disk_pressure as fn(&Path) -> ProbeResult,
        ),
        (
            18,
            "ARC/read-thrash graceful degradation",
            probe_h18_arc_read_thrash,
        ),
        (19, "clock skew monotonic sequence", probe_h19_clock_skew),
        (20, "Anneal thrash hysteresis", probe_h20_anneal_thrashing),
        (
            21,
            "panel-version / cross-term explosion",
            probe_h21_panel_explosion,
        ),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}
