use super::*;
use crate::wal::{Wal, WalOptions, replay_dir, replay_dir_after};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-wal-prune-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// Rolls one record per segment (index 0..n-1); seq i+1 lives in segment i.
fn build_rolled_wal(dir: &Path, records: usize) {
    let options = WalOptions {
        max_segment_bytes: 1,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(dir, options).expect("open wal");
    for index in 0..records {
        wal.append(format!("record-{index:04}").as_bytes())
            .expect("append record");
    }
    drop(wal);
}

fn config() -> WalPruneConfig {
    WalPruneConfig {
        max_segments: 1024,
        max_bytes: u64::MAX,
    }
}

fn segment_count(dir: &Path) -> usize {
    super::segment::list_segments(dir)
        .expect("list segments")
        .len()
}

#[test]
fn prunes_covered_prefix_and_reopen_reads_every_tail_row_back() {
    let dir = test_dir("covered-prefix");
    build_rolled_wal(&dir, 6);
    assert_eq!(segment_count(&dir), 6, "one segment per record");

    // Floor 3: seq 1,2,3 (segments 0,1,2) are covered; seq 4,5,6 are the tail.
    let before = replay_dir_after(&dir, 3).expect("replay tail before prune");
    let outcome = prune_covered_segments(&dir, 3, config()).expect("prune");

    assert_eq!(outcome.coverage_floor_seq, 3);
    assert_eq!(outcome.segments_before, 6);
    assert_eq!(outcome.prunable_segments_before, 3);
    assert_eq!(outcome.segments_pruned, 3);
    assert_eq!(outcome.segments_after, 3);
    assert_eq!(segment_count(&dir), 3);

    // Replay-correctness proof: the surviving records replay byte-identically,
    // and the directory still validates as a contiguous segment chain despite
    // segment 0 being gone (replay never assumes index 0 exists).
    let after = replay_dir(&dir).expect("full replay after prune");
    assert_eq!(
        after.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![4, 5, 6]
    );
    assert_eq!(
        after
            .records
            .iter()
            .map(|r| r.payload.clone())
            .collect::<Vec<_>>(),
        before
            .records
            .iter()
            .map(|r| r.payload.clone())
            .collect::<Vec<_>>()
    );
    cleanup(dir);
}

#[test]
fn floor_below_every_segment_is_a_noop() {
    let dir = test_dir("floor-below-all");
    build_rolled_wal(&dir, 4);

    // Floor 0: the first non-active segment (seq 1) is already above it.
    let outcome = prune_covered_segments(&dir, 0, config()).expect("prune");
    assert_eq!(outcome.segments_pruned, 0);
    assert_eq!(outcome.bytes_reclaimed, 0);
    assert_eq!(outcome.prunable_segments_before, 0);
    assert_eq!(segment_count(&dir), 4);
    cleanup(dir);
}

#[test]
fn refuses_rows_above_floor_and_survivors_read_back() {
    let dir = test_dir("refuse-above-floor");
    build_rolled_wal(&dir, 4); // segments 0..3, seq 1..4, active = segment 3

    // Floor 2: seq 1,2 covered; segment 2 (seq 3) is non-active but above the
    // floor, so it must survive along with the active segment.
    let outcome = prune_covered_segments(&dir, 2, config()).expect("prune");
    assert_eq!(outcome.segments_pruned, 2);
    assert_eq!(segment_count(&dir), 2);

    let replay = replay_dir(&dir).expect("full replay");
    assert_eq!(
        replay.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![3, 4],
        "the uncheckpointed tail (seq 3, 4) reads back intact"
    );
    cleanup(dir);
}

#[test]
fn active_segment_is_never_deleted_even_when_covered() {
    let dir = test_dir("active-never-deleted");
    build_rolled_wal(&dir, 3); // segments 0,1,2 = seq 1,2,3; active = 2

    // Floor far above every seq: still keeps the active segment.
    let outcome = prune_covered_segments(&dir, 1_000, config()).expect("prune");
    assert_eq!(outcome.segments_pruned, 2);
    assert_eq!(outcome.segments_after, 1, "active segment survives");

    let replay = replay_dir(&dir).expect("full replay");
    assert_eq!(
        replay.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![3]
    );
    cleanup(dir);
}

#[test]
fn bounded_round_caps_segments_and_is_resumable() {
    let dir = test_dir("bounded-round");
    build_rolled_wal(&dir, 6); // seq 1..6, active = segment 5

    let bounded = WalPruneConfig {
        max_segments: 2,
        max_bytes: u64::MAX,
    };
    let first = prune_covered_segments(&dir, 5, bounded).expect("round 1");
    assert_eq!(first.segments_pruned, 2, "capped at 2 per round");
    assert_eq!(segment_count(&dir), 4);

    let second = prune_covered_segments(&dir, 5, bounded).expect("round 2");
    assert_eq!(second.segments_pruned, 2);
    assert_eq!(segment_count(&dir), 2);

    // Only the active segment (seq 6) and seq 5's segment remain covered; the
    // active is never touched, so the run converges to a single segment.
    let third = prune_covered_segments(&dir, 5, bounded).expect("round 3");
    assert_eq!(third.segments_pruned, 1);
    assert_eq!(segment_count(&dir), 1);

    let replay = replay_dir(&dir).expect("full replay");
    assert_eq!(
        replay.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![6]
    );
    cleanup(dir);
}

#[test]
fn double_prune_is_idempotent() {
    let dir = test_dir("idempotent");
    build_rolled_wal(&dir, 5);

    let first = prune_covered_segments(&dir, 3, config()).expect("first prune");
    assert_eq!(first.segments_pruned, 3);
    let second = prune_covered_segments(&dir, 3, config()).expect("second prune");
    assert_eq!(
        second.segments_pruned, 0,
        "nothing left to prune at same floor"
    );
    assert_eq!(segment_count(&dir), 2);
    cleanup(dir);
}

#[test]
fn zero_config_bounds_fail_closed() {
    let dir = test_dir("bad-config");
    build_rolled_wal(&dir, 2);

    let bad_segments = WalPruneConfig {
        max_segments: 0,
        max_bytes: 1,
    };
    let error = prune_covered_segments(&dir, 1, bad_segments).expect_err("segments=0 rejected");
    assert_eq!(error.code, CALYX_WAL_PRUNE_CONFIG_INVALID);

    let bad_bytes = WalPruneConfig {
        max_segments: 1,
        max_bytes: 0,
    };
    let error = prune_covered_segments(&dir, 1, bad_bytes).expect_err("bytes=0 rejected");
    assert_eq!(error.code, CALYX_WAL_PRUNE_CONFIG_INVALID);
    cleanup(dir);
}
