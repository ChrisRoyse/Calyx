use super::*;
use crate::cf::{CfRouter, ColumnFamily, base_key, slot_key};
use crate::vault::AsterVault;
use calyx_core::{
    AbsentReason, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId, VaultStore,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

mod allocator;
mod freshness;
mod isolation;
mod read_barrier;
mod router_bridge;
mod snapshot_gc;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn read_pair(cx_id: CxId) -> [CfRead; 2] {
    [
        CfRead::new(ColumnFamily::Base, base_key(cx_id)),
        CfRead::new(ColumnFamily::slot(SlotId::new(0)), slot_key(cx_id)),
    ]
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn sample_constellation(vault_id: VaultId) -> Constellation {
    let cx_id = cx(0x52);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![0.25, 0.5, 0.75],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );
    Constellation {
        cx_id,
        vault_id,
        panel_version: 1,
        created_at: 100,
        input_ref: InputRef {
            hash: [0x52; 32],
            pointer: Some("synthetic://mvcc-router".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [9; 32],
        },
        flags: CxFlags::default(),
    }
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-mvcc-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sst_count(dir: PathBuf) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("sst")
                })
                .count()
        })
        .unwrap_or(0)
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
