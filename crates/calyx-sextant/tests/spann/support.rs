use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_aster::cf::base_key;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SparseEntry,
    VaultId,
};
use calyx_sextant::index::spann::centroids::SpannCentroidIndex;

#[path = "../sextant_support/mod.rs"]
mod sextant_support;
pub(crate) use sextant_support::{cx_usize_be, hex};

pub(crate) fn fsv_roots() -> (PathBuf, Option<PathBuf>) {
    if let Ok(vault) = std::env::var("CALYX_SPANN_FSV_VAULT") {
        let vault = PathBuf::from(vault);
        return (vault.join("idx").join("slot_00.sparse"), Some(vault));
    }
    let root = std::env::var("CALYX_SPANN_FSV_DIR")
        .map(PathBuf::from)
        .expect("set CALYX_SPANN_FSV_DIR or CALYX_SPANN_FSV_VAULT");
    (root, None)
}

pub(crate) fn write_fsv_vault(vault_dir: &PathBuf, rows: &[(u32, Vec<f32>)], cx_map: &[CxId]) {
    std::fs::create_dir_all(vault_dir).expect("create FSV vault dir");
    let vault = AsterVault::open(
        vault_dir,
        fsv_vault_id(),
        b"issue547-spann-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open FSV vault");
    for chunk in rows.chunks(1000) {
        let batch = chunk
            .iter()
            .map(|(id, vector)| fsv_constellation(*id, vector, cx_map[*id as usize]))
            .collect::<Vec<_>>();
        vault.put_batch(batch).expect("write FSV batch");
    }
    vault.flush().expect("flush FSV vault");
}

fn fsv_constellation(local_id: u32, vector: &[f32], cx_id: CxId) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), sparse_from_dense(vector));
    let input = format!("synthetic://issue547-spann/{local_id}");
    let mut metadata = BTreeMap::new();
    metadata.insert("fsv_issue".to_string(), "547".to_string());
    metadata.insert("local_id".to_string(), local_id.to_string());
    Constellation {
        cx_id,
        vault_id: fsv_vault_id(),
        panel_version: 547,
        created_at: 1_786_000_000 + u64::from(local_id),
        input_ref: InputRef {
            hash: *blake3::hash(input.as_bytes()).as_bytes(),
            pointer: Some(input),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn sparse_from_dense(vector: &[f32]) -> SlotVector {
    SlotVector::Sparse {
        dim: vector.len() as u32,
        entries: vector
            .iter()
            .enumerate()
            .map(|(idx, val)| SparseEntry {
                idx: idx as u32,
                val: *val,
            })
            .collect(),
    }
}

pub(crate) fn fsv_cx_map(centroids: &SpannCentroidIndex, cx_map: &[CxId]) -> String {
    let mut rows = vec!["local_id,cx_id,base_key_hex,centroid_id".to_string()];
    rows.extend(
        centroids
            .assignments()
            .iter()
            .map(|(local_id, centroid_id)| {
                let cx_id = cx_map[*local_id as usize];
                format!(
                    "{local_id},{cx_id},{},{}",
                    hex(&base_key(cx_id)),
                    centroid_id
                )
            }),
    );
    rows.join("\n")
}

fn fsv_vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

pub(crate) fn dir_listing(dir: &Path) -> String {
    let mut rows = std::fs::read_dir(dir)
        .expect("read edge dir")
        .map(|entry| {
            let entry = entry.expect("read edge entry");
            let size = entry.metadata().expect("edge metadata").len();
            format!("{} {size} bytes", entry.file_name().to_string_lossy())
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.join("\n")
}

pub(crate) fn file_state(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read edge file");
    format!("size={} blake3={}\n", bytes.len(), blake3::hash(&bytes))
}

pub(crate) fn first_bytes(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read edge bytes");
    hex(&bytes[..16.min(bytes.len())])
}

pub(crate) fn sparse(entries: &[(u32, f32)], dim: u32) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}
