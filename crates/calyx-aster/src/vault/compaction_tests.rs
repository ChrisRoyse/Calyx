use super::{AsterVault, VaultOptions};
use crate::cf::{ColumnFamily, base_key, slot_key};
use crate::compaction::{CompactionResult, CompactionSchedulerOptions};
use calyx_core::{SlotId, VaultStore};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

mod support;

use support::*;

#[test]
fn durable_vault_flushes_router_ssts_alongside_manifest_checkpoint() {
    let dir = test_dir("router-flush");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let cx = sample_constellation(0x41);
    let id = cx.cx_id;

    vault.put(cx.clone()).unwrap();
    let summaries = vault.flush_all_cfs().unwrap();
    vault.flush().unwrap();
    let base_dir = dir.join("cf/base");
    let base_names = sst_names(&base_dir);
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();

    // Router flushes carry their commit watermark in the file name (issue
    // #1138): `flush-{watermark:020}-{ordinal:04}.sst` with a nonzero
    // watermark on a durable vault.
    let is_commit_anchored_flush = |name: &str| {
        matches!(
            crate::storage_names::classify_sst(Path::new(name)),
            Ok(Some(crate::storage_names::SstName::Flush { watermark, ordinal: 1 }))
                if watermark > 0
        )
    };
    assert!(summaries.iter().any(|summary| {
        summary.path.parent() == Some(base_dir.as_path())
            && is_commit_anchored_flush(summary.path.file_name().unwrap().to_str().unwrap())
    }));
    assert!(
        base_names.iter().any(|name| is_commit_anchored_flush(name)),
        "{base_names:?}"
    );
    // Durable group-commit batch SSTs land alongside the flush.
    assert!(
        base_names.iter().any(|name| matches!(
            crate::storage_names::classify_sst(Path::new(name)),
            Ok(Some(crate::storage_names::SstName::DurableBatch { .. }))
        )),
        "{base_names:?}"
    );
    assert_recovered_matches(cx, reopened.get(id, reopened.snapshot()).unwrap());
    cleanup(dir);
}

#[test]
fn vault_compaction_scheduler_compacts_flushed_cf_catalog() {
    let dir = test_dir("scheduler");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let cx = sample_constellation(0x52);
    let id = cx.cx_id;

    vault.put(cx.clone()).unwrap();
    vault.flush().unwrap();
    let catalog = vault.compaction_catalog().unwrap().unwrap();
    assert!(catalog.shard_count_for_cf(ColumnFamily::Base) > 1);

    let options = CompactionSchedulerOptions {
        interval_ms: 1,
        debt_trigger_score_milli: 0,
        output_root: dir.join("cf"),
        ..CompactionSchedulerOptions::default()
    };
    let scheduler = vault.start_compaction_scheduler(options).unwrap().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while scheduler.shard_count_for_cf(ColumnFamily::Base) != 1 {
        assert!(
            Instant::now() < deadline,
            "vault scheduler did not compact before deadline"
        );
        std::thread::yield_now();
    }
    scheduler.stop().unwrap();
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();

    // Issue #1137 regression: the scheduler output must be named in the
    // commit domain (an adoption slot at the max input commit seq, covered by
    // the manifest), never a run-counter `compacted-{run_id}` name that
    // full-restore readback would restore at commit seq ~1.
    let durable_seq = crate::manifest::ManifestStore::open(&dir)
        .load_current()
        .unwrap()
        .durable_seq;
    let base_names = sst_names(&dir.join("cf/base"));
    assert!(
        base_names
            .iter()
            .any(|name| name == &format!("{durable_seq:020}-9999.sst")),
        "expected adoption slot at durable_seq {durable_seq}: {base_names:?}"
    );
    assert!(
        !base_names.iter().any(|name| name.starts_with("compacted-")),
        "run-counter compacted name leaked into the vault CF dir: {base_names:?}"
    );
    assert_recovered_matches(cx, reopened.get(id, reopened.snapshot()).unwrap());
    cleanup(dir);
}

