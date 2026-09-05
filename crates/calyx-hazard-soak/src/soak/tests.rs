use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        Self(env::temp_dir().join(format!(
            "calyx-issue1302-{label}-{}-{id}",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn soak_sample(op: u64, tombstone_ratio: f64, oldest_pinned_seq_gap: u64) -> SoakSample {
    SoakSample {
        op,
        rss_kib: 100_000,
        vram_mib: 0,
        tombstone_ratio,
        wal_bytes_active: 0,
        oldest_pinned_seq_gap,
    }
}

#[test]
fn integrated_soak_pinned_gap_matches_live_commit_distance() {
    let root = TestRoot::new("pin-gap");
    let report = run_integrated_soak_at(root.path(), SAMPLE_EVERY, DEFAULT_SOAK_SEED)
        .expect("run one measured sample window");

    assert!(report.counts.writes > 0, "seed must exercise writes");
    assert_eq!(
        report.max_gap_seqs, report.counts.writes,
        "one durable commit per write must define the measured reader-pin distance"
    );
}

#[test]
fn bounded_tombstone_jitter_is_not_oscillation() {
    let ratios = [
        0.0, 0.1939, 0.1988, 0.1980, 0.1966, 0.1974, 0.1970, 0.1977, 0.1969, 0.1972, 0.2001, 0.2000,
    ];
    let samples: Vec<_> = ratios
        .into_iter()
        .enumerate()
        .map(|(idx, ratio)| soak_sample(idx as u64 * SAMPLE_EVERY, ratio, 10_000 - idx as u64))
        .collect();

    assert!(!oscillates(&samples));
}

#[test]
fn large_tombstone_sawtooth_is_oscillation() {
    let ratios = [
        0.20, 0.35, 0.12, 0.34, 0.11, 0.33, 0.10, 0.32, 0.09, 0.31, 0.08, 0.30, 0.07, 0.29, 0.06,
    ];
    let samples: Vec<_> = ratios
        .into_iter()
        .enumerate()
        .map(|(idx, ratio)| soak_sample(idx as u64 * SAMPLE_EVERY, ratio, 0))
        .collect();

    assert!(oscillates(&samples));
}

#[test]
fn monotone_pinned_gap_cleanup_is_not_oscillation() {
    let gaps = [10_000, 9_999, 9_998, 9_997, 9_996, 8_000, 0];
    let samples: Vec<_> = gaps
        .into_iter()
        .enumerate()
        .map(|(idx, gap)| soak_sample(idx as u64 * SAMPLE_EVERY, 0.20, gap))
        .collect();

    assert!(!oscillates(&samples));
}

#[test]
fn large_pinned_gap_sawtooth_is_oscillation() {
    let gaps = [
        10_000, 8_000, 9_500, 7_500, 9_000, 7_000, 8_500, 6_500, 8_000, 6_000, 7_500, 5_500, 7_000,
        5_000, 6_500,
    ];
    let samples: Vec<_> = gaps
        .into_iter()
        .enumerate()
        .map(|(idx, gap)| soak_sample(idx as u64 * SAMPLE_EVERY, 0.20, gap))
        .collect();

    assert!(oscillates(&samples));
}
