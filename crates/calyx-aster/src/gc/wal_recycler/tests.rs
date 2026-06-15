use super::*;
use crate::wal::{Wal, WalOptions};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn recycles_durable_non_active_segments_and_preserves_active_tail() {
    let dir = test_dir("wal-recycle-happy");
    let mut wal = one_record_per_segment_wal(&dir, 6);
    let before = wal.segment_inventory().expect("inventory before");
    assert_eq!(before.len(), 6);
    assert!(before[..5].iter().all(|segment| segment.bytes > 0));

    let recycler = WalRecycler::with_limits(5, 5, Duration::ZERO);
    let result = recycler.run_once_at(&mut wal, 5, 1_000);

    assert!(result.triggered);
    assert_eq!(result.segments_recycled, 5);
    assert_eq!(result.recyclable_segments_before, 5);
    assert!(result.wal_bytes_active_after < result.wal_bytes_active_before);
    assert_eq!(result.wal_segments_recycled_total, 5);
    let after = wal.segment_inventory().expect("inventory after");
    assert!(after[..5].iter().all(|segment| segment.bytes == 0));
    assert_eq!(after[5].first_seq, Some(6));
    assert_eq!(after[5].last_seq, Some(6));
    assert!(after[5].bytes > 0);
    cleanup(dir);
}

#[test]
fn fsync_guard_sets_backoff_before_touching_segments() {
    let dir = test_dir("wal-recycle-fsync");
    let mut wal = one_record_per_segment_wal(&dir, 3);
    let before_bytes = wal.total_segment_bytes().expect("bytes before");
    let recycler = WalRecycler::with_limits(2, 2, Duration::from_millis(1_000));
    recycler.set_fsync_p99_us(15_000);

    let guarded = recycler.run_once_at(&mut wal, 2, 1_000);
    assert_eq!(guarded.skipped_reason, Some(SKIP_FSYNC_GUARD));
    assert!(guarded.rate_limited);
    assert_eq!(wal.total_segment_bytes().unwrap(), before_bytes);

    recycler.set_fsync_p99_us(0);
    let backed_off = recycler.run_once_at(&mut wal, 2, 2_999);
    assert_eq!(backed_off.skipped_reason, Some(SKIP_BACKOFF));
    assert_eq!(wal.total_segment_bytes().unwrap(), before_bytes);

    let after_backoff = recycler.run_once_at(&mut wal, 2, 3_000);
    assert!(after_backoff.triggered);
    assert_eq!(after_backoff.segments_recycled, 2);
    cleanup(dir);
}

#[test]
fn zero_budget_is_explicit_disable() {
    let dir = test_dir("wal-recycle-disabled");
    let mut wal = one_record_per_segment_wal(&dir, 3);
    let before = wal.total_segment_bytes().expect("bytes before");
    let recycler = WalRecycler::with_limits(0, 2, Duration::ZERO);

    let result = recycler.run_once_at(&mut wal, 2, 1);

    assert_eq!(result.skipped_reason, Some(SKIP_DISABLED));
    assert_eq!(wal.total_segment_bytes().unwrap(), before);
    cleanup(dir);
}

#[test]
fn no_durable_non_active_segments_is_noop() {
    let dir = test_dir("wal-recycle-none");
    let mut wal = one_record_per_segment_wal(&dir, 3);
    let before = wal.total_segment_bytes().expect("bytes before");
    let recycler = WalRecycler::with_limits(3, 3, Duration::ZERO);

    let result = recycler.run_once_at(&mut wal, 0, 1);

    assert_eq!(result.skipped_reason, Some(SKIP_NO_RECYCLABLE));
    assert_eq!(wal.total_segment_bytes().unwrap(), before);
    cleanup(dir);
}

#[test]
fn metrics_text_uses_required_metric_names() {
    let result = WalRecyclerResult {
        triggered: true,
        rate_limited: false,
        skipped_reason: None,
        error_code: None,
        error_message: None,
        newest_durable_seq: 10,
        wal_bytes_active_before: 400,
        wal_bytes_active_after: 120,
        recyclable_segments_before: 3,
        segments_recycled: 2,
        bytes_recycled: 280,
        fsync_p99_us: 5_000,
        wal_segments_recycled_total: 2,
    };

    let metrics = result.to_metrics_text("issue484");

    assert!(metrics.contains("calyx_wal_bytes_active{vault=\"issue484\"} 120"));
    assert!(metrics.contains("calyx_wal_segments_recycled_total{vault=\"issue484\"} 2"));
    assert!(metrics.contains("calyx_fsync_p99_us{vault=\"issue484\"} 5000"));
}

fn one_record_per_segment_wal(dir: &PathBuf, records: usize) -> Wal {
    let mut wal = Wal::open(
        dir,
        WalOptions {
            max_segment_bytes: 32,
            ..WalOptions::default()
        },
    )
    .expect("open wal");
    for seq in 1..=records {
        wal.append(format!("record-{seq:03}").as_bytes())
            .expect("append record");
    }
    wal
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-{name}-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup");
}
