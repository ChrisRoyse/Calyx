use calyx_aster::gc::WalRecycler;
use calyx_aster::wal::{Wal, WalOptions, WalSegmentStatus};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod fsv_support;
use fsv_support::write_json;

#[test]
#[ignore = "manual FSV for issue #484 WAL recycler"]
fn ph58_wal_recycler_fsv() {
    let root = fsv_root().join("wal");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create FSV root");

    let happy_dir = root.join("happy");
    let mut happy_wal = one_record_per_segment_wal(&happy_dir, 10);
    let happy_before = happy_wal.segment_inventory().expect("happy before");
    write_inventory(&root, "happy-before", &happy_before);
    let recycler = WalRecycler::with_limits(9, 9, Duration::ZERO);
    recycler.set_fsync_p99_us(5_000);
    let happy_result = recycler.run_once_at(&mut happy_wal, 9, 1_000);
    let happy_after = happy_wal.segment_inventory().expect("happy after");
    write_inventory(&root, "happy-after", &happy_after);
    fs::write(
        root.join("metrics-happy.prom"),
        happy_result.to_metrics_text("issue484-wal"),
    )
    .expect("write happy metrics");

    let fsync_dir = root.join("edge-fsync-guard");
    let mut fsync_wal = one_record_per_segment_wal(&fsync_dir, 3);
    let fsync_before = fsync_wal.segment_inventory().expect("fsync before");
    let guarded = WalRecycler::with_limits(2, 2, Duration::from_millis(1_000));
    guarded.set_fsync_p99_us(15_000);
    let fsync_guard = guarded.run_once_at(&mut fsync_wal, 2, 2_000);
    guarded.set_fsync_p99_us(0);
    let fsync_backoff = guarded.run_once_at(&mut fsync_wal, 2, 3_999);
    let fsync_after = fsync_wal.segment_inventory().expect("fsync after");
    write_inventory(&root, "edge-fsync-after", &fsync_after);
    fs::write(
        root.join("metrics-edge-fsync.prom"),
        fsync_guard.to_metrics_text("issue484-wal-fsync"),
    )
    .expect("write fsync metrics");

    let no_durable_dir = root.join("edge-no-durable");
    let mut no_durable_wal = one_record_per_segment_wal(&no_durable_dir, 3);
    let no_durable_before = no_durable_wal
        .segment_inventory()
        .expect("no durable before");
    let no_durable =
        WalRecycler::with_limits(3, 3, Duration::ZERO).run_once_at(&mut no_durable_wal, 0, 1);
    let no_durable_after = no_durable_wal
        .segment_inventory()
        .expect("no durable after");
    write_inventory(&root, "edge-no-durable-after", &no_durable_after);

    let disabled_dir = root.join("edge-disabled");
    let mut disabled_wal = one_record_per_segment_wal(&disabled_dir, 3);
    let disabled_before = disabled_wal.segment_inventory().expect("disabled before");
    let disabled =
        WalRecycler::with_limits(0, 3, Duration::ZERO).run_once_at(&mut disabled_wal, 2, 1);
    let disabled_after = disabled_wal.segment_inventory().expect("disabled after");
    write_inventory(&root, "edge-disabled-after", &disabled_after);

    let summary = json!({
        "issue": 484,
        "source_of_truth": {
            "root": root.display().to_string(),
            "happy_wal_dir": happy_dir.display().to_string(),
            "metrics_happy": "metrics-happy.prom"
        },
        "trigger": "WalRecycler::run_once_at(newest_durable_seq=9)",
        "happy": {
            "before": inventory_counts(&happy_before),
            "after": inventory_counts(&happy_after),
            "result": result_json(&happy_result),
            "expected_recycled_segments": 9,
            "expected_active_tail_seq": 10
        },
        "edge_fsync_guard": {
            "before": inventory_counts(&fsync_before),
            "after": inventory_counts(&fsync_after),
            "guard_result": result_json(&fsync_guard),
            "backoff_result": result_json(&fsync_backoff)
        },
        "edge_no_durable": {
            "before": inventory_counts(&no_durable_before),
            "after": inventory_counts(&no_durable_after),
            "result": result_json(&no_durable)
        },
        "edge_disabled": {
            "before": inventory_counts(&disabled_before),
            "after": inventory_counts(&disabled_after),
            "result": result_json(&disabled)
        }
    });
    let summary_path = root.join("wal-recycler-summary.json");
    write_json(&summary_path, &summary);
    let summary_bytes = fs::read(&summary_path).expect("read summary");
    println!("PH58_WAL_RECYCLER_FSV_ROOT={}", root.display());
    println!("PH58_WAL_RECYCLER_SUMMARY={}", summary_path.display());
    println!(
        "PH58_WAL_RECYCLER_SUMMARY_BLAKE3={}",
        digest_hex(&summary_bytes)
    );
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());

    assert!(happy_result.triggered);
    assert_eq!(happy_result.segments_recycled, 9);
    assert_eq!(
        happy_after
            .iter()
            .filter(|segment| segment.bytes == 0)
            .count(),
        9
    );
    assert_eq!(
        happy_after.last().and_then(|segment| segment.last_seq),
        Some(10)
    );
    assert_eq!(fsync_guard.skipped_reason, Some("fsync_p99_guard"));
    assert_eq!(fsync_backoff.skipped_reason, Some("fsync_backoff_active"));
    assert_eq!(
        inventory_counts(&fsync_before),
        inventory_counts(&fsync_after)
    );
    assert_eq!(
        no_durable.skipped_reason,
        Some("no_recyclable_wal_segments")
    );
    assert_eq!(
        inventory_counts(&no_durable_before),
        inventory_counts(&no_durable_after)
    );
    assert_eq!(disabled.skipped_reason, Some("wal_recycler_disabled"));
    assert_eq!(
        inventory_counts(&disabled_before),
        inventory_counts(&disabled_after)
    );
}

