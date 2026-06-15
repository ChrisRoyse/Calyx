use super::collect::{collect_compaction, collect_wal};
use super::heap::parse_vm_rss_bytes;
use super::*;
use crate::cf::{CfRouter, ColumnFamily};
use crate::gc::GcMetrics;
use crate::mvcc::{Freshness, ReaderLease, VersionedCfStore};
use crate::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("calyx-resource-{name}-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn parse_vm_rss_reads_kernel_status_format() {
    let text = "Name:\tcalyx\nVmPeak:\t  104860 kB\nVmRSS:\t   51200 kB\nThreads:\t8\n";
    assert_eq!(parse_vm_rss_bytes(text).unwrap(), 51_200 * 1024);
}

#[test]
fn parse_vm_rss_fails_closed_on_unknown_unit() {
    let error = parse_vm_rss_bytes("VmRSS:\t  12 mB\n").unwrap_err();
    assert_eq!(error.code, CALYX_RESOURCE_PROBE_UNAVAILABLE);
}

#[test]
fn parse_vm_rss_fails_closed_when_line_missing() {
    let error = parse_vm_rss_bytes("Name:\tcalyx\n").unwrap_err();
    assert_eq!(error.code, CALYX_RESOURCE_PROBE_UNAVAILABLE);
}

#[test]
fn router_counts_absorbed_memtable_backpressure_and_flushes_real_ssts() {
    let dir = test_dir("absorb");
    // Cap 100: each 60-byte row fits alone; the second projected put (120 > 100)
    // fires CALYX_BACKPRESSURE, which the router absorbs with an SST flush.
    let mut router = CfRouter::open(&dir, 100).unwrap();
    let counters = router.resource_counters();
    for op in 0u64..5 {
        router
            .put(ColumnFamily::Base, &op.to_be_bytes(), &[0xAB; 52])
            .unwrap();
    }

    let status = counters.snapshot();
    assert_eq!(status.memtable_absorbed_total, 4);
    assert_eq!(status.memtable_rejected_total, 0);
    assert_eq!(status.disk_pressure_events_total, 0);
    assert_eq!(status.events_total, 4);
    // Physical evidence: each absorbed event produced one SST flush on disk.
    assert_eq!(router.level_file_count(ColumnFamily::Base), 4);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn router_counts_rejected_row_larger_than_cap() {
    let dir = test_dir("reject");
    let mut router = CfRouter::open(&dir, 32).unwrap();
    let counters = router.resource_counters();
    let error = router
        .put(ColumnFamily::Base, b"k", &[0xCD; 64])
        .unwrap_err();

    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    let status = counters.snapshot();
    assert_eq!(status.memtable_absorbed_total, 0);
    assert_eq!(status.memtable_rejected_total, 1);
    assert_eq!(status.disk_pressure_events_total, 0);
    assert_eq!(status.events_total, 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lease_registry_tracks_live_releases_and_expires() {
    let registry = LeaseRegistry::default();
    registry.register(ReaderLease::new(1, 10, 1_000, 500));
    registry.register(ReaderLease::new(2, 14, 1_000, 5_000));

    let both_live = registry.live_view(1_400);
    assert_eq!(both_live.active_leases, 2);
    assert_eq!(both_live.oldest_pinned_seq, Some(10));

    // Lease 1 expires at 1_500; expiry is the bound, not a fallback.
    let one_expired = registry.live_view(1_500);
    assert_eq!(one_expired.active_leases, 1);
    assert_eq!(one_expired.oldest_pinned_seq, Some(14));

    assert!(registry.release(2));
    assert!(!registry.release(2));
    let drained = registry.live_view(1_500);
    assert_eq!(drained.active_leases, 0);
    assert_eq!(drained.oldest_pinned_seq, None);
}

#[test]
fn store_pin_snapshot_registers_lease_and_gap_follows_commits() {
    let clock = FixedClock::new(50_000);
    let store = VersionedCfStore::new(7);
    let snapshot = store.pin_snapshot(Freshness::FreshDerived, &clock, 60_000);
    assert_eq!(snapshot.lease().pinned_seq(), 7);

    for _ in 0..3 {
        store
            .commit_batch([(ColumnFamily::Base, b"k".to_vec(), b"v".to_vec())])
            .unwrap();
    }

    let view = store.lease_view(50_001);
    assert_eq!(view.active_leases, 1);
    assert_eq!(view.oldest_pinned_seq, Some(7));
    assert_eq!(store.current_seq() - view.oldest_pinned_seq.unwrap(), 3);

    assert!(store.release_lease(snapshot.lease().id()));
    let released = store.lease_view(50_001);
    assert_eq!(released.active_leases, 0);
}

#[test]
fn vault_pin_reader_gap_matches_hand_computed_write_count() {
    let dir = test_dir("vault-gap");
    let vault = AsterVault::open(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"resource-tests".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let pinned = vault.pin_reader(Freshness::FreshDerived, 60_000);
    let pinned_seq = pinned.lease().pinned_seq();

    for op in 0u64..4 {
        vault
            .write_cf(
                ColumnFamily::Base,
                op.to_be_bytes().to_vec(),
                vec![0xEF; 16],
            )
            .unwrap();
    }

    // 2+2=4 discipline: 4 single-row commits move the seq exactly 4 past the pin.
    assert_eq!(vault.latest_seq(), pinned_seq + 4);
    assert!(vault.release_reader(pinned.lease().id()));
    assert!(!vault.release_reader(pinned.lease().id()));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wal_section_matches_filesystem_byte_for_byte() {
    let dir = test_dir("wal-bytes");
    let vault = AsterVault::open(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"resource-tests".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    vault
        .write_cf(ColumnFamily::Base, b"key".to_vec(), vec![0x11; 64])
        .unwrap();
    drop(vault);

    let wal = collect_wal(&dir).unwrap();
    // Independent SoT read: re-stat the segment files directly.
    let mut expected_bytes = 0;
    let mut expected_count = 0;
    for entry in fs::read_dir(dir.join("wal")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) == Some("wal") {
            expected_count += 1;
            expected_bytes += fs::metadata(&path).unwrap().len();
        }
    }
    assert!(wal.bytes > 0);
    assert_eq!(wal.segment_count, expected_count);
    assert_eq!(wal.bytes, expected_bytes);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compaction_section_matches_sst_file_sizes() {
    let dir = test_dir("debt");
    let mut router = CfRouter::open(&dir, 1024).unwrap();
    router.put(ColumnFamily::Base, b"a", &[0x22; 100]).unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    router.put(ColumnFamily::Base, b"b", &[0x33; 100]).unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    drop(router);

    let compaction = collect_compaction(&dir).unwrap();
    let base = compaction
        .per_cf
        .iter()
        .find(|cf| cf.cf == "base")
        .expect("base CF debt present");
    let disk_bytes: u64 = fs::read_dir(dir.join("cf/base"))
        .unwrap()
        .map(|entry| fs::metadata(entry.unwrap().path()).unwrap().len())
        .sum();
    assert_eq!(base.sst_files, 2);
    assert_eq!(base.pending_bytes, disk_bytes);
    assert_eq!(
        base.score_milli,
        disk_bytes * 1_000 / compaction.target_bytes.max(1)
    );
    assert_eq!(compaction.total_pending_bytes, disk_bytes);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn metrics_text_renders_prometheus_conventions() {
    let status = ResourceStatus {
        schema_version: RESOURCE_STATUS_SCHEMA_VERSION,
        vault_dir: "/tmp/v".to_string(),
        collected_at: 1,
        heap: HeapStatus { rss_bytes: 2048 },
        memtable: MemtableStatus {
            total_used_bytes: 64,
            total_cap_bytes: 1024,
            per_cf: vec![MemtableCfStatus {
                cf: "base".to_string(),
                used_bytes: 64,
                cap_bytes: 1024,
                high_water_bytes: 819,
                flush_triggered: false,
            }],
        },
        vram: VramBudgetStatus {
            budget_bytes: 512,
            used_bytes: 128,
            probe_warning: None,
        },
        compaction: CompactionDebtStatus {
            target_bytes: 1000,
            total_pending_bytes: 300,
            max_score_milli: 300,
            per_cf: vec![CfCompactionDebt {
                cf: "base".to_string(),
                sst_files: 1,
                pending_bytes: 300,
                score_milli: 300,
            }],
        },
        gc: GcMetrics {
            versions_reclaimed_total: 7,
            bytes_freed_total: 2048,
            soft_deletes_purged_total: 0,
            compaction_debt: 11,
        },
        pinned: PinnedSeqStatus {
            current_seq: 9,
            oldest_pinned_seq: Some(5),
            oldest_pinned_seq_gap: 4,
            active_leases: 1,
            reader_lease_expired_total: 2,
        },
        backpressure: BackpressureStatus {
            memtable_absorbed_total: 3,
            memtable_rejected_total: 1,
            disk_pressure_events_total: 2,
            events_total: 4,
        },
        wal: WalStatus {
            segment_count: 2,
            bytes: 640,
        },
    };

    let text = status.to_metrics_text("demo");
    assert!(text.contains("calyx_heap_rss_bytes{vault=\"demo\"} 2048"));
    assert!(text.contains("calyx_memtable_used_bytes{vault=\"demo\",cf=\"base\"} 64"));
    assert!(text.contains("calyx_memtable_cap_bytes{vault=\"demo\",cf=\"base\"} 1024"));
    assert!(
        text.contains("calyx_compaction_pending_compaction_bytes{vault=\"demo\",cf=\"base\"} 300")
    );
    assert!(text.contains("calyx_gc_versions_reclaimed_total{vault=\"demo\"} 7"));
    assert!(text.contains("calyx_gc_bytes_freed_total{vault=\"demo\"} 2048"));
    assert!(text.contains("calyx_gc_soft_deletes_purged_total{vault=\"demo\"} 0"));
    assert!(text.contains("calyx_compaction_debt{vault=\"demo\"} 11"));
    assert!(text.contains("calyx_oldest_pinned_seq_gap{vault=\"demo\"} 4"));
    assert!(text.contains("calyx_reader_lease_expired_total{vault=\"demo\"} 2"));
    assert!(text.contains(
        "calyx_backpressure_events_total{vault=\"demo\",source=\"memtable_absorbed\"} 3"
    ));
    assert!(text.contains("calyx_disk_pressure_events_total{vault=\"demo\"} 2"));
    assert!(text.contains("calyx_wal_bytes{vault=\"demo\"} 640"));
    assert!(text.contains("calyx_wal_bytes_active{vault=\"demo\"} 640"));
}

#[cfg(target_os = "linux")]
#[test]
fn full_collect_reads_live_heap_rss_on_linux() {
    let dir = test_dir("collect-linux");
    let vault = AsterVault::open(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"resource-tests".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let status = vault
        .resource_status(
            &dir,
            VramBudgetStatus {
                budget_bytes: 0,
                used_bytes: 0,
                probe_warning: None,
            },
        )
        .unwrap();
    assert!(status.heap.rss_bytes > 0);
    assert_eq!(status.schema_version, RESOURCE_STATUS_SCHEMA_VERSION);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(not(target_os = "linux"))]
#[test]
fn full_collect_fails_closed_without_proc() {
    let dir = test_dir("collect-closed");
    let vault = AsterVault::open(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"resource-tests".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let error = vault
        .resource_status(
            &dir,
            VramBudgetStatus {
                budget_bytes: 0,
                used_bytes: 0,
                probe_warning: None,
            },
        )
        .unwrap_err();
    assert_eq!(error.code, CALYX_RESOURCE_PROBE_UNAVAILABLE);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn collect_rejects_non_vault_directory() {
    let dir = test_dir("not-a-vault");
    let store = VersionedCfStore::new(0);
    let error = collect_resource_status(
        &dir,
        VramBudgetStatus {
            budget_bytes: 0,
            used_bytes: 0,
            probe_warning: None,
        },
        &store,
        1,
    )
    .unwrap_err();
    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    fs::remove_dir_all(dir).unwrap();
}
