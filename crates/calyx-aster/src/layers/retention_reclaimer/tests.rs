use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use calyx_core::{CalyxError, Clock, FixedClock, VaultId};

use crate::cf::ColumnFamily;
use crate::collection::{
    DedupPolicy, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::layers::{BlobLayer, RollupWindow, TimeSeriesLayer};
use crate::vault::VaultOptions;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    fn new(ts: u64) -> Self {
        Self(Arc::new(AtomicU64::new(ts)))
    }

    fn set(&self, ts: u64) {
        self.0.store(ts, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn ts_collection(name: &str, retention: RetentionPolicy) -> Collection {
    collection(name, CollectionMode::TimeSeries, retention)
}

fn blob_collection(name: &str, retention: RetentionPolicy) -> Collection {
    collection(name, CollectionMode::Blob, retention)
}

fn kv_collection() -> Collection {
    collection("kv", CollectionMode::KV, RetentionPolicy::Forever)
}

fn collection(name: &str, mode: CollectionMode, retention: RetentionPolicy) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

fn config(max_rows_per_sweep: usize) -> RetentionReclaimerConfig {
    RetentionReclaimerConfig {
        max_rows_per_sweep,
        compact_after_tombstone: false,
    }
}

#[test]
fn timeseries_drop_after_reclaims_old_points_preserves_rollups() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-ts", FixedClock::new(1_000));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection(
        "ts-drop",
        RetentionPolicy::DropAfter(Duration::from_millis(1)),
    );

    layer.ts_write(&col, 7, 997_000_000, 1.0).unwrap();
    layer.ts_write(&col, 7, 999_000_000, 2.0).unwrap();
    layer.ts_write(&col, 7, 1_000_000_000, 3.0).unwrap();
    assert_eq!(ts_point_count(&vault, &col), 3);

    let report = RetentionReclaimer::new(&vault, config(10))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert_eq!(report.ts_points_tombstoned, 1);
    assert_eq!(report.rows_tombstoned, 1);
    assert_eq!(ts_point_count(&vault, &col), 2);
    let rollup = layer
        .ts_rollup(&col, 7, RollupWindow::OneHour, 1_000_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(rollup.count, 3);
    assert_eq!(rollup.sum, 6.0);
}

#[test]
fn timeseries_rollup_only_reclaims_all_raw_points() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-rollup", FixedClock::new(0));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection("ts-rollup", RetentionPolicy::RollupOnly);

    layer.ts_write(&col, 3, 100, 10.0).unwrap();
    layer.ts_write(&col, 3, 200, 20.0).unwrap();

    let report = RetentionReclaimer::new(&vault, config(10))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert_eq!(report.ts_points_tombstoned, 2);
    assert_eq!(ts_point_count(&vault, &col), 0);
    assert_eq!(
        layer
            .ts_rollup(&col, 3, RollupWindow::OneMinute, 100)
            .unwrap()
            .unwrap()
            .count,
        2
    );
}

#[test]
fn bounded_sweep_caps_timeseries_rows() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-cap", FixedClock::new(0));
    let layer = TimeSeriesLayer::new(&vault);
    let col = ts_collection("ts-cap", RetentionPolicy::RollupOnly);
    for ts in 0..5 {
        layer.ts_write(&col, 1, ts, ts as f64).unwrap();
    }

    let report = RetentionReclaimer::new(&vault, config(2))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert!(report.capped);
    assert_eq!(report.ts_points_tombstoned, 2);
    assert_eq!(report.rows_tombstoned, 2);
    assert_eq!(ts_point_count(&vault, &col), 3);
}

#[test]
fn blob_drop_after_uses_manifest_created_at() {
    let clock = ManualClock::new(100);
    let vault = AsterVault::with_clock(vault_id(), b"retention-blob", clock.clone());
    let layer = BlobLayer::new(&vault);
    let col = blob_collection(
        "blob-drop",
        RetentionPolicy::DropAfter(Duration::from_secs(1)),
    );

    let old = BlobId::from_text("old");
    let live = BlobId::from_text("live");
    layer.blob_put(&col, old, b"expired").unwrap();
    clock.set(10_000);
    layer.blob_put(&col, live, b"still-live").unwrap();
    clock.set(10_500);
    assert_eq!(blob_manifest_count(&vault, &col), 2);

    let report = RetentionReclaimer::new(&vault, config(10))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert_eq!(report.blob_manifests_tombstoned, 1);
    assert_eq!(report.blob_chunks_tombstoned, 1);
    assert_eq!(layer.blob_get(&col, old).unwrap(), None);
    assert_eq!(
        layer.blob_get(&col, live).unwrap(),
        Some(b"still-live".to_vec())
    );
    assert_eq!(blob_manifest_count(&vault, &col), 1);
}

