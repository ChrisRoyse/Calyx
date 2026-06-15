use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{CalyxError, LedgerRef};
use serde::{Deserialize, Serialize};

use super::dual_write::aster_dir;
use super::read_flip::{AskResult, ask_calyx};
use super::shadow_harness::{VaultMode, read_shadow_manifest};
use crate::migrate;
use crate::migrate::adapter::METADATA_CONTENT_HASH;
use crate::migrate::manifest::{MigrationManifest, hex_encode};

mod cli;
mod pg;

pub(crate) use cli::run_production_fsv;
#[cfg(test)]
pub(crate) use pg::{CALYX_PG_WRITE_ATTEMPTED, CALYX_VAULT_NOT_IN_PG};
pub(crate) use pg::{PgConn, PgSnapshot, REQUIRED_TABLES, snapshot_pg_state};

pub(crate) const CALYX_PG_STATE_CHANGED: &str = "CALYX_PG_STATE_CHANGED";
pub(crate) const CALYX_PG_CONTRACT_VIOLATION: &str = "CALYX_PG_CONTRACT_VIOLATION";
pub(crate) const CALYX_VAULT_NOT_CALYX_ONLY: &str = "CALYX_VAULT_NOT_CALYX_ONLY";
pub(crate) const CALYX_REPRODUCE_MISMATCH: &str = "CALYX_REPRODUCE_MISMATCH";
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PgUnchangedProof {
    pub(crate) vault_name: String,
    pub(crate) matched_tables: usize,
    pub(crate) all_hashes_match: bool,
    pub(crate) tables: Vec<TableHashMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TableHashMatch {
    pub(crate) table: String,
    pub(crate) before_blake3: String,
    pub(crate) after_blake3: String,
    pub(crate) bytes_identical: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AskProof {
    pub(crate) mode: VaultMode,
    pub(crate) top_k: usize,
    pub(crate) hits: Vec<HitProof>,
    pub(crate) all_ledger_refs_valid: bool,
    pub(crate) reproduced_byte_exact: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HitProof {
    pub(crate) rank: usize,
    pub(crate) chunk_id: String,
    pub(crate) database_name: String,
    pub(crate) cx_id: String,
    pub(crate) score: f64,
    pub(crate) ledger_ref: LedgerRef,
    pub(crate) ledger_value_blake3: String,
    pub(crate) ledger_hash_matches_ref: bool,
    pub(crate) base_value_blake3: String,
    pub(crate) text_hash: String,
    pub(crate) chunk_id_byte_exact: bool,
    pub(crate) text_hash_byte_exact: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContractProof {
    pub(crate) database_name: String,
    pub(crate) matched_tables: usize,
    pub(crate) creator_database_row_present: bool,
    pub(crate) all_required_tables_present: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvidenceBundle {
    pub(crate) schema_version: u32,
    pub(crate) database_name: String,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) calyx_only_at_seq: u64,
    pub(crate) pg_snapshot_before_hash: String,
    pub(crate) pg_snapshot_after_hash: String,
    pub(crate) all_hashes_match: bool,
    pub(crate) all_ledger_refs_valid: bool,
    pub(crate) reproduced_byte_exact: bool,
    pub(crate) ask_proof: AskProof,
    pub(crate) pg_unchanged: PgUnchangedProof,
    pub(crate) contract: ContractProof,
}

pub(crate) struct ProductionFSV;

impl ProductionFSV {
    pub(crate) fn verify_pg_unchanged(
        before: &PgSnapshot,
        after: &PgSnapshot,
    ) -> Result<PgUnchangedProof, CalyxError> {
        if before.vault_name != after.vault_name {
            return Err(pg_state_changed(format!(
                "vault_name {} != {}",
                before.vault_name, after.vault_name
            )));
        }
        let after_by_table = after
            .tables
            .iter()
            .map(|table| (table.table.as_str(), table))
            .collect::<BTreeMap<_, _>>();
        let mut matches = Vec::with_capacity(before.tables.len());
        for table in &before.tables {
            let Some(after_table) = after_by_table.get(table.table.as_str()) else {
                return Err(pg_state_changed(format!(
                    "table {} missing from after snapshot",
                    table.table
                )));
            };
            let bytes_identical = table.blake3 == after_table.blake3
                && table.bytes_len == after_table.bytes_len
                && table.row_count == after_table.row_count;
            if !bytes_identical {
                return Err(pg_state_changed(format!(
                    "table={} before_hash={} after_hash={}",
                    table.table, table.blake3, after_table.blake3
                )));
            }
            matches.push(TableHashMatch {
                table: table.table.clone(),
                before_blake3: table.blake3.clone(),
                after_blake3: after_table.blake3.clone(),
                bytes_identical,
            });
        }
        Ok(PgUnchangedProof {
            vault_name: before.vault_name.clone(),
            matched_tables: matches.len(),
            all_hashes_match: true,
            tables: matches,
        })
    }

    pub(crate) fn verify_control_plane_contract(
        vault_name: &str,
        snapshot: &PgSnapshot,
    ) -> Result<ContractProof, CalyxError> {
        if snapshot.vault_name != vault_name {
            return Err(pg_contract_violation(format!(
                "snapshot vault_name {} != {vault_name}",
                snapshot.vault_name
            )));
        }
        let creator = snapshot
            .tables
            .iter()
            .find(|table| table.table == "creator_databases")
            .ok_or_else(|| pg_contract_violation("creator_databases snapshot missing"))?;
        if !creator.contains_vault_name || creator.row_count == 0 {
            return Err(pg_contract_violation(format!(
                "creator_databases has no byte-exact row for {vault_name}"
            )));
        }
        Ok(ContractProof {
            database_name: vault_name.to_string(),
            matched_tables: snapshot.tables.len(),
            creator_database_row_present: true,
            all_required_tables_present: snapshot.tables.len() == REQUIRED_TABLES.len(),
        })
    }

    pub(crate) fn run_full_ask_cycle(
        calyx_dir: &Path,
        query_vec: &[f32],
        top_k: usize,
    ) -> Result<AskProof, CalyxError> {
        let manifest = read_shadow_manifest(calyx_dir)?;
        if manifest.mode != VaultMode::CalyxOnly {
            return Err(error(
                CALYX_VAULT_NOT_CALYX_ONLY,
                format!("vault mode is {:?}", manifest.mode),
                "run remove-shadow before production-fsv",
            ));
        }
        let ask = ask_calyx(calyx_dir, manifest.mode, query_vec, top_k)?;
        Self::prove_ask(calyx_dir, ask)
    }

    pub(crate) fn emit_evidence(path: &Path, bundle: &EvidenceBundle) -> Result<(), CalyxError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                pg_contract_violation(format!("create {}: {err}", parent.display()))
            })?;
        }
        let bytes = serde_json::to_vec_pretty(bundle)
            .map_err(|err| pg_contract_violation(format!("encode evidence: {err}")))?;
        fs::write(path, bytes)
            .map_err(|err| pg_contract_violation(format!("write {}: {err}", path.display())))
    }

    pub(crate) fn bundle(
        calyx_dir: &Path,
        before: PgSnapshot,
        after: PgSnapshot,
        ask_proof: AskProof,
    ) -> Result<EvidenceBundle, CalyxError> {
        let pg_unchanged = Self::verify_pg_unchanged(&before, &after)?;
        let contract = Self::verify_control_plane_contract(&before.vault_name, &after)?;
        let manifest = read_shadow_manifest(calyx_dir)?;
        Ok(EvidenceBundle {
            schema_version: SCHEMA_VERSION,
            database_name: before.vault_name.clone(),
            calyx_dir: calyx_dir.to_path_buf(),
            calyx_only_at_seq: manifest
                .features
                .get("calyx_only_at_seq")
                .and_then(|value| value.parse().ok())
                .unwrap_or(manifest.chunk_count),
            pg_snapshot_before_hash: before.snapshot_blake3.clone(),
            pg_snapshot_after_hash: after.snapshot_blake3.clone(),
            all_hashes_match: pg_unchanged.all_hashes_match,
            all_ledger_refs_valid: ask_proof.all_ledger_refs_valid,
            reproduced_byte_exact: ask_proof.reproduced_byte_exact,
            ask_proof,
            pg_unchanged,
            contract,
        })
    }

    fn prove_ask(calyx_dir: &Path, ask: AskResult) -> Result<AskProof, CalyxError> {
        let (aster, _manifest) = open_aster(calyx_dir)?;
        let snapshot = aster.latest_seq();
        let mut bases = BTreeMap::new();
        for (_key, bytes) in aster.scan_cf_at(snapshot, ColumnFamily::Base)? {
            let cx = decode_constellation_base(&bytes)?;
            bases.insert(cx.cx_id.to_string(), (cx, bytes));
        }
        let mut proofs = Vec::with_capacity(ask.hits.len());
        for hit in ask.hits {
            let (cx, base_bytes) = bases
                .get(&hit.cx_id)
                .ok_or_else(|| reproduce_mismatch(format!("hit {} base row absent", hit.cx_id)))?;
            let ledger_bytes = aster
                .read_cf_at(
                    snapshot,
                    ColumnFamily::Ledger,
                    &ledger_key(hit.ledger_ref.seq),
                )?
                .ok_or_else(|| {
                    reproduce_mismatch(format!("ledger seq {} absent", hit.ledger_ref.seq))
                })?;
            let entry = calyx_ledger::decode(&ledger_bytes)?;
            let text_hash = cx
                .metadata
                .get(METADATA_CONTENT_HASH)
                .cloned()
                .unwrap_or_default();
            let chunk_id_byte_exact = cx.chunk_id() == Some(hit.chunk_id.as_str());
            let text_hash_byte_exact = text_hash == hex_encode(&cx.input_ref.hash);
            let ledger_hash_matches_ref = entry.entry_hash == hit.ledger_ref.hash;
            if !chunk_id_byte_exact || !text_hash_byte_exact || !ledger_hash_matches_ref {
                return Err(reproduce_mismatch(format!(
                    "hit {} failed byte proof chunk={} text_hash={} ledger_hash={}",
                    hit.cx_id, chunk_id_byte_exact, text_hash_byte_exact, ledger_hash_matches_ref
                )));
            }
            proofs.push(HitProof {
                rank: hit.rank,
                chunk_id: hit.chunk_id,
                database_name: hit.database_name,
                cx_id: hit.cx_id,
                score: hit.score,
                ledger_ref: hit.ledger_ref,
                ledger_value_blake3: blake3::hash(&ledger_bytes).to_string(),
                ledger_hash_matches_ref,
                base_value_blake3: blake3::hash(base_bytes).to_string(),
                text_hash,
                chunk_id_byte_exact,
                text_hash_byte_exact,
            });
        }
        Ok(AskProof {
            mode: ask.mode,
            top_k: ask.top_k,
            hits: proofs,
            all_ledger_refs_valid: true,
            reproduced_byte_exact: true,
        })
    }
}

fn open_aster(
    calyx_dir: &Path,
) -> Result<(calyx_aster::vault::AsterVault, MigrationManifest), CalyxError> {
    let aster_dir = aster_dir(calyx_dir);
    let manifest = MigrationManifest::load(&aster_dir)?;
    let vault = migrate::open_vault(&aster_dir, &manifest)?;
    Ok((vault, manifest))
}

fn pg_state_changed(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_PG_STATE_CHANGED,
        message,
        "compare pg_before.json and pg_after.json table hashes before retrying",
    )
}

fn pg_contract_violation(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_PG_CONTRACT_VIOLATION,
        message,
        "inspect control-plane table dumps for table names, values, and database_name bytes",
    )
}

fn reproduce_mismatch(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_REPRODUCE_MISMATCH,
        message,
        "read the referenced base and ledger CF rows and repair provenance before retrying",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
