use std::collections::BTreeMap;
use std::path::Path;

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

#[path = "label_store/args.rs"]
mod args;
#[path = "label_store/import.rs"]
mod import;

#[cfg(test)]
pub(crate) use import::load_rows_jsonl;
pub(crate) use import::run_import;

const KEY_PREFIX: &[u8] = b"calyx/assay/i8bin-label-anchor/v1/";
const VALUE_MAGIC: &[u8] = b"CAILBL1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const DEFAULT_ASSOCIATION_KEY: &str = "i8bin_label_anchor";
pub(crate) const DEFAULT_CHUNK_ROWS: usize = 8192;
pub(crate) const FORMAT: &str = "calyx-assay-i8bin-label-anchor-v1";
pub(crate) const MODE: &str = "i8bin_label_anchor";
pub(crate) const ROW_ID_SPACE: &str = "partitioned_rrf_plan_corpus_row_idx";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LabelAnchorManifestRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) row_id_space: String,
    pub(crate) imported_rows_sha256: String,
    pub(crate) anchor_name: String,
    pub(crate) target_class: usize,
    pub(crate) row_count: usize,
    pub(crate) positive_count: usize,
    pub(crate) negative_count: usize,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) chunk_rows: usize,
    pub(crate) chunk_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LabelAnchorChunkRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) association_key: String,
    pub(crate) chunk_index: usize,
    pub(crate) first_row_idx: usize,
    pub(crate) labels: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedLabels {
    pub(crate) labels: Vec<bool>,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) source_sha256: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedLabelAnchor {
    pub(crate) manifest: LabelAnchorManifestRecord,
    pub(crate) labels: Vec<bool>,
    pub(crate) db_readback: LabelAnchorDbReadback,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LabelAnchorDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) manifest_key_sha256: String,
    pub(crate) manifest_value_bytes: usize,
    pub(crate) manifest_value_sha256: String,
    pub(crate) chunk_count: usize,
    pub(crate) chunk_value_bytes: usize,
    pub(crate) chunk_value_sha256: String,
    pub(crate) row_count: usize,
    pub(crate) positive_count: usize,
    pub(crate) negative_count: usize,
    pub(crate) readback_matches: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write(
    cf_root: &Path,
    association_key: &str,
    anchor_name: &str,
    target_class: usize,
    imported_rows_sha256: &str,
    label_counts: &BTreeMap<String, usize>,
    labels: &[bool],
    chunk_rows: usize,
) -> Result<LabelAnchorDbReadback> {
    validate_labels(labels)?;
    let chunk_rows = validate_chunk_rows(chunk_rows)?;
    let manifest_key = manifest_key(association_key)?;
    let chunk_count = labels.len().div_ceil(chunk_rows);
    let positive_count = labels.iter().filter(|label| **label).count();
    let manifest = LabelAnchorManifestRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        row_id_space: ROW_ID_SPACE.to_string(),
        imported_rows_sha256: imported_rows_sha256.to_string(),
        anchor_name: anchor_name.to_string(),
        target_class,
        row_count: labels.len(),
        positive_count,
        negative_count: labels.len().saturating_sub(positive_count),
        label_counts: label_counts.clone(),
        chunk_rows,
        chunk_count,
    };
    let manifest_value = encode(&manifest)?;
    let chunks = encode_chunks(association_key, labels, chunk_rows)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    fail_if_exists(&router, &manifest_key)?;
    for (key, _) in &chunks {
        fail_if_exists(&router, key)?;
    }
    for (key, value) in &chunks {
        router.put(ColumnFamily::Graph, key, value)?;
    }
    router.put(ColumnFamily::Graph, &manifest_key, &manifest_value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);

    let reopened = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let manifest_readback = read_exact(&reopened, &manifest_key, "MISSING_AFTER_WRITE")?;
    if manifest_readback != manifest_value {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISMATCH",
            "label-anchor manifest Graph CF readback bytes changed after write",
        ));
    }
    let mut chunk_values = Vec::with_capacity(chunks.len());
    for (key, expected) in &chunks {
        let readback = read_exact(&reopened, key, "CHUNK_MISSING_AFTER_WRITE")?;
        if readback != *expected {
            return Err(error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISMATCH",
                "label-anchor chunk Graph CF readback bytes changed after write",
            ));
        }
        chunk_values.push(readback);
    }
    Ok(readback_report(
        cf_root,
        association_key,
        &manifest_key,
        &manifest_readback,
        &chunk_values,
        &manifest,
        true,
    ))
}

