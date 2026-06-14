use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{Constellation, CxId, Result, Seq, SlotId, SlotVector, VaultStore};
use calyx_ledger::{LedgerCfStore, VerifyResult, verify_chain};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ledger_store::AsterLedgerCfStore;

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
    pub mismatched: usize,
    pub errors: Vec<VerifyError>,
    pub base_slot_matches: usize,
    pub backfill_slots_checked: usize,
    pub missing_backfill: Vec<String>,
    pub gate: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyError {
    pub row_num: u64,
    pub chunk_id: String,
    pub expected_hash: [u8; 32],
    pub actual_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub base_rows: usize,
    pub slot_rows: BTreeMap<String, usize>,
    pub ledger_chain: LedgerChainStatus,
    pub first_chunk_id: Option<String>,
    pub last_chunk_id: Option<String>,
    pub latest_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerChainStatus {
    pub state: String,
    pub count: u64,
    pub checked_range: String,
    pub at_seq: Option<u64>,
    pub reason: Option<String>,
}

pub fn verify_migration(
    vault: &AsterVault,
    rows: &[ChunkRow],
    adapter: &VaultSqliteAdapter,
    require_backfill: bool,
) -> Result<VerifyReport> {
    let mut matched = 0;
    let mut base_slot_matches = 0;
    let mut errors = Vec::new();
    let mut missing_backfill = Vec::new();
    let snapshot = vault.snapshot();
    for row in rows {
        let cx_id = adapter.cx_id(row);
        let expected_hash = content_hash(&row.content);
        let cx = match vault.get(cx_id, snapshot) {
            Ok(cx) => cx,
            Err(_) => {
                errors.push(VerifyError {
                    row_num: row.row_num,
                    chunk_id: row.chunk_id.clone(),
                    expected_hash,
                    actual_hash: [0; 32],
                });
                continue;
            }
        };
        if cx.input_ref.hash == expected_hash {
            matched += 1;
        } else {
            errors.push(VerifyError {
                row_num: row.row_num,
                chunk_id: row.chunk_id.clone(),
                expected_hash,
                actual_hash: cx.input_ref.hash,
            });
        }
        if slot_matches(
            vault.read_slot_vector_at(snapshot, cx_id, BASE_SLOT)?,
            &row.embedding,
        ) {
            base_slot_matches += 1;
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
    let mismatched = errors.len();
    Ok(VerifyReport {
        total: rows.len(),
        matched,
        mismatched,
        errors,
        base_slot_matches,
        backfill_slots_checked: checked,
        missing_backfill,
        gate: if mismatched == 0 && matched == rows.len() {
            "PASS"
        } else {
            "FAIL"
        }
        .to_string(),
    })
}

pub fn row_exists_and_matches(
    vault: &AsterVault,
    row: &ChunkRow,
    adapter: &VaultSqliteAdapter,
) -> Result<bool> {
    let snapshot = vault.snapshot();
    let cx_id = adapter.cx_id(row);
    let cx = match vault.get(cx_id, snapshot) {
        Ok(cx) => cx,
        Err(err) if err.code == "CALYX_STALE_DERIVED" => return Ok(false),
        Err(err) => return Err(err),
    };
    verify_base_row(vault, snapshot, row, cx_id, &cx)?;
    Ok(true)
}

fn verify_base_row(
    vault: &AsterVault,
    snapshot: Seq,
    row: &ChunkRow,
    cx_id: CxId,
    cx: &Constellation,
) -> Result<()> {
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
    if !slot_matches(
        vault.read_slot_vector_at(snapshot, cx_id, BASE_SLOT)?,
        &row.embedding,
    ) {
        return Err(errors::verify_mismatch(format!(
            "{} base slot mismatch",
            row.chunk_id
        )));
    }
    Ok(())
}

pub(super) fn content_hash(content: &[u8]) -> [u8; 32] {
    *blake3::hash(content).as_bytes()
}

pub fn status(vault: &AsterVault, vault_dir: &Path) -> Result<StatusReport> {
    let snapshot = vault.snapshot();
    let mut slot_rows = BTreeMap::new();
    for slot in default_slot_ids() {
        let count = vault.scan_cf_at(snapshot, ColumnFamily::slot(slot))?.len();
        slot_rows.insert(format!("slot_{}", slot.get()), count);
    }
    let base_rows = vault.scan_cf_at(snapshot, ColumnFamily::Base)?;
    let (first_chunk_id, last_chunk_id) = chunk_id_extents(&base_rows)?;
    Ok(StatusReport {
        base_rows: base_rows.len(),
        slot_rows,
        ledger_chain: ledger_chain_status(vault_dir)?,
        first_chunk_id,
        last_chunk_id,
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

fn chunk_id_extents(rows: &[(Vec<u8>, Vec<u8>)]) -> Result<(Option<String>, Option<String>)> {
    let mut chunks = Vec::new();
    for (idx, (_, bytes)) in rows.iter().enumerate() {
        let cx = decode_constellation_base(bytes)?;
        if let Some(chunk_id) = cx.chunk_id() {
            let row_num = cx
                .metadata
                .get(METADATA_ROWID)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(idx as u64);
            chunks.push((row_num, chunk_id.to_string()));
        }
    }
    chunks.sort_by_key(|(row_num, _)| *row_num);
    Ok((
        chunks.first().map(|(_, chunk_id)| chunk_id.clone()),
        chunks.last().map(|(_, chunk_id)| chunk_id.clone()),
    ))
}

fn ledger_chain_status(vault_dir: &Path) -> Result<LedgerChainStatus> {
    let store = match AsterLedgerCfStore::open(vault_dir) {
        Ok(store) => store,
        Err(error) => {
            return Ok(LedgerChainStatus {
                state: "unavailable".to_string(),
                count: 0,
                checked_range: "0..0".to_string(),
                at_seq: None,
                reason: Some(error.to_string()),
            });
        }
    };
    let rows = store.scan()?;
    let end = rows
        .iter()
        .map(|row| row.seq)
        .max()
        .map_or(0, |seq| seq.saturating_add(1));
    let checked_range = format!("0..{end}");
    match verify_chain(&store, 0..end)? {
        VerifyResult::Intact { count } => Ok(LedgerChainStatus {
            state: "Intact".to_string(),
            count,
            checked_range,
            at_seq: None,
            reason: None,
        }),
        VerifyResult::Broken { at_seq, .. } => Ok(LedgerChainStatus {
            state: "Broken".to_string(),
            count: rows.len() as u64,
            checked_range,
            at_seq: Some(at_seq),
            reason: Some(format!("ledger chain broken at seq {at_seq}")),
        }),
        VerifyResult::Corrupt { at_seq, reason } => Ok(LedgerChainStatus {
            state: "Corrupt".to_string(),
            count: rows.len() as u64,
            checked_range,
            at_seq: Some(at_seq),
            reason: Some(reason),
        }),
    }
}
