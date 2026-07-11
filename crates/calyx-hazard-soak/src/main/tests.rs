use super::*;
use calyx_hazard_soak::soak::SoakCounts;

fn passing_soak_report() -> SoakReport {
    SoakReport {
        op_count: 1,
        seed: 1,
        sample_every: 1,
        key_space: 1,
        ann_index_cap: 1,
        wal_segment_bytes: 1,
        wal_records_flushed: 1,
        counts: SoakCounts::default(),
        trend_bytes_per_op: 0.0,
        vram_trend_bytes_per_op: 0.0,
        rss_max_mib: 1,
        vram_max_mib: 1,
        soft_cap_mib: 1,
        rss_bounded: true,
        vram_bounded: true,
        oldest_pinned_seq_gap_bounded: true,
        compaction_gc_exercised: true,
        soak_oscillation_detected: false,
        max_gap_seqs: 1,
        final_tombstone_ratio: 0.0,
        wal_bytes_active_final: 1,
        samples: Vec::new(),
        target_files: Vec::new(),
        elapsed_ms: 1,
        panic_free: true,
    }
}

#[test]
fn stage_gate_rejects_unavailable_oom_evidence() {
    let soak = passing_soak_report();

    assert!(!stage_passed(true, Some(&soak), None));
}

#[test]
fn stage_gate_rejects_observed_oom_event() {
    let soak = passing_soak_report();
    let oom = OomEvidence {
        source: "test".to_string(),
        count: 1,
    };

    assert!(!stage_passed(true, Some(&soak), Some(&oom)));
}

#[test]
fn stage_gate_requires_measured_pin_and_compaction_evidence() {
    let mut soak = passing_soak_report();
    let oom = OomEvidence {
        source: "test".to_string(),
        count: 0,
    };
    soak.max_gap_seqs = 0;
    assert!(!stage_passed(true, Some(&soak), Some(&oom)));

    soak.max_gap_seqs = 1;
    soak.compaction_gc_exercised = false;
    assert!(!stage_passed(true, Some(&soak), Some(&oom)));
}