/// Issue #1137 regression, end to end: background-scheduler compaction
/// outputs must never rewrite commit history. Pre-fix, the scheduler named
/// its first output `compacted-{run_id=1}` inside the vault CF dir, so
/// full-restore readback restored the merged latest state at commit seq 1
/// and a historical read pinned at seq 1 saw rows committed at seq 2.
#[test]
fn vault_scheduler_compaction_preserves_commit_history() {
    let dir = test_dir("scheduler-history");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    vault
        .write_cf_batch([(ColumnFamily::Kv, b"k1".to_vec(), b"v1".to_vec())])
        .unwrap(); // commit seq 1
    vault
        .write_cf_batch([(ColumnFamily::Kv, b"k2".to_vec(), b"v2".to_vec())])
        .unwrap(); // commit seq 2
    vault.flush().unwrap();

    let options = CompactionSchedulerOptions {
        interval_ms: 1,
        debt_trigger_score_milli: 0,
        output_root: dir.join("cf"),
        ..CompactionSchedulerOptions::default()
    };
    let scheduler = vault.start_compaction_scheduler(options).unwrap().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while scheduler.shard_count_for_cf(ColumnFamily::Kv) != 1 {
        assert!(
            Instant::now() < deadline,
            "vault scheduler did not compact kv before deadline"
        );
        std::thread::yield_now();
    }
    scheduler.stop().unwrap();
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    assert_eq!(
        reopened.read_cf_at(1, ColumnFamily::Kv, b"k2").unwrap(),
        None,
        "row committed at seq 2 is visible at seq 1: the scheduler output was restored at the \
         wrong commit seq (issue #1137)"
    );
    assert_eq!(
        reopened.read_cf_at(2, ColumnFamily::Kv, b"k2").unwrap(),
        Some(b"v2".to_vec())
    );
    assert_eq!(
        reopened
            .read_cf_at(reopened.snapshot(), ColumnFamily::Kv, b"k1")
            .unwrap(),
        Some(b"v1".to_vec())
    );
    drop(reopened);
    cleanup(dir);
}