#[test]
fn blob_orphan_chunks_reclaimed_under_forever() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-orphan", FixedClock::new(1));
    let col = blob_collection("blob-orphan", RetentionPolicy::Forever);
    let id = BlobId::from_text("orphan");
    vault
        .write_cf(
            ColumnFamily::Blob,
            blob::chunk_key(&col, id, 0),
            b"orphan-bytes".to_vec(),
        )
        .unwrap();
    assert_eq!(blob_chunk_count(&vault, &col), 1);

    let report = RetentionReclaimer::new(&vault, config(10))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert_eq!(report.orphan_blob_chunks_tombstoned, 1);
    assert_eq!(report.blob_chunks_tombstoned, 1);
    assert_eq!(blob_chunk_count(&vault, &col), 0);
}

#[test]
fn legacy_blob_manifest_is_skipped_under_drop_after() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-legacy", FixedClock::new(10_000));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection(
        "blob-legacy",
        RetentionPolicy::DropAfter(Duration::from_millis(1)),
    );
    let id = BlobId::from_text("legacy");
    vault
        .write_cf(
            ColumnFamily::Blob,
            blob::manifest_key(&col, id),
            legacy_manifest_value(b"legacy"),
        )
        .unwrap();

    let report = RetentionReclaimer::new(&vault, config(10))
        .unwrap()
        .run_collection(&col)
        .unwrap();

    assert_eq!(report.legacy_blob_manifests_skipped, 1);
    assert_eq!(report.blob_manifests_tombstoned, 0);
    assert_eq!(
        layer
            .blob_manifest(&col, id)
            .unwrap()
            .unwrap()
            .created_at_ms,
        None
    );
}

#[test]
fn invalid_config_and_collection_fail_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"retention-invalid", FixedClock::new(1));

    assert_eq!(
        expect_new_error(&vault, config(0)).code,
        CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION
    );
    assert_eq!(
        RetentionReclaimer::new(&vault, config(10))
            .unwrap()
            .run_collection(&kv_collection())
            .unwrap_err()
            .code,
        CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION
    );
}

