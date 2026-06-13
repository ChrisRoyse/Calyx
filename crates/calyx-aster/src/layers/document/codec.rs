use std::str;

use bincode::config;
use calyx_core::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use serde_json::{Number, Value};

use crate::collection::Collection;

use super::errors::{corrupt_doc, invalid_argument};

pub(super) const DISC_DOCUMENT: u8 = 0x02;
pub(super) const DOC_ID_BYTES: usize = 16;
const COLLECTION_ID_BYTES: usize = 8;
const KEY_PREFIX_BYTES: usize = 1 + COLLECTION_ID_BYTES + DOC_ID_BYTES;
const MAX_PATH_SEGMENT_BYTES: usize = u8::MAX as usize;
const MAX_DOCUMENT_LEAF_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocId([u8; DOC_ID_BYTES]);

impl DocId {
    pub const fn from_bytes(bytes: [u8; DOC_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != DOC_ID_BYTES {
            return Err(invalid_argument(format!(
                "document id must be {DOC_ID_BYTES} bytes"
            )));
        }
        let mut out = [0_u8; DOC_ID_BYTES];
        out.copy_from_slice(bytes);
        Ok(Self(out))
    }

    pub fn from_text(value: &str) -> Self {
        let hash = blake3::hash(value.as_bytes());
        let mut out = [0_u8; DOC_ID_BYTES];
        out.copy_from_slice(&hash.as_bytes()[..DOC_ID_BYTES]);
        Self(out)
    }

    pub const fn as_bytes(&self) -> &[u8; DOC_ID_BYTES] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum DocumentCell {
    Leaf(StoredJsonValue),
    Tombstone,
}

impl DocumentCell {
    pub(super) fn leaf(value: Value) -> Result<Self> {
        Ok(Self::Leaf(StoredJsonValue::from_value(value)?))
    }

    pub(super) fn into_leaf_value(self) -> Result<Option<Value>> {
        match self {
            Self::Leaf(value) => value.into_value().map(Some),
            Self::Tombstone => Ok(None),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum StoredJsonValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Array(Vec<StoredJsonValue>),
    Object(BTreeMap<String, StoredJsonValue>),
}

impl StoredJsonValue {
    fn from_value(value: Value) -> Result<Self> {
        Ok(match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(value),
            Value::Number(value) => number_from_value(&value)?,
            Value::String(value) => Self::String(value),
            Value::Array(values) => Self::Array(
                values
                    .into_iter()
                    .map(Self::from_value)
                    .collect::<Result<Vec<_>>>()?,
            ),
            Value::Object(map) => Self::Object(
                map.into_iter()
                    .map(|(key, value)| Ok((key, Self::from_value(value)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
            ),
        })
    }

    fn into_value(self) -> Result<Value> {
        Ok(match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::I64(value) => Value::Number(value.into()),
            Self::U64(value) => Value::Number(value.into()),
            Self::F64(value) => Value::Number(
                Number::from_f64(value)
                    .ok_or_else(|| corrupt_doc("document F64 leaf is not finite"))?,
            ),
            Self::String(value) => Value::String(value),
            Self::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(Self::into_value)
                    .collect::<Result<Vec<_>>>()?,
            ),
            Self::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| Ok((key, value.into_value()?)))
                    .collect::<Result<serde_json::Map<_, _>>>()?,
            ),
        })
    }
}

fn number_from_value(value: &Number) -> Result<StoredJsonValue> {
    if let Some(value) = value.as_i64() {
        return Ok(StoredJsonValue::I64(value));
    }
    if let Some(value) = value.as_u64() {
        return Ok(StoredJsonValue::U64(value));
    }
    let value = value
        .as_f64()
        .ok_or_else(|| invalid_argument("document number cannot be represented"))?;
    if !value.is_finite() {
        return Err(invalid_argument("document F64 leaf must be finite"));
    }
    Ok(StoredJsonValue::F64(value))
}

pub fn collection_id(col: &Collection) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx:document:collection:v1");
    hasher.update(&col.tenant.0.to_be_bytes());
    hasher.update(&(col.name.len() as u16).to_be_bytes());
    hasher.update(col.name.as_bytes());
    u64::from_be_bytes(hasher.finalize().as_bytes()[0..8].try_into().unwrap())
}

pub fn document_prefix(col: &Collection, doc_id: DocId) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_PREFIX_BYTES);
    key.push(DISC_DOCUMENT);
    key.extend_from_slice(&collection_id(col).to_be_bytes());
    key.extend_from_slice(doc_id.as_bytes());
    key
}