pub(crate) fn read(cf_root: &Path, association_key: &str) -> Result<LoadedLabelAnchor> {
    let manifest_key = manifest_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let manifest_value = read_exact(&router, &manifest_key, "MISSING")?;
    let manifest: LabelAnchorManifestRecord = decode(&manifest_value)?;
    validate_manifest(&manifest)?;
    let mut labels = Vec::with_capacity(manifest.row_count);
    let mut chunk_values = Vec::with_capacity(manifest.chunk_count);
    for chunk_index in 0..manifest.chunk_count {
        let key = chunk_key(association_key, chunk_index)?;
        let value = read_exact(&router, &key, "CHUNK_MISSING")?;
        let chunk: LabelAnchorChunkRecord = decode(&value)?;
        validate_chunk(&chunk, association_key, chunk_index, labels.len())?;
        labels.extend(chunk.labels.iter().map(|label| *label != 0));
        chunk_values.push(value);
    }
    if labels.len() != manifest.row_count {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            format!(
                "label-anchor DB rows={} manifest row_count={}",
                labels.len(),
                manifest.row_count
            ),
        ));
    }
    validate_labels(&labels)?;
    let positive_count = labels.iter().filter(|label| **label).count();
    if positive_count != manifest.positive_count
        || labels.len().saturating_sub(positive_count) != manifest.negative_count
    {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor manifest counts do not match chunk labels",
        ));
    }
    let db_readback = readback_report(
        cf_root,
        association_key,
        &manifest_key,
        &manifest_value,
        &chunk_values,
        &manifest,
        true,
    );
    Ok(LoadedLabelAnchor {
        manifest,
        labels,
        db_readback,
    })
}

pub(super) fn validate_labels(labels: &[bool]) -> Result<()> {
    if labels.is_empty() {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_INVALID",
            "label-anchor rows are empty",
        ));
    }
    if labels.iter().all(|label| *label) || labels.iter().all(|label| !*label) {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_INVALID",
            "label-anchor target produces one class",
        ));
    }
    Ok(())
}

fn validate_manifest(manifest: &LabelAnchorManifestRecord) -> Result<()> {
    if manifest.format != FORMAT || manifest.mode != MODE || manifest.row_id_space != ROW_ID_SPACE {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor manifest decoded to the wrong format, mode, or row id space",
        ));
    }
    validate_chunk_rows(manifest.chunk_rows)?;
    if manifest.chunk_count != manifest.row_count.div_ceil(manifest.chunk_rows) {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor manifest chunk_count does not match row_count/chunk_rows",
        ));
    }
    if manifest.positive_count == 0 || manifest.negative_count == 0 {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor manifest lacks both classes",
        ));
    }
    Ok(())
}

fn validate_chunk(
    chunk: &LabelAnchorChunkRecord,
    association_key: &str,
    chunk_index: usize,
    expected_first_row: usize,
) -> Result<()> {
    if chunk.format != FORMAT
        || chunk.mode != MODE
        || chunk.association_key != association_key
        || chunk.chunk_index != chunk_index
        || chunk.first_row_idx != expected_first_row
    {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor chunk decoded to the wrong format, key, or row offset",
        ));
    }
    if chunk.labels.iter().any(|label| *label > 1) {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor chunk contains a non-binary label",
        ));
    }
    Ok(())
}

