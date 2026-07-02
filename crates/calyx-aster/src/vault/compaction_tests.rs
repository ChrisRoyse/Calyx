use super::{AsterVault, VaultOptions};
use crate::cf::ColumnFamily;
use crate::compaction::{CompactionResult, CompactionSchedulerOptions, TieringPolicy};
use calyx_core::{
    CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

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

fn sample_constellation(seed: u8) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1780831800 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue69/{seed:02x}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed.wrapping_add(1); 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn add_inactive_slot(cx: &mut calyx_core::Constellation, seed: u8) {
    cx.slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed) / 255.0, 1.0],
        },
    );
}

fn assert_recovered_matches(
    mut expected: calyx_core::Constellation,
    got: calyx_core::Constellation,
) {
    expected.provenance = got.provenance.clone();
    assert_ne!(got.provenance.hash, [0; 32]);
    assert_eq!(got, expected);
}

fn tiered_options(hot: &Path, archive: &Path) -> VaultOptions {
    VaultOptions {
        tiering_policy: Some(TieringPolicy::new(hot, archive, [SlotId::new(0)], 7)),
        ..VaultOptions::default()
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn sst_names(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".sst"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn maybe_sst_names(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    sst_names(dir)
}

fn remove_non_compacted_ssts(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".sst") && !name.starts_with("compacted-") {
            fs::remove_file(path).unwrap();
        }
    }
}

fn write_tiered_readback(
    root: &Path,
    vault_root: &Path,
    hot: &Path,
    archive: &Path,
    compacted_path: &Path,
    manifest_bytes: &[u8],
) {
    fs::create_dir_all(root).unwrap();
    let readback = serde_json::json!({
        "vault_root": vault_root,
        "hot_root": hot,
        "archive_root": archive,
        "current_manifest": String::from_utf8_lossy(manifest_bytes),
        "hot_base_ssts": sst_names(&hot.join("cf/base")),
        "hot_active_slot_ssts": sst_names(&hot.join("cf/slot_00")),
        "archive_inactive_slot_ssts": sst_names(&archive.join("cf/slot_01")),
        "vault_inactive_slot_ssts": maybe_sst_names(&vault_root.join("cf/slot_01")),
        "compacted_inactive_slot": compacted_path,
    });
    fs::write(
        root.join("tiered-vault-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

fn write_compacted_recovery_readback(
    root: &Path,
    vault_root: &Path,
    base_before_removal: &[String],
    slot_before_removal: &[String],
    cold_open_snapshot: u64,
    got: &calyx_core::Constellation,
) {
    fs::create_dir_all(root).unwrap();
    let current_manifest = fs::read(vault_root.join("CURRENT")).unwrap();
    let readback = serde_json::json!({
        "vault_root": vault_root,
        "current_manifest": String::from_utf8_lossy(&current_manifest),
        "base_ssts_before_removal": base_before_removal,
        "slot_ssts_before_removal": slot_before_removal,
        "base_ssts_after_removal": sst_names(&vault_root.join("cf/base")),
        "slot_ssts_after_removal": sst_names(&vault_root.join("cf/slot_00")),
        "cold_open_snapshot": cold_open_snapshot,
        "cx_id": got.cx_id.to_string(),
        "input_pointer": got.input_ref.pointer.clone(),
        "slot_count": got.slots.len(),
        "provenance_seq": got.provenance.seq,
        "provenance_hash_is_nonzero": got.provenance.hash != [0; 32],
    });
    fs::write(
        root.join("compacted-recovery-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-compaction-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