#[test]
fn durable_reclaimer_fsv_writes_readback_artifacts() {
    const FSV_NOW_MS: u64 = 1_700_000_002_000;

    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("retention-vault"))
        .unwrap_or_else(|| temp_dir("retention-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable_with_clock(
        &dir,
        vault_id(),
        b"retention-fsv",
        VaultOptions::default(),
        FixedClock::new(FSV_NOW_MS),
    )
    .unwrap();
    let ts_col = ts_collection(
        "fsv-ts",
        RetentionPolicy::DropAfter(Duration::from_millis(1)),
    );
    let blob_col = blob_collection(
        "fsv-blob",
        RetentionPolicy::DropAfter(Duration::from_secs(1)),
    );
    create_collection(&vault, ts_col.clone()).unwrap();
    create_collection(&vault, blob_col.clone()).unwrap();

    let ts = TimeSeriesLayer::new(&vault);
    let now_nanos = vault.clock_now().saturating_mul(NANOS_PER_MILLI);
    let live_nanos = now_nanos.saturating_add(NANOS_PER_MILLI / 2);
    ts.ts_write(&ts_col, 42, now_nanos.saturating_sub(5_000_000), 4.0)
        .unwrap();
    ts.ts_write(&ts_col, 42, live_nanos, 8.0).unwrap();

    let now_ms = vault.clock_now();
    let expired = BlobId::from_text("fsv-expired");
    let live = BlobId::from_text("fsv-live");
    write_blob_rows(&vault, &blob_col, expired, b"expired", now_ms - 10_000);
    write_blob_rows(&vault, &blob_col, live, b"live", now_ms + 60_000);
    vault
        .write_cf(
            ColumnFamily::Blob,
            blob::chunk_key(&blob_col, BlobId::from_text("fsv-orphan"), 0),
            b"orphan".to_vec(),
        )
        .unwrap();
    let before = serde_json::json!({
        "ts_points": ts_point_count(&vault, &ts_col),
        "blob_manifests": blob_manifest_count(&vault, &blob_col),
        "blob_chunks": blob_chunk_count(&vault, &blob_col),
    });

    let reclaimer = RetentionReclaimer::new(
        &vault,
        RetentionReclaimerConfig {
            max_rows_per_sweep: 20,
            compact_after_tombstone: true,
        },
    )
    .unwrap();
    let ts_report = reclaimer.run_collection(&ts_col).unwrap();
    let blob_report = reclaimer.run_collection(&blob_col).unwrap();
    vault.flush().unwrap();

    let blob_layer = BlobLayer::new(&vault);
    let after = serde_json::json!({
        "ts_points": ts_point_count(&vault, &ts_col),
        "ts_rollup_count": ts.ts_rollup(&ts_col, 42, RollupWindow::OneHour, live_nanos).unwrap().unwrap().count,
        "expired_blob_absent": blob_layer.blob_get(&blob_col, expired).unwrap().is_none(),
        "live_blob_bytes": blob_layer.blob_get(&blob_col, live).unwrap().unwrap(),
        "blob_manifests": blob_manifest_count(&vault, &blob_col),
        "blob_chunks": blob_chunk_count(&vault, &blob_col),
    });
    let invalid_code = expect_new_error(&vault, config(0)).code;
    let readback = serde_json::json!({
        "issue": 591,
        "source_of_truth": dir.display().to_string(),
        "before": before,
        "after": after,
        "timeseries_report": ts_report,
        "blob_report": blob_report,
        "fail_closed_code": invalid_code,
        "cf_files": physical_files(&dir.join("cf")),
    });
    assert_eq!(readback["after"]["ts_points"], serde_json::json!(1));
    assert_eq!(readback["after"]["ts_rollup_count"], serde_json::json!(2));
    assert_eq!(
        readback["after"]["expired_blob_absent"],
        serde_json::json!(true)
    );
    assert_eq!(
        readback["after"]["live_blob_bytes"],
        serde_json::json!(b"live")
    );
    assert_eq!(
        readback["fail_closed_code"],
        serde_json::json!(CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION)
    );

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("issue591-retention-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        println!("issue591_retention_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn ts_point_count(vault: &AsterVault<impl Clock>, col: &Collection) -> usize {
    let cid = timeseries::collection_id(col).to_be_bytes();
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::TimeSeries)
        .unwrap()
        .into_iter()
        .filter(|(key, _)| matches!(parse_ts_key(key, &cid), Some(TimeSeriesKey::Point { .. })))
        .count()
}

fn blob_manifest_count(vault: &AsterVault<impl Clock>, col: &Collection) -> usize {
    blob_row_count(vault, col, |key, cid| {
        matches!(parse_blob_key(key, cid), Some(BlobKey::Manifest { .. }))
    })
}

fn blob_chunk_count(vault: &AsterVault<impl Clock>, col: &Collection) -> usize {
    blob_row_count(vault, col, |key, cid| {
        matches!(parse_blob_key(key, cid), Some(BlobKey::Chunk { .. }))
    })
}

fn blob_row_count(
    vault: &AsterVault<impl Clock>,
    col: &Collection,
    matches_row: impl Fn(&[u8], &[u8; 8]) -> bool,
) -> usize {
    let cid = blob::collection_id(col).to_be_bytes();
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Blob)
        .unwrap()
        .into_iter()
        .filter(|(key, _)| matches_row(key, &cid))
        .count()
}

fn write_blob_rows(
    vault: &AsterVault<impl Clock>,
    col: &Collection,
    id: BlobId,
    bytes: &[u8],
    created_at_ms: u64,
) {
    vault
        .write_cf(
            ColumnFamily::Blob,
            blob::chunk_key(col, id, 0),
            bytes.to_vec(),
        )
        .unwrap();
    vault
        .write_cf(
            ColumnFamily::Blob,
            blob::manifest_key(col, id),
            manifest_value(bytes, Some(created_at_ms)),
        )
        .unwrap();
}

fn legacy_manifest_value(bytes: &[u8]) -> Vec<u8> {
    manifest_value(bytes, None)
}

fn manifest_value(bytes: &[u8], created_at_ms: Option<u64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(53);
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(&1_u32.to_be_bytes());
    out.extend_from_slice(blake3::hash(bytes).as_bytes());
    out.push(0);
    if let Some(created_at_ms) = created_at_ms {
        out.extend_from_slice(&created_at_ms.to_be_bytes());
    }
    out
}

fn physical_files(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(physical_files(&path));
            } else {
                files.push(serde_json::json!({
                    "path": path.display().to_string(),
                    "bytes": fs::read(&path).unwrap().len(),
                }));
            }
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}

fn expect_new_error<C: Clock>(
    vault: &AsterVault<C>,
    config: RetentionReclaimerConfig,
) -> CalyxError {
    match RetentionReclaimer::new(vault, config) {
        Ok(_) => panic!("retention reclaimer construction should fail"),
        Err(error) => error,
    }
}
