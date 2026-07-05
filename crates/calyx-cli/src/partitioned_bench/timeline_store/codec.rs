use bincode::config;
use calyx_core::{CalyxError, Result};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const KEY_PREFIX: &[u8] = b"calyx/partitioned-rrf/timeline/v1/";
const VALUE_MAGIC: &[u8] = b"CRRFTL1\0";

pub(super) fn manifest_key(association_key: &str) -> Result<Vec<u8>> {
    scoped_key(association_key, b"/manifest", None)
}

pub(super) fn chunk_key(association_key: &str, chunk_index: usize) -> Result<Vec<u8>> {
    scoped_key(association_key, b"/chunk/", Some(chunk_index))
}

pub(super) fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_ENCODE",
            format!("encode timeline record failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_DECODE",
                format!("decode timeline record failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline row has trailing bytes",
        ));
    }
    Ok(record)
}

pub(super) fn chunk_values_sha256(values: &[Vec<u8>]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hex_from_digest(hasher.finalize())
}

pub(super) fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read partitioned RRF timelines through Calyx/Aster Graph CF",
    }
}

pub(super) fn hex_sha256(bytes: &[u8]) -> String {
    hex_from_digest(Sha256::digest(bytes))
}

fn scoped_key(association_key: &str, tag: &[u8], chunk_index: Option<usize>) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID_KEY",
            "timeline association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len() + tag.len() + 8);
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    key.extend_from_slice(tag);
    if let Some(chunk_index) = chunk_index {
        key.extend_from_slice(&(chunk_index as u64).to_be_bytes());
    }
    Ok(key)
}

fn hex_from_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
