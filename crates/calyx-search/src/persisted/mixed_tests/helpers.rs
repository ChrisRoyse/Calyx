use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SparseEntry,
    VaultId,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::super::SearchIndexEntry;
use crate::error::CliError;

pub(super) fn mixed_docs() -> BTreeMap<CxId, Constellation> {
    [
        constellation(
            cx(1),
            [
                (SlotId::new(0), dense(vec![1.0, 0.0])),
                (SlotId::new(1), sparse(8, [1, 2])),
                (SlotId::new(2), multi(2, [[1.0, 0.0], [0.0, 1.0]])),
            ],
        ),
        constellation(
            cx(2),
            [
                (SlotId::new(0), dense(vec![0.0, 1.0])),
                (SlotId::new(1), sparse(8, [3])),
                (SlotId::new(2), multi(2, [[0.0, 1.0], [0.5, 0.5]])),
            ],
        ),
        constellation(
            cx(3),
            [
                (SlotId::new(0), dense(vec![0.8, 0.2])),
                (SlotId::new(1), sparse(8, [1])),
                (SlotId::new(2), multi(2, [[1.0, 0.0]])),
            ],
        ),
    ]
    .into_iter()
    .map(|cx| (cx.cx_id, cx))
    .collect()
}

pub(super) fn constellation<const N: usize>(
    cx_id: CxId,
    slot_rows: [(SlotId, SlotVector); N],
) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.extend(slot_rows);
    Constellation {
        cx_id,
        vault_id: VaultId::from_ulid(Ulid::from_bytes([9; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [0; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }
}

pub(super) fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

pub(super) fn sparse<const N: usize>(dim: u32, terms: [u32; N]) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: terms
            .into_iter()
            .map(|idx| SparseEntry { idx, val: 1.0 })
            .collect(),
    }
}

pub(super) fn multi<const N: usize, const D: usize>(
    token_dim: u32,
    tokens: [[f32; D]; N],
) -> SlotVector {
    SlotVector::Multi {
        token_dim,
        tokens: tokens.into_iter().map(Vec::from).collect(),
    }
}

pub(super) fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub(super) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "calyx-cli-persisted-search-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

pub(super) fn cleanup(root: PathBuf) {
    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}

pub(super) fn sidecar_state(root: &Path, rel: &str) -> Value {
    let path = root.join(rel);
    let bytes = fs::read(&path).unwrap_or_default();
    json!({
        "rel": rel,
        "exists": path.exists(),
        "bytes": bytes.len(),
        "sha256": if bytes.is_empty() { None } else { Some(sha256_hex(&bytes)) },
        "first16_ascii": String::from_utf8_lossy(&bytes[..bytes.len().min(16)]).to_string(),
    })
}

pub(super) fn read_multi_segment_manifest(root: &Path, entry: &SearchIndexEntry) -> Value {
    serde_json::from_slice(&fs::read(root.join(entry.index_rel.as_ref().unwrap())).unwrap())
        .expect("decode multi segment manifest")
}

pub(super) fn first_segment_rel(manifest: &Value) -> String {
    manifest["segments"][0]["index_rel"]
        .as_str()
        .expect("first segment rel")
        .to_string()
}

pub(super) fn error_json(error: &CliError) -> Value {
    json!({
        "code": error.code(),
        "message": error.message(),
    })
}

pub(super) fn maybe_write_fsv_json(name: &str, value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
