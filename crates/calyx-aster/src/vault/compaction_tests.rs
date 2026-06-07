use super::{AsterVault, VaultOptions};
use crate::cf::ColumnFamily;
use crate::compaction::CompactionSchedulerOptions;
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

    assert!(summaries.iter().any(|summary| {
        summary.path.parent() == Some(base_dir.as_path())
            && summary.path.file_name().unwrap() == "00000000000000000001.sst"
    }));
    assert!(
        base_names
            .iter()
            .any(|name| name == "00000000000000000001.sst")
    );
    assert!(base_names.iter().any(|name| name.contains("-0000.sst")));
    assert_eq!(reopened.get(id, reopened.snapshot()).unwrap(), cx);
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

    assert!(
        sst_names(&dir.join("cf/base"))
            .iter()
            .any(|name| { name.starts_with("compacted-") && name.ends_with(".sst") })
    );
    assert_eq!(reopened.get(id, reopened.snapshot()).unwrap(), cx);
    cleanup(dir);
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
