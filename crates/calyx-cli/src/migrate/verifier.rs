use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{Result, SlotId, SlotVector, VaultStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::adapter::{
    BASE_SLOT, METADATA_CONTENT_HASH, METADATA_ROWID, VaultSqliteAdapter, default_panel_version,
};
use super::backfill::default_slot_ids;
use super::errors;
use super::manifest::hex_encode;
use super::reader::ChunkRow;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub total: usize,
    pub matched: usize,
    pub base_slot_matches: usize,
    pub backfill_slots_checked: usize,
    pub missing_backfill: Vec<String>,
    pub gate: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub base_rows: usize,
    pub slot_rows: BTreeMap<String, usize>,
    pub latest_seq: u64,
}

pub fn verify_migration(
    vault: &AsterVault,
    rows: &[ChunkRow],
    adapter: &VaultSqliteAdapter,
    require_backfill: bool,
) -> Result<VerifyReport> {
    let mut matched = 0;
    let mut base_slot_matches = 0;
    let mut missing_backfill = Vec::new();
    let snapshot = vault.snapshot();
    for row in rows {
        let cx_id = adapter.cx_id(row);
        let cx = vault.get(cx_id, snapshot)?;
        if cx.input_ref.hash != row.content_hash() {
            return Err(errors::verify_mismatch(format!(
                "{} content hash mismatch",
                row.chunk_id
            )));
        }
        if cx.chunk_id() != Some(row.chunk_id.as_str())
            || cx.database_name() != Some(row.database_name.as_str())
            || cx.metadata.get(METADATA_ROWID) != Some(&row.row_num.to_string())
            || cx.metadata.get(METADATA_CONTENT_HASH) != Some(&hex_encode(&row.content_hash()))
            || cx.panel_version != default_panel_version()
        {
            return Err(errors::verify_mismatch(format!(
                "{} metadata mismatch",
                row.chunk_id
            )));
        }
        matched += 1;
        if slot_matches(
            vault.read_slot_vector_at(snapshot, cx_id, BASE_SLOT)?,
            &row.embedding,
        ) {
            base_slot_matches += 1;
        } else {
            return Err(errors::verify_mismatch(format!(
                "{} base slot mismatch",
                row.chunk_id
            )));
        }
        for slot in default_slot_ids().into_iter().skip(1) {
            if vault.read_slot_vector_at(snapshot, cx_id, slot)?.is_none() {
                missing_backfill.push(format!("{}:slot{}", row.chunk_id, slot.get()));
            }
        }
    }
    let checked = rows.len() * default_slot_ids().len().saturating_sub(1);
    if require_backfill && !missing_backfill.is_empty() {
        return Err(errors::backfill_incomplete(format!(
            "{} missing slot rows",
            missing_backfill.len()
        )));
    }
    Ok(VerifyReport {
        total: rows.len(),
        matched,
        base_slot_matches,
        backfill_slots_checked: checked,
        missing_backfill,
        gate: if matched == rows.len() {
            "PASS"
        } else {
            "FAIL"
        }
        .to_string(),
    })
}

pub fn status(vault: &AsterVault) -> Result<StatusReport> {
    let snapshot = vault.snapshot();
    let mut slot_rows = BTreeMap::new();
    for slot in default_slot_ids() {
        let count = vault.scan_cf_at(snapshot, ColumnFamily::slot(slot))?.len();
        slot_rows.insert(format!("slot_{}", slot.get()), count);
    }
    Ok(StatusReport {
        base_rows: vault.scan_cf_at(snapshot, ColumnFamily::Base)?.len(),
        slot_rows,
        latest_seq: snapshot,
    })
}

pub fn readback_chunk(
    vault: &AsterVault,
    row: &ChunkRow,
    adapter: &VaultSqliteAdapter,
) -> Result<serde_json::Value> {
    let snapshot = vault.snapshot();
    let cx_id = adapter.cx_id(row);
    let cx = vault.get(cx_id, snapshot)?;
    let mut slots = BTreeMap::new();
    for slot in default_slot_ids() {
        let vector = vault.read_slot_vector_at(snapshot, cx_id, slot)?;
        slots.insert(slot.get().to_string(), slot_json(slot, vector)?);
    }
    Ok(json!({
        "chunk_id": row.chunk_id,
        "database_name": row.database_name,
        "cx_id": cx_id.to_string(),
        "snapshot": snapshot,
        "input_hash": hex_encode(&cx.input_ref.hash),
        "expected_content_hash": hex_encode(&row.content_hash()),
        "metadata": cx.metadata,
        "slots": slots,
    }))
}

fn slot_matches(vector: Option<SlotVector>, expected: &[f32]) -> bool {
    matches!(
        vector,
        Some(SlotVector::Dense { dim, data })
            if dim as usize == expected.len() && data == expected
    )
}

fn slot_json(_slot: SlotId, vector: Option<SlotVector>) -> Result<serde_json::Value> {
    let Some(vector) = vector else {
        return Ok(json!({"present": false}));
    };
    let bytes = serde_json::to_vec(&vector)
        .map_err(|err| errors::verify_mismatch(format!("encode slot vector: {err}")))?;
    let kind = match &vector {
        SlotVector::Dense { dim, .. } => format!("dense:{dim}"),
        SlotVector::Sparse { dim, entries } => format!("sparse:{dim}:{}", entries.len()),
        SlotVector::Multi { token_dim, tokens } => format!("multi:{token_dim}:{}", tokens.len()),
        SlotVector::Absent { .. } => "absent".to_string(),
    };
    Ok(json!({
        "present": true,
        "kind": kind,
        "json_sha256": hex_encode(blake3::hash(&bytes).as_bytes()),
    }))
}
