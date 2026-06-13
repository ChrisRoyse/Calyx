use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use calyx_core::{FixedClock, VaultId};
use proptest::prelude::*;

use crate::collection::{
    DedupPolicy, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::vault::VaultOptions;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn ts_collection() -> Collection {
    Collection {
        name: "metrics".to_string(),
        mode: CollectionMode::TimeSeries,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

#[test]
fn points_round_trip_in_time_order_with_correct_rollup() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(0));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection();

    for (ts, val) in [(100_u64, 1.0_f64), (300, 3.0), (200, 2.0)] {
        layer.ts_write(&col, 7, ts, val).unwrap();
    }

    // Key discriminants land in the disjoint TS key-space.
    let pk = point_key(&col, 7, 100);
    assert_eq!(pk[0], DISC_TS);
    assert_eq!(pk[1], KIND_POINT);
    let rk = rollup_key(&col, 7, RollupWindow::OneHour, 0);
    assert_eq!(rk[0], DISC_TS);
    assert_eq!(rk[1], KIND_ROLLUP);

    let range = layer.ts_range(&col, 7, 0, 400).unwrap();
    assert_eq!(range, vec![(100, 1.0), (200, 2.0), (300, 3.0)]);

    for window in RollupWindow::ALL {
        let rollup = layer.ts_rollup(&col, 7, window, 0).unwrap().unwrap();
        assert_eq!(rollup.count, 3, "{window:?}");
        assert_eq!(rollup.sum, 6.0, "{window:?}");
        assert_eq!(rollup.min, 1.0, "{window:?}");
        assert_eq!(rollup.max, 3.0, "{window:?}");
    }

    // A different series is fully isolated.
    assert!(layer.ts_range(&col, 8, 0, 400).unwrap().is_empty());
    assert!(
        layer
            .ts_rollup(&col, 8, RollupWindow::OneHour, 0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn rollups_bucket_by_window_start() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(0));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection();

    // Two points one hour apart fall in distinct 1h buckets but the same 1d
    // bucket.
    let t0 = 0_u64;
    let t1 = NANOS_PER_HOUR + 5;
    layer.ts_write(&col, 1, t0, 10.0).unwrap();
    layer.ts_write(&col, 1, t1, 20.0).unwrap();

    let h0 = layer
        .ts_rollup(&col, 1, RollupWindow::OneHour, t0)
        .unwrap()
        .unwrap();
    let h1 = layer
        .ts_rollup(&col, 1, RollupWindow::OneHour, t1)
        .unwrap()
        .unwrap();
    assert_eq!((h0.count, h0.sum), (1, 10.0));
    assert_eq!((h1.count, h1.sum), (1, 20.0));

    let day = layer
        .ts_rollup(&col, 1, RollupWindow::OneDay, t0)
        .unwrap()
        .unwrap();
    assert_eq!(
        (day.count, day.sum, day.min, day.max),
        (2, 30.0, 10.0, 20.0)
    );
}

#[test]
fn retention_drop_after_skips_old_points_on_read() {
    // now = 1000 ms -> 1e9 ns; DropAfter(1ms) -> floor at 999_000_000 ns.
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = TimeSeriesLayer::new(&vault);
    let mut col = ts_collection();
    col.retention = RetentionPolicy::DropAfter(Duration::from_millis(1));

    layer.ts_write(&col, 1, 100, 1.0).unwrap(); // ancient, below floor
    layer.ts_write(&col, 1, 999_000_000, 2.0).unwrap(); // exactly at floor (kept)
    layer.ts_write(&col, 1, 1_000_000_000, 3.0).unwrap(); // now (kept)

    let range = layer.ts_range(&col, 1, 0, u64::MAX).unwrap();
    assert_eq!(range, vec![(999_000_000, 2.0), (1_000_000_000, 3.0)]);
}

#[test]
fn edge_cases_fail_closed_with_exact_codes() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(0));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection();

    // (1) empty range returns empty.
    layer.ts_write(&col, 1, 500, 1.0).unwrap();
    assert!(layer.ts_range(&col, 1, 0, 100).unwrap().is_empty());

    // (2) inverted range returns empty.
    assert!(layer.ts_range(&col, 1, 900, 100).unwrap().is_empty());

    // (3) rollup before any write to a series returns None.
    assert!(
        layer
            .ts_rollup(&col, 99, RollupWindow::OneMinute, 0)
            .unwrap()
            .is_none()
    );

    // (4) non-finite value rejected loudly (protects rollups from NaN).
    assert_eq!(
        layer.ts_write(&col, 1, 600, f64::NAN).unwrap_err().code,
        CALYX_INVALID_ARGUMENT
    );
    assert_eq!(
        layer
            .ts_write(&col, 1, 700, f64::INFINITY)
            .unwrap_err()
            .code,
        CALYX_INVALID_ARGUMENT
    );

    // (5) wrong collection mode rejected.
    let mut wrong = col.clone();
    wrong.mode = CollectionMode::KV;
    assert_eq!(
        layer.ts_write(&wrong, 1, 1, 1.0).unwrap_err().code,
        CALYX_INVALID_ARGUMENT
    );

    // (6) corrupt point value bytes fail closed.
    vault
        .write_cf(
            ColumnFamily::TimeSeries,
            point_key(&col, 1, 800),
            vec![0, 0, 0],
        )
        .unwrap();
    assert_eq!(
        layer.ts_range(&col, 1, 0, 1000).unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );
}