fn encode_chunks(
    association_key: &str,
    labels: &[bool],
    chunk_rows: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    labels
        .chunks(chunk_rows)
        .enumerate()
        .map(|(chunk_index, labels)| {
            let first_row_idx = chunk_index.saturating_mul(chunk_rows);
            let record = LabelAnchorChunkRecord {
                format: FORMAT.to_string(),
                mode: MODE.to_string(),
                association_key: association_key.to_string(),
                chunk_index,
                first_row_idx,
                labels: labels.iter().map(|label| u8::from(*label)).collect(),
            };
            Ok((chunk_key(association_key, chunk_index)?, encode(&record)?))
        })
        .collect()
}

fn validate_chunk_rows(chunk_rows: usize) -> Result<usize> {
    if chunk_rows == 0 {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor chunk_rows must be > 0",
        ));
    }
    Ok(chunk_rows)
}

fn fail_if_exists(router: &CfRouter, row_key: &[u8]) -> Result<()> {
    if router.get(ColumnFamily::Graph, row_key)?.is_some() {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_EXISTS",
            "label-anchor row already exists in Graph CF",
        ));
    }
    Ok(())
}

fn read_exact(router: &CfRouter, row_key: &[u8], missing_suffix: &str) -> Result<Vec<u8>> {
    router.get(ColumnFamily::Graph, row_key)?.ok_or_else(|| {
        error(
            match missing_suffix {
                "MISSING_AFTER_WRITE" => "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISSING_AFTER_WRITE",
                "CHUNK_MISSING" => "CALYX_FSV_ASSAY_I8BIN_LABELS_CHUNK_DB_MISSING",
                "CHUNK_MISSING_AFTER_WRITE" => {
                    "CALYX_FSV_ASSAY_I8BIN_LABELS_CHUNK_DB_MISSING_AFTER_WRITE"
                }
                _ => "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISSING",
            },
            "label-anchor row missing in Graph CF",
        )
    })
}

fn manifest_key(association_key: &str) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID_KEY",
            "label-anchor association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len() + 9);
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    key.extend_from_slice(b"/manifest");
    Ok(key)
}

fn chunk_key(association_key: &str, chunk_index: usize) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID_KEY",
            "label-anchor association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len() + 24);
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    key.extend_from_slice(b"/chunk/");
    key.extend_from_slice(format!("{chunk_index:016}").as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_ENCODE",
            format!("encode label-anchor record failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_DECODE",
                format!("decode label-anchor record failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
            "label-anchor row has trailing bytes",
        ));
    }
    Ok(record)
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    manifest_key: &[u8],
    manifest_value: &[u8],
    chunk_values: &[Vec<u8>],
    manifest: &LabelAnchorManifestRecord,
    readback_matches: bool,
) -> LabelAnchorDbReadback {
    let chunk_value_bytes = chunk_values.iter().map(Vec::len).sum();
    LabelAnchorDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        manifest_key_sha256: hex_sha256(manifest_key),
        manifest_value_bytes: manifest_value.len(),
        manifest_value_sha256: hex_sha256(manifest_value),
        chunk_count: chunk_values.len(),
        chunk_value_bytes,
        chunk_value_sha256: chunk_values_sha256(chunk_values),
        row_count: manifest.row_count,
        positive_count: manifest.positive_count,
        negative_count: manifest.negative_count,
        readback_matches,
    }
}

fn chunk_values_sha256(chunks: &[Vec<u8>]) -> String {
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update((chunk.len() as u64).to_be_bytes());
        hasher.update(chunk);
    }
    hex_from_digest(&hasher.finalize())
}

pub(super) fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read i8bin label anchors through Calyx/Aster Graph CF",
    }
}

pub(super) fn hex_sha256(bytes: &[u8]) -> String {
    hex_from_digest(&Sha256::digest(bytes))
}

fn hex_from_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "label_store_tests.rs"]
mod tests;
