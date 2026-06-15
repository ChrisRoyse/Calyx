use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{SlotVector, VaultStore};

use super::{
    BenchmarkRow, CALYX_CONTRACT_NAME_MISMATCH, CALYX_ROUND_TRIP_MISMATCH, CalyxEntry,
    MismatchDetail, SourceRow, blake3_bytes, manifest_corrupt, mismatch, vector_hash,
};
use crate::leapable::dual_write::aster_dir;
use crate::leapable::shadow_harness::read_shadow_manifest;
use crate::migrate;
use crate::migrate::adapter::{BASE_SLOT, METADATA_CONTENT_HASH};
use crate::migrate::manifest::{MigrationManifest, hex_decode, hex_encode, manifest_path};

const METADATA_TEXT_HASH: &str = "text_hash";

pub(super) fn open_calyx(
    calyx_dir: &Path,
) -> Result<(PathBuf, calyx_aster::vault::AsterVault, MigrationManifest), calyx_core::CalyxError> {
    let aster = resolve_aster_dir(calyx_dir)?;
    let manifest = MigrationManifest::load(&aster).map_err(|err| manifest_corrupt(err.message))?;
    let vault = migrate::open_vault(&aster, &manifest)?;
    Ok((aster, vault, manifest))
}

pub(super) fn calyx_index(
    vault: &calyx_aster::vault::AsterVault,
) -> Result<BTreeMap<String, Vec<CalyxEntry>>, calyx_core::CalyxError> {
    let snapshot = vault.snapshot();
    let mut out: BTreeMap<String, Vec<CalyxEntry>> = BTreeMap::new();
    for (_key, bytes) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let cx = decode_constellation_base(&bytes)?;
        let chunk_id = cx.chunk_id().unwrap_or("").to_string();
        let vector = match vault.read_slot_vector_at(snapshot, cx.cx_id, BASE_SLOT)? {
            Some(SlotVector::Dense { data, .. }) => Some(data),
            _ => None,
        };
        out.entry(chunk_id)
            .or_default()
            .push(CalyxEntry { cx, vector });
    }
    Ok(out)
}

pub(super) fn verify_row(
    row: &SourceRow,
    index: &BTreeMap<String, Vec<CalyxEntry>>,
    mismatches: &mut Vec<MismatchDetail>,
) {
    let Some(entry) = choose_entry(row, index) else {
        mismatches.push(mismatch(
            CALYX_ROUND_TRIP_MISMATCH,
            &row.row.chunk_id,
            "missing",
            &row.text_hash,
            &[0; 32],
            Some(row.row.database_name.clone()),
            None,
        ));
        return;
    };
    let actual_database = entry.cx.database_name().unwrap_or("").to_string();
    if actual_database != row.row.database_name {
        mismatches.push(mismatch(
            CALYX_CONTRACT_NAME_MISMATCH,
            &row.row.chunk_id,
            "database_name",
            &blake3_bytes(row.row.database_name.as_bytes()),
            &blake3_bytes(actual_database.as_bytes()),
            Some(row.row.database_name.clone()),
            Some(actual_database),
        ));
    }
    let (actual_text_hash, actual_text_value) = calyx_text_hash(&entry.cx);
    if actual_text_hash != row.text_hash {
        mismatches.push(mismatch(
            CALYX_ROUND_TRIP_MISMATCH,
            &row.row.chunk_id,
            "text_hash",
            &row.text_hash,
            &actual_text_hash,
            None,
            Some(actual_text_value),
        ));
    }
    match &entry.vector {
        Some(vector) if vector == &row.row.embedding => {}
        Some(vector) => mismatches.push(mismatch(
            CALYX_ROUND_TRIP_MISMATCH,
            &row.row.chunk_id,
            "vector",
            &vector_hash(&row.row.embedding),
            &vector_hash(vector),
            None,
            None,
        )),
        None => mismatches.push(mismatch(
            CALYX_ROUND_TRIP_MISMATCH,
            &row.row.chunk_id,
            "vector",
            &vector_hash(&row.row.embedding),
            &[0; 32],
            None,
            Some("missing base slot".to_string()),
        )),
    }
}

pub(super) fn calyx_benchmark_rows(
    vault: &calyx_aster::vault::AsterVault,
) -> Result<Vec<BenchmarkRow>, calyx_core::CalyxError> {
    let mut rows = Vec::new();
    for entries in calyx_index(vault)?.into_values() {
        for entry in entries {
            if let Some(embedding) = entry.vector {
                rows.push(BenchmarkRow {
                    chunk_id: entry.cx.chunk_id().unwrap_or("").to_string(),
                    embedding,
                });
            }
        }
    }
    Ok(rows)
}

fn resolve_aster_dir(calyx_dir: &Path) -> Result<PathBuf, calyx_core::CalyxError> {
    let shadow_aster = aster_dir(calyx_dir);
    let shadow_manifest = calyx_dir.join("MANIFEST");
    if manifest_path(&shadow_aster).is_file() {
        if shadow_manifest.exists() {
            read_shadow_manifest(calyx_dir)?;
        }
        return Ok(shadow_aster);
    }
    if manifest_path(calyx_dir).is_file() {
        return Ok(calyx_dir.to_path_buf());
    }
    if shadow_manifest.exists() {
        read_shadow_manifest(calyx_dir)?;
    }
    Err(manifest_corrupt(format!(
        "no migration manifest found under {} or {}",
        calyx_dir.display(),
        shadow_aster.display()
    )))
}

fn choose_entry<'a>(
    row: &SourceRow,
    index: &'a BTreeMap<String, Vec<CalyxEntry>>,
) -> Option<&'a CalyxEntry> {
    let candidates = index.get(&row.row.chunk_id)?;
    candidates
        .iter()
        .find(|entry| entry.cx.database_name() == Some(row.row.database_name.as_str()))
        .or_else(|| candidates.first())
}

fn calyx_text_hash(cx: &calyx_core::Constellation) -> ([u8; 32], String) {
    for key in [METADATA_TEXT_HASH, METADATA_CONTENT_HASH] {
        if let Some(value) = cx.metadata.get(key) {
            if let Ok(bytes) = hex_decode(value)
                && let Ok(hash) = bytes.try_into()
            {
                return (hash, value.clone());
            }
            return ([0; 32], format!("invalid {key}={value}"));
        }
    }
    (cx.input_ref.hash, hex_encode(&cx.input_ref.hash))
}