fn one_record_per_segment_wal(dir: &Path, records: usize) -> Wal {
    fs::create_dir_all(dir).expect("create wal dir");
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

fn write_inventory(root: &Path, name: &str, inventory: &[WalSegmentStatus]) {
    let rows = inventory
        .iter()
        .map(|segment| {
            let bytes = fs::read(&segment.path).expect("read segment bytes");
            json!({
                "index": segment.index,
                "path": segment.path.display().to_string(),
                "bytes": segment.bytes,
                "first_seq": segment.first_seq,
                "last_seq": segment.last_seq,
                "record_count": segment.record_count,
                "active": segment.active,
                "blake3": digest_hex(&bytes),
            })
        })
        .collect::<Vec<_>>();
    write_json(&root.join(format!("{name}.json")), &json!(rows));
}

fn inventory_counts(inventory: &[WalSegmentStatus]) -> serde_json::Value {
    json!({
        "segments": inventory.len(),
        "non_empty_segments": inventory.iter().filter(|segment| segment.bytes > 0).count(),
        "zero_segments": inventory.iter().filter(|segment| segment.bytes == 0).count(),
        "total_bytes": inventory.iter().map(|segment| segment.bytes).sum::<u64>(),
        "last_seq": inventory.iter().filter_map(|segment| segment.last_seq).max(),
    })
}

fn result_json(result: &calyx_aster::gc::WalRecyclerResult) -> serde_json::Value {
    json!({
        "triggered": result.triggered,
        "rate_limited": result.rate_limited,
        "skipped_reason": result.skipped_reason,
        "error_code": result.error_code,
        "newest_durable_seq": result.newest_durable_seq,
        "wal_bytes_active_before": result.wal_bytes_active_before,
        "wal_bytes_active_after": result.wal_bytes_active_after,
        "recyclable_segments_before": result.recyclable_segments_before,
        "segments_recycled": result.segments_recycled,
        "bytes_recycled": result.bytes_recycled,
        "fsync_p99_us": result.fsync_p99_us,
        "wal_segments_recycled_total": result.wal_segments_recycled_total,
    })
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_PH58_WAL_ANN_GC_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph58-wal-ann-gc-fsv"))
}

fn digest_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