#[test]
fn compacted_ssts_recover_after_original_shards_are_absent() {
    let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("compacted-recovery"),
        |root| {
            let dir = root.join("compacted-recovery").join("vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        },
    );
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let cx = sample_constellation(0x54);
    let id = cx.cx_id;
    let base_dir = dir.join("cf/base");
    let slot_dir = dir.join("cf/slot_00");

    vault.put(cx.clone()).unwrap();
    vault.flush().unwrap();
    vault
        .compact_cf_once(ColumnFamily::Base)
        .unwrap()
        .expect("base compacted");
    vault
        .compact_cf_once(ColumnFamily::slot(SlotId::new(0)))
        .unwrap()
        .expect("slot compacted");
    let base_before_removal = sst_names(&base_dir);
    let slot_before_removal = sst_names(&slot_dir);
    remove_non_compacted_ssts(&base_dir);
    remove_non_compacted_ssts(&slot_dir);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let got = reopened.get(id, reopened.snapshot()).unwrap();

    assert_recovered_matches(cx, got.clone());
    if let Some(root) = fsv_root {
        write_compacted_recovery_readback(
            &root,
            &dir,
            &base_before_removal,
            &slot_before_removal,
            reopened.snapshot(),
            &got,
        );
    } else {
        cleanup(dir);
    }
}

#[test]
fn tiered_vault_flush_recovery_and_compaction_use_hot_archive_roots() {
    let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("tiered-vault"),
        |root| {
            let dir = root.join("tiered-vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        },
    );
    let vault_root = dir.join("vault");
    let hot = dir.join("hot");
    let archive = dir.join("archive");
    let options = tiered_options(&hot, &archive);
    let vault = AsterVault::new_durable(&vault_root, vault_id(), b"salt", options.clone()).unwrap();
    let mut first = sample_constellation(0x61);
    let mut second = sample_constellation(0x62);
    add_inactive_slot(&mut first, 0x11);
    add_inactive_slot(&mut second, 0x22);
    let first_id = first.cx_id;
    let second_id = second.cx_id;

    vault.put(first.clone()).unwrap();
    vault.put(second.clone()).unwrap();
    vault.flush().unwrap();

    let manifest_bytes = fs::read(vault_root.join("CURRENT")).unwrap();
    let hot_base = sst_names(&hot.join("cf/base"));
    let hot_active = sst_names(&hot.join("cf/slot_00"));
    let cold_inactive = sst_names(&archive.join("cf/slot_01"));
    let misplaced_cold = maybe_sst_names(&vault_root.join("cf/slot_01"));
    let catalog = vault.compaction_catalog().unwrap().unwrap();

    assert!(!manifest_bytes.is_empty());
    assert!(!hot_base.is_empty());
    assert!(!hot_active.is_empty());
    assert!(cold_inactive.iter().any(|name| name.contains('-')));
    assert!(misplaced_cold.is_empty());
    assert!(catalog.shard_count_for_cf(ColumnFamily::slot(SlotId::new(1))) >= 2);

    let compacted = vault
        .compact_cf_once(ColumnFamily::slot(SlotId::new(1)))
        .unwrap()
        .unwrap();
    let CompactionResult::Compacted(report) = compacted else {
        panic!("expected inactive slot compaction");
    };
    assert!(report.output_path.starts_with(archive.join("cf/slot_01")));
    assert!(
        sst_names(&archive.join("cf/slot_01"))
            .iter()
            .any(|name| name.starts_with("compacted-"))
    );

    let reopened = AsterVault::open(&vault_root, vault_id(), b"salt", options).unwrap();
    assert_recovered_matches(first, reopened.get(first_id, reopened.snapshot()).unwrap());
    assert_recovered_matches(
        second,
        reopened.get(second_id, reopened.snapshot()).unwrap(),
    );
    if let Some(root) = fsv_root {
        write_tiered_readback(
            &root,
            &vault_root,
            &hot,
            &archive,
            &report.output_path,
            &manifest_bytes,
        );
    } else {
        cleanup(dir);
    }
}

#[test]
fn selected_cf_open_uses_tiering_policy_for_archive_readback() {
    let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("selected-tiered-vault"),
        |root| {
            let dir = root.join("selected-tiered-vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        },
    );
    let vault_root = dir.join("vault");
    let hot = dir.join("hot");
    let archive = dir.join("archive");
    let options = tiered_options(&hot, &archive);
    let vault = AsterVault::new_durable(&vault_root, vault_id(), b"salt", options.clone()).unwrap();
    let mut cx = sample_constellation(0x71);
    add_inactive_slot(&mut cx, 0x33);
    let cx_id = cx.cx_id;

    vault.put(cx).unwrap();
    vault.flush().unwrap();
    drop(vault);

    let selected_options = VaultOptions {
        read_only: true,
        restore_mvcc_rows: false,
        restore_ledger_hook: false,
        selected_cfs: Some(vec![ColumnFamily::Base, ColumnFamily::slot(SlotId::new(1))]),
        tiering_policy: options.tiering_policy.clone(),
        ..VaultOptions::default()
    };
    let selected = AsterVault::open(&vault_root, vault_id(), b"salt", selected_options).unwrap();
    let snapshot = selected.snapshot();
    let base = selected
        .read_cf_at(snapshot, ColumnFamily::Base, &base_key(cx_id))
        .unwrap();
    let inactive_slot = selected
        .read_cf_at(
            snapshot,
            ColumnFamily::slot(SlotId::new(1)),
            &slot_key(cx_id),
        )
        .unwrap();

    assert!(base.is_some());
    assert!(inactive_slot.is_some());
    assert!(!sst_names(&archive.join("cf/slot_01")).is_empty());
    assert!(maybe_sst_names(&vault_root.join("cf/slot_01")).is_empty());
    if let Some(root) = fsv_root {
        let readback = serde_json::json!({
            "vault_root": vault_root,
            "hot_root": hot,
            "archive_root": archive,
            "selected_cfs": ["base", "slot_01"],
            "snapshot": snapshot,
            "base_present": base.is_some(),
            "inactive_slot_present": inactive_slot.is_some(),
            "archive_slot_ssts": sst_names(&archive.join("cf/slot_01")),
            "vault_slot_ssts": maybe_sst_names(&vault_root.join("cf/slot_01")),
        });
        fs::write(
            root.join("selected-tiered-cf-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        cleanup(dir);
    }
}
