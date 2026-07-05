use std::path::Path;

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const KEY_PREFIX: &[u8] = b"calyx/a37/admission/v1/";
const VALUE_MAGIC: &[u8] = b"CA37ADM1\0";
const CF_MEMTABLE_CAP: usize = 1_048_576;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct A37AdmissionDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) row_key_sha256: String,
    pub(crate) value_bytes: usize,
    pub(crate) value_sha256: String,
    pub(crate) readback_matches: bool,
}

pub(crate) fn write<T>(
    cf_root: &Path,
    association_key: &str,
    record: &T,
) -> Result<A37AdmissionDbReadback>
where
    T: Serialize,
{
    let row_key = row_key(association_key)?;
    let value = encode(record)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    router.put(ColumnFamily::Graph, &row_key, &value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);

    let reopened = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let readback = reopened
        .get(ColumnFamily::Graph, &row_key)?
        .ok_or_else(|| {
            error(
                "CALYX_FSV_A37_ADMISSION_DB_MISSING",
                "A37 admission row missing after Graph CF write",
            )
        })?;
    if readback != value {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_DB_MISMATCH",
            "A37 admission Graph CF readback bytes changed after write",
        ));
    }
    Ok(readback_report(
        cf_root,
        association_key,
        &row_key,
        &readback,
        true,
    ))
}

pub(crate) fn read<T>(cf_root: &Path, association_key: &str) -> Result<(T, A37AdmissionDbReadback)>
where
    T: DeserializeOwned,
{
    let row_key = row_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let value = router.get(ColumnFamily::Graph, &row_key)?.ok_or_else(|| {
        error(
            "CALYX_FSV_A37_ADMISSION_DB_MISSING",
            "A37 admission row missing in Graph CF",
        )
    })?;
    let record = decode(&value)?;
    let report = readback_report(cf_root, association_key, &row_key, &value, true);
    Ok((record, report))
}

fn row_key(association_key: &str) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_DB_INVALID_KEY",
            "A37 admission association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    Ok(key)
}

fn encode<T>(record: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_FSV_A37_ADMISSION_DB_ENCODE",
            format!("encode A37 admission record failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_A37_ADMISSION_DB_INVALID",
            "A37 admission row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_FSV_A37_ADMISSION_DB_DECODE",
                format!("decode A37 admission record failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_DB_INVALID",
            "A37 admission row has trailing bytes",
        ));
    }
    Ok(record)
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    row_key: &[u8],
    value: &[u8],
    readback_matches: bool,
) -> A37AdmissionDbReadback {
    A37AdmissionDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        row_key_sha256: hex_sha256(row_key),
        value_bytes: value.len(),
        value_sha256: hex_sha256(value),
        readback_matches,
    }
}

fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read the A37 admission record through Calyx/Aster Graph CF",
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
