use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{LensCatalog, LensCatalogEntry};

const INDEX_KEY: &[u8] = b"calyx/lens/catalog/v1/index";
const ENTRY_PREFIX: &[u8] = b"calyx/lens/catalog/v1/entry/";
const INDEX_MAGIC: &[u8] = b"CLCATIX1\0";
const ENTRY_MAGIC: &[u8] = b"CLCATEN1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const LEGACY_CATALOG_FILE: &str = "registry.json";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensCatalogDbReadback {
    pub(crate) catalog_db: PathBuf,
    pub(crate) row_count: usize,
    pub(crate) lens_count: usize,
    pub(crate) total_value_bytes: u64,
    pub(crate) index_value_sha256: String,
    pub(crate) catalog_sha256: String,
    pub(crate) readback_matches: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct LensCatalogIndex {
    format: String,
    lens_ids: Vec<String>,
}

pub(crate) fn read(db_root: &Path) -> Result<LensCatalog> {
    Ok(read_with_readback(db_root)?.0)
}

pub(crate) fn read_with_readback(db_root: &Path) -> Result<(LensCatalog, LensCatalogDbReadback)> {
    let router = CfRouter::open(db_root, CF_MEMTABLE_CAP)?;
    let Some(index_value) = router.get(ColumnFamily::Graph, INDEX_KEY)? else {
        if legacy_catalog_path(db_root).exists() {
            return Err(error(
                "CALYX_LENS_CATALOG_DB_MISSING",
                "legacy lens registry.json exists but the authoritative Calyx/Aster catalog row is missing",
            ));
        }
        let catalog = LensCatalog { lenses: Vec::new() };
        return Ok((catalog, empty_readback(db_root)));
    };
    let index: LensCatalogIndex = decode(&index_value, INDEX_MAGIC)?;
    if index.format != "calyx-lens-catalog-v1" {
        return Err(error(
            "CALYX_LENS_CATALOG_DB_INVALID",
            "lens catalog index decoded to an unsupported format",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut lenses = Vec::with_capacity(index.lens_ids.len());
    let mut total_value_bytes = index_value.len() as u64;
    for lens_id in &index.lens_ids {
        if !seen.insert(lens_id.clone()) {
            return Err(error(
                "CALYX_LENS_CATALOG_DB_INVALID",
                format!("lens catalog index contains duplicate lens_id {lens_id}"),
            ));
        }
        let key = entry_key(lens_id)?;
        let value = router.get(ColumnFamily::Graph, &key)?.ok_or_else(|| {
            error(
                "CALYX_LENS_CATALOG_DB_MISSING",
                format!("lens catalog entry row missing for lens_id {lens_id}"),
            )
        })?;
        let entry: LensCatalogEntry = decode(&value, ENTRY_MAGIC)?;
        if entry.lens_id != *lens_id {
            return Err(error(
                "CALYX_LENS_CATALOG_DB_INVALID",
                format!(
                    "lens catalog entry row key {lens_id} decoded as {}",
                    entry.lens_id
                ),
            ));
        }
        total_value_bytes = total_value_bytes.saturating_add(value.len() as u64);
        lenses.push(entry);
    }
    lenses.sort_by(|left, right| left.lens_id.cmp(&right.lens_id));
    let catalog = LensCatalog { lenses };
    let readback = LensCatalogDbReadback {
        catalog_db: db_root.to_path_buf(),
        row_count: catalog.lenses.len().saturating_add(1),
        lens_count: catalog.lenses.len(),
        total_value_bytes,
        index_value_sha256: hex_sha256(&index_value),
        catalog_sha256: catalog_sha256(&catalog)?,
        readback_matches: true,
    };
    Ok((catalog, readback))
}

pub(crate) fn write(db_root: &Path, catalog: &LensCatalog) -> Result<LensCatalogDbReadback> {
    let catalog = canonical_catalog(catalog)?;
    let index = LensCatalogIndex {
        format: "calyx-lens-catalog-v1".to_string(),
        lens_ids: catalog
            .lenses
            .iter()
            .map(|entry| entry.lens_id.clone())
            .collect(),
    };
    let index_value = encode(&index, INDEX_MAGIC)?;
    let mut router = CfRouter::open(db_root, CF_MEMTABLE_CAP)?;
    for entry in &catalog.lenses {
        router.put(
            ColumnFamily::Graph,
            &entry_key(&entry.lens_id)?,
            &encode(entry, ENTRY_MAGIC)?,
        )?;
    }
    router.put(ColumnFamily::Graph, INDEX_KEY, &index_value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);

    let (readback_catalog, readback) = read_with_readback(db_root)?;
    if catalog_sha256(&readback_catalog)? != catalog_sha256(&catalog)? {
        return Err(error(
            "CALYX_LENS_CATALOG_DB_MISMATCH",
            "lens catalog Calyx/Aster Graph CF readback does not match the written catalog",
        ));
    }
    Ok(readback)
}

pub(crate) fn catalog_sha256(catalog: &LensCatalog) -> Result<String> {
    let catalog = canonical_catalog(catalog)?;
    let payload = bincode::serde::encode_to_vec(&catalog, config::standard()).map_err(|err| {
        error(
            "CALYX_LENS_CATALOG_DB_ENCODE",
            format!("encode lens catalog fingerprint failed: {err}"),
        )
    })?;
    Ok(hex_sha256(&payload))
}

pub(crate) fn legacy_catalog_path(db_root: &Path) -> PathBuf {
    db_root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LEGACY_CATALOG_FILE)
}

fn canonical_catalog(catalog: &LensCatalog) -> Result<LensCatalog> {
    let mut out = catalog.clone();
    let mut ids = BTreeSet::new();
    for entry in &out.lenses {
        if entry.lens_id.trim().is_empty() {
            return Err(error(
                "CALYX_LENS_CATALOG_DB_INVALID",
                "lens catalog entry has an empty lens_id",
            ));
        }
        if !ids.insert(entry.lens_id.clone()) {
            return Err(error(
                "CALYX_LENS_CATALOG_DB_INVALID",
                format!("lens catalog contains duplicate lens_id {}", entry.lens_id),
            ));
        }
    }
    out.lenses
        .sort_by(|left, right| left.lens_id.cmp(&right.lens_id));
    Ok(out)
}

fn entry_key(lens_id: &str) -> Result<Vec<u8>> {
    if lens_id.trim().is_empty() || lens_id.as_bytes().contains(&0) {
        return Err(error(
            "CALYX_LENS_CATALOG_DB_INVALID_KEY",
            "lens catalog entry key requires a non-empty lens_id without NUL bytes",
        ));
    }
    let mut key = Vec::with_capacity(ENTRY_PREFIX.len() + lens_id.len());
    key.extend_from_slice(ENTRY_PREFIX);
    key.extend_from_slice(lens_id.as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T, magic: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = magic.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_LENS_CATALOG_DB_ENCODE",
            format!("encode lens catalog row failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], magic: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(magic).ok_or_else(|| {
        error(
            "CALYX_LENS_CATALOG_DB_INVALID",
            "lens catalog row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_LENS_CATALOG_DB_DECODE",
                format!("decode lens catalog row failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_LENS_CATALOG_DB_INVALID",
            "lens catalog row has trailing bytes",
        ));
    }
    Ok(record)
}

fn empty_readback(db_root: &Path) -> LensCatalogDbReadback {
    LensCatalogDbReadback {
        catalog_db: db_root.to_path_buf(),
        row_count: 0,
        lens_count: 0,
        total_value_bytes: 0,
        index_value_sha256: String::new(),
        catalog_sha256: hex_sha256(&[]),
        readback_matches: true,
    }
}

fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read the lens catalog through Calyx/Aster Graph CF; use calyx lens migrate-catalog only for explicit legacy imports",
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_core::{LensCost, Placement};

    use super::*;

    #[test]
    fn graph_cf_catalog_round_trips_without_registry_json() {
        let root = temp_root("round-trip");
        let catalog = LensCatalog {
            lenses: vec![entry("b", "bee"), entry("a", "aye")],
        };

        let written = write(&root, &catalog).unwrap();
        let (readback, read_report) = read_with_readback(&root).unwrap();

        assert!(written.readback_matches);
        assert_eq!(written.catalog_sha256, read_report.catalog_sha256);
        assert_eq!(readback.lenses[0].lens_id, "a");
        assert_eq!(readback.lenses[1].lens_id, "b");
        assert!(!legacy_catalog_path(&root).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_json_without_db_row_is_refused() {
        let root = temp_root("legacy-refused");
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        fs::write(legacy_catalog_path(&root), br#"{"lenses":[]}"#).unwrap();

        let err = read(&root).unwrap_err();

        assert_eq!(err.code, "CALYX_LENS_CATALOG_DB_MISSING");
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    fn entry(lens_id: &str, name: &str) -> LensCatalogEntry {
        LensCatalogEntry {
            lens_id: lens_id.to_string(),
            name: name.to_string(),
            modality: "text".to_string(),
            runtime: "tei_http".to_string(),
            dim: 768,
            retrieval_only: false,
            excluded_from_dedup: false,
            weights_sha256: "00".repeat(32),
            manifest: PathBuf::from(format!("{name}.json")),
            cost: LensCost::zero(),
            placement: Placement::Gpu,
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "calyx-lens-catalog-store-{label}-{}-{nanos}",
            std::process::id()
        ));
        home.join("lenses").join("catalog-db")
    }
}