pub fn document_key(col: &Collection, doc_id: DocId, path: &[&str]) -> Result<Vec<u8>> {
    let path = path_segments(path)?;
    document_key_from_segments(col, doc_id, &path)
}

pub(super) fn document_path_prefix(
    col: &Collection,
    doc_id: DocId,
    path: &[String],
) -> Result<Vec<u8>> {
    document_key_from_segments(col, doc_id, path)
}

pub(super) fn document_key_from_segments(
    col: &Collection,
    doc_id: DocId,
    path: &[String],
) -> Result<Vec<u8>> {
    let mut key = document_prefix(col, doc_id);
    for segment in path {
        append_segment(&mut key, segment)?;
    }
    Ok(key)
}

pub(super) fn validate_segment(segment: &str) -> Result<()> {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_SEGMENT_BYTES {
        return Err(invalid_argument(format!(
            "document path segment must be 1..={MAX_PATH_SEGMENT_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(super) fn path_segments(path: &[&str]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(path.len());
    for segment in path {
        validate_segment(segment)?;
        out.push((*segment).to_string());
    }
    Ok(out)
}

pub(super) fn encode_cell(cell: &DocumentCell) -> Result<Vec<u8>> {
    let bytes = bincode::serde::encode_to_vec(cell, config::standard())
        .map_err(|error| corrupt_doc(format!("encode document cell: {error}")))?;
    if bytes.len() > MAX_DOCUMENT_LEAF_BYTES {
        return Err(invalid_argument(format!(
            "encoded document leaf exceeds {MAX_DOCUMENT_LEAF_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

pub(super) fn decode_cell(bytes: &[u8]) -> Result<DocumentCell> {
    let (cell, read): (DocumentCell, usize) =
        bincode::serde::decode_from_slice(bytes, config::standard())
            .map_err(|error| corrupt_doc(format!("decode document cell: {error}")))?;
    if read != bytes.len() {
        return Err(corrupt_doc("document cell has trailing bytes"));
    }
    Ok(cell)
}

pub(super) fn parse_document_key(key: &[u8]) -> Result<([u8; DOC_ID_BYTES], Vec<String>)> {
    if key.len() < KEY_PREFIX_BYTES || key[0] != DISC_DOCUMENT {
        return Err(corrupt_doc("document key has an invalid prefix"));
    }
    let mut doc_id = [0_u8; DOC_ID_BYTES];
    doc_id.copy_from_slice(&key[1 + COLLECTION_ID_BYTES..KEY_PREFIX_BYTES]);
    let mut path = Vec::new();
    let mut offset = KEY_PREFIX_BYTES;
    while offset < key.len() {
        let len = key[offset] as usize;
        offset += 1;
        let bytes = key
            .get(offset..offset + len)
            .ok_or_else(|| corrupt_doc("document key has a truncated path segment"))?;
        if bytes.is_empty() {
            return Err(corrupt_doc("document key contains an empty path segment"));
        }
        let segment = str::from_utf8(bytes)
            .map_err(|error| corrupt_doc(format!("document key path is not UTF-8: {error}")))?;
        path.push(segment.to_string());
        offset += len;
    }
    Ok((doc_id, path))
}

fn append_segment(key: &mut Vec<u8>, segment: &str) -> Result<()> {
    validate_segment(segment)?;
    key.push(segment.len() as u8);
    key.extend_from_slice(segment.as_bytes());
    Ok(())
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