proptest! {
    #[test]
    fn rollup_sum_equals_sum_of_window_values(
        // Small integers keep f64 addition exact regardless of fold order.
        vals in proptest::collection::vec(0_i32..1000, 1..32),
    ) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(0));
        let layer = TimeSeriesLayer::new(&vault);
        let col = ts_collection();
        let mut expected_sum = 0.0_f64;
        let mut expected_min = f64::INFINITY;
        let mut expected_max = f64::NEG_INFINITY;
        for (i, raw) in vals.iter().enumerate() {
            let val = f64::from(*raw);
            // All ts within the same 1-minute window (ts < 60e9 ns).
            layer.ts_write(&col, 1, i as u64, val).unwrap();
            expected_sum += val;
            expected_min = expected_min.min(val);
            expected_max = expected_max.max(val);
        }
        let rollup = layer.ts_rollup(&col, 1, RollupWindow::OneMinute, 0).unwrap().unwrap();
        prop_assert_eq!(rollup.count, vals.len() as u64);
        prop_assert_eq!(rollup.sum, expected_sum);
        prop_assert_eq!(rollup.min, expected_min);
        prop_assert_eq!(rollup.max, expected_max);
    }
}

#[test]
fn durable_ts_fsv_writes_readback_artifacts() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("ts-vault"))
        .unwrap_or_else(|| temp_dir("ts-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"ts-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection();
    create_collection(&vault, col.clone()).unwrap();

    layer.ts_write(&col, 1, 1_700_000_100, 0.42).unwrap();
    layer.ts_write(&col, 1, 1_700_000_200, 0.55).unwrap();

    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"ts-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_layer = TimeSeriesLayer::new(&reopened);

    let range = reopened_layer.ts_range(&col, 1, 0, 9_999_999_999).unwrap();
    assert_eq!(range, vec![(1_700_000_100, 0.42), (1_700_000_200, 0.55)]);
    let rollup = reopened_layer
        .ts_rollup(&col, 1, RollupWindow::OneHour, 1_700_000_100)
        .unwrap()
        .unwrap();
    assert_eq!(rollup.count, 2);
    // 0.42 + 0.55 == 0.97 within f64 epsilon.
    assert!((rollup.sum - 0.97).abs() < 1e-9, "sum was {}", rollup.sum);

    let point_files = physical_files(&dir.join("cf").join("timeseries"));
    assert!(
        !point_files.is_empty(),
        "cf/timeseries must hold on-disk shards"
    );

    let pk = point_key(&col, 1, 1_700_000_100);
    let readback = serde_json::json!({
        "issue": 453,
        "layer": "timeseries",
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::TimeSeries.name(),
        "point_key_hex": hex_bytes(&pk),
        "point_discriminant": format!("{:#04x}", pk[0]),
        "point_kind": format!("{:#04x}", pk[1]),
        "rollup_1h_count": rollup.count,
        "rollup_1h_sum": rollup.sum,
        "range_len": range.len(),
        "timeseries_cf_files": point_files,
    });
    assert_eq!(readback["rollup_1h_count"], serde_json::json!(2));

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-timeseries-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        println!("ph53_timeseries_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn physical_files(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            }));
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
