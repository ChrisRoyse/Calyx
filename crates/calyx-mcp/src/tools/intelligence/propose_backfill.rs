use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    CalyxError, Clock, Constellation, CxId, Input, LensId, Modality, Result, SlotId, SlotVector,
    VaultStore,
};
use serde::Serialize;

const VAULT_POINTER_PREFIX: &str = "calyx-vault://";

#[derive(Clone, Debug)]
pub(super) struct CandidateBackfill {
    pub(super) slot_id: SlotId,
    pub(super) lens_id: LensId,
    pub(super) bits: f64,
    pub(super) vectors: BTreeMap<CxId, SlotVector>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct BackfillWriteReport {
    pub slot_id: u16,
    pub lens_id: String,
    pub rows_written: usize,
    pub seq: u64,
    pub candidate_bits: f64,
}

pub(super) struct BackfillUndo {
    slot_id: SlotId,
    base_rows: Vec<(CxId, Vec<u8>)>,
}

pub(super) fn input_for_constellation(
    vault_dir: &Path,
    cx: &Constellation,
    modality: Modality,
) -> Result<Input> {
    if cx.input_ref.redacted {
        return Err(input_unavailable(format!(
            "input bytes for {} are redacted",
            cx.cx_id
        )));
    }
    if let Some(value) = cx
        .metadata
        .get("calyx.input_utf8")
        .or_else(|| cx.metadata.get("raw_input"))
        .or_else(|| cx.metadata.get("input_text"))
    {
        return Ok(Input::new(modality, value.as_bytes().to_vec()));
    }
    let Some(pointer) = cx.input_ref.pointer.as_deref() else {
        return Err(input_unavailable(format!(
            "input bytes for {} have no retained pointer",
            cx.cx_id
        )));
    };
    let bytes = read_pointer(vault_dir, pointer)?;
    Ok(Input::new(modality, bytes).with_pointer(pointer.to_string()))
}

pub(super) fn apply_slot_backfill<C: Clock>(
    vault: &AsterVault<C>,
    docs: &BTreeMap<CxId, Constellation>,
    backfill: &CandidateBackfill,
) -> Result<(BackfillWriteReport, BackfillUndo)> {
    let mut rows = Vec::with_capacity(docs.len() * 2);
    let mut undo = Vec::with_capacity(docs.len());
    for (cx_id, cx) in docs {
        let vector = backfill.vectors.get(cx_id).ok_or_else(|| {
            backfill_error(format!(
                "candidate vector for constellation {cx_id} is missing"
            ))
        })?;
        let base_key = base_key(*cx_id);
        let prior = vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key)?
            .ok_or_else(|| backfill_error(format!("base row for {cx_id} is missing")))?;
        let mut updated = cx.clone();
        updated.slots.insert(backfill.slot_id, vector.clone());
        rows.push((
            ColumnFamily::Base,
            base_key,
            encode::encode_constellation_base(&updated)?,
        ));
        rows.push((
            ColumnFamily::slot(backfill.slot_id),
            slot_key(*cx_id),
            encode::encode_slot_vector(vector)?,
        ));
        undo.push((*cx_id, prior));
    }
    let seq = vault.write_cf_batch(rows)?;
    vault.flush()?;
    Ok((
        BackfillWriteReport {
            slot_id: backfill.slot_id.get(),
            lens_id: backfill.lens_id.to_string(),
            rows_written: docs.len(),
            seq,
            candidate_bits: backfill.bits,
        },
        BackfillUndo {
            slot_id: backfill.slot_id,
            base_rows: undo,
        },
    ))
}

pub(super) fn restore_slot_backfill<C: Clock>(
    vault: &AsterVault<C>,
    undo: BackfillUndo,
) -> Result<()> {
    let mut rows = Vec::with_capacity(undo.base_rows.len() * 2);
    for (cx_id, bytes) in undo.base_rows {
        rows.push((ColumnFamily::Base, base_key(cx_id), bytes));
        rows.push((
            ColumnFamily::slot(undo.slot_id),
            slot_key(cx_id),
            tombstone_value(),
        ));
    }
    vault.write_cf_batch(rows)?;
    vault.flush()?;
    Ok(())
}

fn read_pointer(vault_dir: &Path, pointer: &str) -> Result<Vec<u8>> {
    if let Some(relative) = pointer.strip_prefix(VAULT_POINTER_PREFIX) {
        return fs::read(vault_relative_path(vault_dir, relative)?)
            .map_err(|error| input_unavailable(format!("read vault input pointer: {error}")));
    }
    if let Some(path) = pointer.strip_prefix("file://") {
        return fs::read(path)
            .map_err(|error| input_unavailable(format!("read file input pointer: {error}")));
    }
    let path = Path::new(pointer);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        vault_relative_path(vault_dir, pointer)?
    };
    fs::read(resolved).map_err(|error| input_unavailable(format!("read input pointer: {error}")))
}

fn vault_relative_path(vault_dir: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(input_unavailable(format!(
            "vault input pointer {relative:?} escapes the vault"
        )));
    }
    Ok(vault_dir.join(path))
}

fn input_unavailable(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_PROPOSE_INPUT_UNAVAILABLE",
        message: message.into(),
        remediation: "retain source input bytes or a readable input_ref.pointer before proposing a backfilled lens",
    }
}

fn backfill_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_PROPOSE_BACKFILL_FAILED",
        message: message.into(),
        remediation: "repair proposal backfill state before admitting the lens",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_aster::cf::full_content_hash;
    use calyx_core::{CxFlags, FixedClock, InputRef, LedgerRef, VaultId, VaultStore};

    #[test]
    fn restore_tombstones_backfilled_slot_rows() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(99));
        let cx = constellation(&vault, b"rollback-input");
        let cx_id = cx.cx_id;
        vault.put(cx.clone()).expect("write base constellation");
        let before_base = vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))
            .expect("read base before")
            .expect("base before");

        let mut docs = BTreeMap::new();
        docs.insert(cx_id, cx);
        let slot_id = SlotId::new(9);
        let mut vectors = BTreeMap::new();
        vectors.insert(
            cx_id,
            SlotVector::Dense {
                dim: 1,
                data: vec![0.75],
            },
        );
        let backfill = CandidateBackfill {
            slot_id,
            lens_id: LensId::from_bytes([7; 16]),
            bits: 0.75,
            vectors,
        };

        let (_, undo) = apply_slot_backfill(&vault, &docs, &backfill).expect("apply backfill");
        assert!(
            vault
                .read_cf_at(
                    vault.snapshot(),
                    ColumnFamily::slot(slot_id),
                    &slot_key(cx_id)
                )
                .expect("read slot after apply")
                .is_some()
        );

        restore_slot_backfill(&vault, undo).expect("restore backfill");
        assert_eq!(
            vault
                .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))
                .expect("read base after")
                .expect("base after"),
            before_base
        );
        assert!(
            vault
                .read_cf_at(
                    vault.snapshot(),
                    ColumnFamily::slot(slot_id),
                    &slot_key(cx_id)
                )
                .expect("read slot after restore")
                .is_none()
        );
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
    }

    fn constellation(vault: &AsterVault<FixedClock>, input: &[u8]) -> calyx_core::Constellation {
        calyx_core::Constellation {
            cx_id: vault.cx_id_for_input(input, 1),
            vault_id: vault_id(),
            panel_version: 1,
            created_at: 99,
            input_ref: InputRef {
                hash: full_content_hash([input]),
                pointer: Some("calyx-vault://inputs/rollback.bin".to_string()),
                redacted: false,
            },
            modality: Modality::Text,
            slots: BTreeMap::new(),
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
}
