#![cfg(target_os = "linux")]

mod fsv_support;

mod soak_ph58 {
    use calyx_aster::cf::{CfRouter, ColumnFamily};
    use calyx_aster::gc::{
        CompactionGcReclaimer, GcRateLimit, VaultCompactionGcTarget, scan_tombstone_inventory,
    };
    use calyx_aster::mvcc::{Freshness, VersionedCfStore, tombstone_value};
    use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
    use calyx_aster::vault::{AsterVault, VaultOptions};
    use calyx_core::{Clock, Ts, VaultId};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::fsv_support::{fsv_root_os, reset_dir};

    const START_TS: Ts = 1_800_000_000_000;
    const BATCH: usize = 1_000;
    const FSV_MEMTABLE_BYTES: usize = 64 * 1024 * 1024;

    #[derive(Clone, Debug)]
    struct SharedClock {
        now: Arc<AtomicU64>,
    }

    impl SharedClock {
        fn new(now: Ts) -> Self {
            Self {
                now: Arc::new(AtomicU64::new(now)),
            }
        }

        fn set(&self, now: Ts) {
            self.now.store(now, Ordering::Relaxed);
        }
    }

    impl Clock for SharedClock {
        fn now(&self) -> Ts {
            self.now.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn long_reader() {
        let started = Instant::now();
        let root = case_root("long_reader");
        reset_dir(&root);
        let summary = long_reader_aborted_version_reclaimed(&root);
        write_artifact("ph58_long_reader.json", &summary);
        println!(
            "PH58_LONG_READER_ROOT={} elapsed_ms={}",
            root.display(),
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn tombstone() {
        let started = Instant::now();
        let root = case_root("tombstone");
        reset_dir(&root);
        let summary = tombstone_ratio_bounded(&root);
        write_artifact("ph58_tombstone.json", &summary);
        println!(
            "PH58_TOMBSTONE_ROOT={} elapsed_ms={}",
            root.display(),
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn janitor() {
        let started = Instant::now();
        let root = case_root("janitor");
        reset_dir(&root);
        let summary = logs_and_artifacts_bounded(&root);
        write_artifact("ph58_janitor.json", &summary);
        println!(
            "PH58_JANITOR_ROOT={} elapsed_ms={}",
            root.display(),
            started.elapsed().as_millis()
        );
    }

    fn long_reader_aborted_version_reclaimed(root: &Path) -> Value {
        let vault_dir = root.join("vault");
        let clock = SharedClock::new(START_TS);
        let router = CfRouter::open(&vault_dir, FSV_MEMTABLE_BYTES).expect("open CF router");
        let store = VersionedCfStore::new_with_router(0, router);
        let key = b"ph58-long-reader-key".to_vec();
        write_versions(&store, &key, 1, 10_000, 512);
        let pinned = store.pin_snapshot_at(5_000, Freshness::FreshDerived, &clock, 200);
        assert_eq!(
            store
                .read_at(pinned, ColumnFamily::Base, &key, &clock)
                .expect("pinned read before expiry")
                .expect("safe value")
                .len(),
            512
        );
        write_versions(&store, &key, 10_001, 20_000, 512);
        store.flush_all_cfs().expect("flush long-reader fixture");
        let before_tick = store.snapshot_gc_tick(&clock, 1_000);
        let metrics_before = store.snapshot_gc_metrics(clock.now());
        let sst_bytes_before = tree_bytes(&vault_dir.join("cf"));
        let df_before = hotpool_df_text();
        let du_before = command_text("du", &["-sb", vault_dir.to_str().unwrap()]);
        assert!(before_tick.metrics.oldest_pinned_seq_gap >= 15_000);

        clock.set(START_TS + 250);
        let expired = store
            .read_at(pinned, ColumnFamily::Base, &key, &clock)
            .expect_err("reader must see expired lease");
        assert_eq!(expired.code, "CALYX_READER_LEASE_EXPIRED");
        let abort_tick = store.snapshot_gc_tick(&clock, 1_000);
        assert_eq!(abort_tick.metrics.reader_lease_expired_total, 1);
        store.set_snapshot_gc_rate_limit(GcRateLimit::new(25_000, Duration::ZERO));
        let result = store
            .snapshot_version_gc_tick(&clock)
            .expect("snapshot GC after abort");
        store.flush_all_cfs().expect("flush after snapshot GC");
        let metrics_after = store.snapshot_gc_metrics(clock.now());
        let sst_bytes_after = tree_bytes(&vault_dir.join("cf"));
        let df_after = hotpool_df_text();
        let du_after = command_text("du", &["-sb", vault_dir.to_str().unwrap()]);
        assert!(result.versions_reclaimed > 0);
        assert!(metrics_after.bytes_freed_total > metrics_before.bytes_freed_total);
        assert!(sst_bytes_after <= sst_bytes_before);
        json!({
            "trigger": "expired reader lease at seq 5000 followed by snapshot_version_gc_once",
            "input": {"versions_before_pin": 10000, "pinned_seq": 5000, "versions_after_pin": 10000, "lease_ms": 200},
            "expected": {"oldest_pinned_seq_gap_min": 15000, "reader_lease_expired_total": 1, "gc_bytes_delta_gt_zero": true, "disk_flat_or_improved": true},
            "actual": {
                "gap_before": before_tick.metrics.oldest_pinned_seq_gap,
                "abort_tick": abort_tick,
                "expired_error_code": expired.code,
                "gc_result": result,
                "gc_bytes_before": metrics_before.bytes_freed_total,
                "gc_bytes_after": metrics_after.bytes_freed_total,
                "sst_bytes_before": sst_bytes_before,
                "sst_bytes_after": sst_bytes_after,
                "du_before": du_before,
                "du_after": du_after,
                "df_before": df_before,
                "df_after": df_after,
                "metrics_before": format!("{metrics_before:?}"),
                "metrics_after": format!("{metrics_after:?}"),
                "panic_free": true
            }
        })
    }

    fn tombstone_ratio_bounded(root: &Path) -> Value {
        let vault_dir = root.join("vault");
        let clock = SharedClock::new(START_TS + 1_000);
        let vault = open_vault(&vault_dir, clock, b"issue487-ph58-tombstone");
        write_live_range(&vault, 0, 50_000);
        vault.flush().expect("flush live rows");
        write_tombstone_range(&vault, 0, 30_000);
        vault.flush().expect("flush tombstones");
        let snapshot_gc = vault
            .snapshot_version_gc_once(GcRateLimit::new(100_000, Duration::ZERO))
            .expect("snapshot GC before tombstone sweep");
        vault.flush().expect("flush after snapshot GC");
        let before = scan_tombstone_inventory(&vault_dir).expect("scan tombstones before");
        let baseline_p99 = measure_read_p99_ns(&vault, 30_000, 32_000);
        assert!(before.tombstone_ratio() > 0.5);

        let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000_000, 0);
        let target = VaultCompactionGcTarget {
            vault: &vault,
            vault_dir: &vault_dir,
        };
        let mut results = Vec::new();
        let mut ratios = vec![before.tombstone_ratio()];
        for pass in 0..3 {
            let result = reclaimer.maybe_trigger_at(&target, 0.5, pass * 1_000);
            ratios.push(result.tombstone_ratio_after);
            results.push(result);
        }
        let after = scan_tombstone_inventory(&vault_dir).expect("scan tombstones after");
        let after_p99 = measure_read_p99_ns(&vault, 30_000, 32_000);
        assert!(after.tombstone_ratio() <= 0.1);
        assert!(after_p99 <= baseline_p99.saturating_mul(2).max(1));
        let status_after = status(&vault, &vault_dir);
        json!({
            "trigger": "delete-heavy SST set followed by three CompactionGcReclaimer passes",
            "input": {"ingested": 50000, "deleted": 30000, "live_after_delete": 20000},
            "expected": {"ratio_before_gt_0_5": true, "ratio_after_lte_0_1": true, "serving_p99_after_lte_2x_baseline": true},
            "actual": {
                "snapshot_gc_result": snapshot_gc,
                "ratio_series": ratios,
                "results": results.into_iter().map(compaction_result_json).collect::<Vec<_>>(),
                "before": inventory_json(&before),
                "after": inventory_json(&after),
                "serving_p99_baseline_ns": baseline_p99,
                "serving_p99_after_ns": after_p99,
                "metrics_after": status_after.to_metrics_text("issue487-tombstone"),
                "tombstone_metrics": format!("calyx_tombstone_ratio{{vault=\"issue487-tombstone\"}} {:.6}\n", after.tombstone_ratio()),
                "du_after": command_text("du", &["-sb", vault_dir.to_str().unwrap()]),
                "df_after": hotpool_df_text(),
                "panic_free": true
            }
        })
    }

    fn logs_and_artifacts_bounded(root: &Path) -> Value {
        let child_root = root.join("issue486-janitor");
        let output = Command::new(cargo_bin())
            .current_dir(repo_root())
            .env("CALYX_FSV_ROOT", &child_root)
            .args([
                "test",
                "-p",
                "calyx-anneal",
                "--test",
                "issue486_janitor_fsv",
                "--",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .expect("run calyx-anneal janitor FSV");
        fs::write(root.join("janitor-cargo-stdout.txt"), &output.stdout).unwrap();
        fs::write(root.join("janitor-cargo-stderr.txt"), &output.stderr).unwrap();
        assert!(
            output.status.success(),
            "janitor child FSV failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary_path = child_root.join("issue486-summary.json");
        let summary: Value =
            serde_json::from_slice(&fs::read(&summary_path).expect("read janitor summary"))
                .expect("decode janitor summary");
        let metrics = fs::read_to_string(child_root.join("metrics.prom")).unwrap();
        let log_bytes = tree_bytes(&child_root.join("happy/logs"));
        let artifact_dirs = fs::read_dir(child_root.join("happy/target"))
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().is_dir())
            .count();
        assert!(metrics.contains("calyx_janitor_bytes_freed_total"));
        assert!(log_bytes <= 80);
        assert_eq!(artifact_dirs, 2);
        json!({
            "trigger": "child cargo run of issue486_janitor_manual_fsv_bytes",
            "source_of_truth": {
                "root": child_root,
                "summary": summary_path,
                "metrics": child_root.join("metrics.prom"),
                "ledger": child_root.join("happy/ledger/janitor.jsonl")
            },
            "expected": {"log_bytes_lte_cap": 80, "artifact_dirs_kept": 2, "janitor_bytes_freed_gt_zero": true},
            "actual": {
                "metrics": metrics,
                "happy_result": summary["happy"]["result"].clone(),
                "log_bytes_after": log_bytes,
                "artifact_dirs_after": artifact_dirs,
                "summary_issue": summary["issue"].clone(),
                "panic_free": true
            }
        })
    }

    fn write_versions(
        store: &VersionedCfStore,
        key: &[u8],
        start: u64,
        end: u64,
        value_len: usize,
    ) {
        for seq in start..=end {
            let committed = store
                .commit_batch([(
                    ColumnFamily::Base,
                    key.to_vec(),
                    known_value(seq, value_len),
                )])
                .expect("write version");
            assert_eq!(committed, seq);
        }
    }

    fn write_live_range(vault: &AsterVault<SharedClock>, start: u64, end: u64) {
        let mut next = start;
        while next < end {
            let upper = (next + BATCH as u64).min(end);
            let rows = (next..upper).map(|id| {
                (
                    ColumnFamily::Base,
                    format!("key-{id:05}").into_bytes(),
                    format!("value-{id:05}").into_bytes(),
                )
            });
            vault.write_cf_batch(rows).expect("write live batch");
            next = upper;
        }
    }

    fn write_tombstone_range(vault: &AsterVault<SharedClock>, start: u64, end: u64) {
        let mut next = start;
        while next < end {
            let upper = (next + BATCH as u64).min(end);
            let rows = (next..upper).map(|id| {
                (
                    ColumnFamily::Base,
                    format!("key-{id:05}").into_bytes(),
                    tombstone_value(),
                )
            });
            vault.write_cf_batch(rows).expect("write tombstone batch");
            next = upper;
        }
    }

    fn measure_read_p99_ns(vault: &AsterVault<SharedClock>, start: u64, end: u64) -> u128 {
        let snapshot = vault.latest_seq();
        let mut samples = Vec::new();
        for id in start..end {
            let key = format!("key-{id:05}");
            let started = Instant::now();
            let _ = vault
                .read_cf_at(snapshot, ColumnFamily::Base, key.as_bytes())
                .expect("serving read");
            samples.push(started.elapsed().as_nanos().max(1));
        }
        samples.sort_unstable();
        samples[(samples.len() * 99 / 100).min(samples.len() - 1)]
    }

    fn open_vault(
        vault_dir: &Path,
        clock: SharedClock,
        encryption_key: &[u8],
    ) -> AsterVault<SharedClock> {
        fs::create_dir_all(vault_dir).expect("create vault dir");
        AsterVault::new_durable_with_clock(
            vault_dir,
            vault_id(),
            encryption_key.to_vec(),
            VaultOptions {
                memtable_byte_cap: FSV_MEMTABLE_BYTES,
                ..VaultOptions::default()
            },
            clock,
        )
        .expect("open durable vault")
    }

    fn known_value(seq: u64, len: usize) -> Vec<u8> {
        let mut value = format!("ph58-v{seq:05}:").into_bytes();
        value.resize(len, b'x');
        value
    }

    fn status(vault: &AsterVault<SharedClock>, vault_dir: &Path) -> ResourceStatus {
        vault.resource_status(vault_dir, vram()).expect("status")
    }

    fn vram() -> VramBudgetStatus {
        VramBudgetStatus {
            budget_bytes: 0,
            used_bytes: 0,
            probe_warning: None,
        }
    }

    fn inventory_json(inventory: &calyx_aster::gc::TombstoneInventory) -> Value {
        json!({
            "tombstone_keys": inventory.tombstone_keys(),
            "live_keys": inventory.live_keys(),
            "tombstone_ratio": inventory.tombstone_ratio(),
            "total_sst_bytes": inventory.total_sst_bytes(),
            "per_cf": inventory.per_cf.iter().map(|cf| json!({
                "cf": cf.cf_name,
                "sst_files": cf.sst_files,
                "sst_bytes": cf.sst_bytes,
                "live_keys": cf.live_keys,
                "tombstone_keys": cf.tombstone_keys,
                "tombstone_ratio": cf.tombstone_ratio()
            })).collect::<Vec<_>>()
        })
    }

    fn compaction_result_json(result: calyx_aster::gc::CompactionGcResult) -> Value {
        json!({
            "triggered": result.triggered,
            "rate_limited": result.rate_limited,
            "skipped_reason": result.skipped_reason,
            "error_code": result.error_code,
            "tombstone_ratio_before": result.tombstone_ratio_before,
            "tombstone_ratio_after": result.tombstone_ratio_after,
            "bytes_compacted": result.bytes_compacted,
            "bytes_freed": result.bytes_freed,
            "tombstones_removed": result.tombstones_removed,
            "compaction_debt": result.compaction_debt,
            "compacted_cfs": result.compacted_cfs
        })
    }

    fn write_artifact(name: &str, value: &Value) {
        let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-soak-ph58");
        fs::create_dir_all(&root).expect("create FSV root");
        fs::write(root.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
        let target = repo_root().join("target");
        fs::create_dir_all(&target).expect("create target dir");
        fs::write(target.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn tree_bytes(path: &Path) -> u64 {
        if !path.exists() {
            return 0;
        }
        let metadata = fs::symlink_metadata(path).unwrap();
        if metadata.is_file() {
            return metadata.len();
        }
        if metadata.is_dir() {
            return fs::read_dir(path)
                .unwrap()
                .map(|entry| tree_bytes(&entry.unwrap().path()))
                .sum();
        }
        metadata.len()
    }

    fn command_text(command: &str, args: &[&str]) -> String {
        let output = Command::new(command)
            .args(args)
            .output()
            .expect("run command");
        assert!(
            output.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("stdout utf8")
    }

    fn hotpool_df_text() -> String {
        if !Path::new("/hotpool").exists() {
            return "skipped: /hotpool is not mounted on this host".to_string();
        }
        command_text("df", &["-B1", "/hotpool"])
    }

    fn case_root(name: &str) -> PathBuf {
        let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-soak-ph58");
        fs::create_dir_all(&root).expect("create FSV root");
        fs::write(
            root.join("cleanup-tag.txt"),
            b"issue487 PH58 phase FSV synthetic data\n",
        )
        .expect("write cleanup tag");
        root.join(name)
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repo root")
            .to_path_buf()
    }

    fn cargo_bin() -> String {
        std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
    }
}
