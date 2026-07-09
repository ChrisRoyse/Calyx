use calyx_aster::cf::{ColumnFamily, KeyRange, prefix_range};
use calyx_aster::collection::{Collection, FieldType};
use calyx_aster::index::IndexMaintenance;
use calyx_aster::layers::blob::BlobId;
use calyx_aster::layers::relational::{
    RecordKey, RecordValue, Row, collection_id as relational_collection_id, encode_record_value,
};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::txn::IsolationLevel;
use calyx_core::CalyxError;
use calyx_ledger::{PayloadBuilder, RedactionPolicy, SubjectId};
use serde_json::{Value, json};

use crate::engine::hex::encode_hex;

use super::super::super::{EngineResult, VaultHandle};
use super::super::params::{BytesParam, IsolationParam, KeyParam, RecordValueParam, RowParam};
use super::super::{CALYX_LEAPABLE_REL_NOT_FOUND, CALYX_LEAPABLE_STORAGE_INPUT_INVALID};
use super::storage_error;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;
const REL_RECORD_DISC: u8 = 0x01;

type StorageWriteRows = Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>;

pub(in crate::engine::storage) fn row_from_param(row: RowParam) -> EngineResult<Row> {
    Ok(Row {
        fields: row
            .into_iter()
            .map(|(name, value)| record_value_from_param(value).map(|value| (name, value)))
            .collect::<EngineResult<_>>()?,
    })
}

pub(in crate::engine::storage) fn record_key_from_param(
    value: KeyParam,
) -> EngineResult<RecordKey> {
    let present = usize::from(value.u64.is_some())
        + usize::from(value.text.is_some())
        + usize::from(value.hex.is_some())
        + usize::from(value.bytes.is_some());
    if present != 1 {
        return Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "key must contain exactly one of u64, text, hex, or bytes",
            "send one canonical key representation",
        )
        .into());
    }
    if let Some(value) = value.u64 {
        return Ok(RecordKey::from_u64(value));
    }
    let bytes = if let Some(text) = value.text {
        text.into_bytes()
    } else if let Some(hex) = value.hex {
        decode_hex(&hex)?
    } else {
        value.bytes.expect("present checked")
    };
    Ok(RecordKey::from_bytes(bytes)?)
}

pub(in crate::engine::storage) fn record_value_from_param(
    value: RecordValueParam,
) -> EngineResult<RecordValue> {
    let present = usize::from(value.bool.is_some())
        + usize::from(value.i64.is_some())
        + usize::from(value.u64.is_some())
        + usize::from(value.f64.is_some())
        + usize::from(value.text.is_some())
        + usize::from(value.hex.is_some())
        + usize::from(value.bytes.is_some())
        + usize::from(value.timestamp.is_some())
        + usize::from(value.null.unwrap_or(false));
    if present != 1 {
        return Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "record value must contain exactly one typed field",
            "send one of bool/i64/u64/f64/text/hex/bytes/timestamp/null",
        )
        .into());
    }
    Ok(match value {
        RecordValueParam { bool: Some(v), .. } => RecordValue::Bool(v),
        RecordValueParam { i64: Some(v), .. } => RecordValue::I64(v),
        RecordValueParam { u64: Some(v), .. } => RecordValue::U64(v),
        RecordValueParam { f64: Some(v), .. } => RecordValue::F64(v),
        RecordValueParam { text: Some(v), .. } => RecordValue::Text(v),
        RecordValueParam { hex: Some(v), .. } => RecordValue::Bytes(decode_hex(&v)?),
        RecordValueParam { bytes: Some(v), .. } => RecordValue::Bytes(v),
        RecordValueParam {
            timestamp: Some(v), ..
        } => RecordValue::Timestamp(v),
        _ => RecordValue::Null,
    })
}

pub(in crate::engine::storage) fn bytes_from_param(value: BytesParam) -> EngineResult<Vec<u8>> {
    let present = usize::from(value.text.is_some())
        + usize::from(value.hex.is_some())
        + usize::from(value.bytes.is_some());
    if present != 1 {
        return Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "bytes input must contain exactly one of text, hex, or bytes",
            "send one canonical bytes representation",
        )
        .into());
    }
    if let Some(text) = value.text {
        return Ok(text.into_bytes());
    }
    if let Some(hex) = value.hex {
        return decode_hex(&hex);
    }
    Ok(value.bytes.expect("present checked"))
}

pub(in crate::engine::storage) fn validate_limit(limit: Option<usize>) -> EngineResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if (1..=MAX_LIMIT).contains(&limit) {
        return Ok(limit);
    }
    Err(storage_error(
        CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
        format!("limit {limit} is outside 1..={MAX_LIMIT}"),
        "choose a positive bounded limit",
    )
    .into())
}

pub(in crate::engine::storage) fn rel_collection_range(col: &Collection) -> KeyRange {
    prefix_range(&rel_prefix(col))
}

fn rel_prefix(col: &Collection) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(REL_RECORD_DISC);
    prefix.extend_from_slice(&relational_collection_id(col).to_be_bytes());
    prefix
}

pub(in crate::engine::storage) fn rel_pk_from_full_key(
    col: &Collection,
    key: &[u8],
) -> EngineResult<RecordKey> {
    let prefix = rel_prefix(col);
    let rest = key.strip_prefix(prefix.as_slice()).ok_or_else(|| {
        storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "relational scan returned a key outside the collection prefix",
            "inspect the relational CF for keyspace corruption",
        )
    })?;
    let len = rest.get(..2).ok_or_else(|| {
        storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "relational key missing primary-key length",
            "inspect the relational CF for keyspace corruption",
        )
    })?;
    let len = u16::from_be_bytes([len[0], len[1]]) as usize;
    let bytes = rest.get(2..2 + len).ok_or_else(|| {
        storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "relational key primary-key length exceeds stored bytes",
            "inspect the relational CF for keyspace corruption",
        )
    })?;
    if 2 + len != rest.len() {
        return Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "relational key has trailing primary-key bytes",
            "inspect the relational CF for keyspace corruption",
        )
        .into());
    }
    Ok(RecordKey::from_bytes(bytes.to_vec())?)
}

pub(in crate::engine::storage) fn blob_id_from_hex(value: &str) -> EngineResult<BlobId> {
    BlobId::from_slice(&decode_hex(value)?).map_err(Into::into)
}

pub(in crate::engine::storage) fn isolation_level(value: IsolationParam) -> IsolationLevel {
    match value {
        IsolationParam::ReadCommitted => IsolationLevel::ReadCommitted,
        IsolationParam::Serializable => IsolationLevel::Serializable,
    }
}

pub(in crate::engine::storage) fn rel_not_found(collection: &str, pk: &RecordKey) -> CalyxError {
    storage_error(
        CALYX_LEAPABLE_REL_NOT_FOUND,
        format!("row {:?}/{} was not found", collection, hex(pk.as_bytes())),
        "insert the row before updating or deleting it",
    )
}

pub(in crate::engine::storage) fn rel_delete_rows(
    handle: &VaultHandle,
    col: &Collection,
    pk: &RecordKey,
    old_row: &Row,
) -> EngineResult<StorageWriteRows> {
    let key = calyx_aster::layers::relational::record_key(col, pk)?;
    let mut rows = vec![(ColumnFamily::Relational, key, tombstone_value())];
    let write_col = super::served_write_collection(col);
    IndexMaintenance::stage_delete(&handle.vault, &mut rows, &write_col, pk, old_row)?;
    Ok(rows)
}

pub(in crate::engine::storage) fn rel_delete_payload(
    col: &Collection,
    pk: &RecordKey,
    old_row: &Row,
) -> EngineResult<Vec<u8>> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("collection", &col.name)
        .insert_str("pk_hash", blake3::hash(pk.as_bytes()).to_hex().to_string())
        .insert_str(
            "old_value_hash",
            blake3::hash(&encode_record_value(old_row)?)
                .to_hex()
                .to_string(),
        );
    Ok(RedactionPolicy::default().apply_to_payload(&payload))
}

pub(in crate::engine::storage) fn ledger_subject(key: &[u8]) -> SubjectId {
    SubjectId::Query(blake3::hash(key).as_bytes().to_vec())
}

pub(super) fn record_field_type(value: &RecordValue) -> Option<FieldType> {
    match value {
        RecordValue::Bool(_) => Some(FieldType::Bool),
        RecordValue::I64(_) => Some(FieldType::I64),
        RecordValue::U64(_) => Some(FieldType::U64),
        RecordValue::F64(_) => Some(FieldType::F64),
        RecordValue::Text(_) => Some(FieldType::Text),
        RecordValue::Bytes(_) => Some(FieldType::Bytes),
        RecordValue::Timestamp(_) => Some(FieldType::Timestamp),
        RecordValue::Null => None,
    }
}

pub(in crate::engine::storage) fn rel_row_json(pk: &RecordKey, row: &Row) -> Value {
    json!({"pk": key_value(pk), "row": row_value(row)})
}

pub(in crate::engine::storage) fn row_value(row: &Row) -> Value {
    Value::Object(
        row.fields
            .iter()
            .map(|(name, value)| (name.clone(), record_value_json(value)))
            .collect(),
    )
}

fn record_value_json(value: &RecordValue) -> Value {
    match value {
        RecordValue::Bool(value) => json!({"bool": value}),
        RecordValue::I64(value) => json!({"i64": value}),
        RecordValue::U64(value) => json!({"u64": value}),
        RecordValue::F64(value) => json!({"f64": value}),
        RecordValue::Text(value) => json!({"text": value}),
        RecordValue::Bytes(value) => json!({"hex": hex(value)}),
        RecordValue::Timestamp(value) => json!({"timestamp": value}),
        RecordValue::Null => json!({"null": true}),
    }
}

pub(in crate::engine::storage) fn key_value(pk: &RecordKey) -> Value {
    let bytes = pk.as_bytes();
    json!({
        "hex": hex(bytes),
        "text": std::str::from_utf8(bytes).ok(),
        "u64": if bytes.len() == 8 {
            Some(u64::from_be_bytes(bytes.try_into().expect("len checked")))
        } else {
            None
        }
    })
}

pub(in crate::engine::storage) fn blob_manifest_value(
    manifest: calyx_aster::layers::blob::BlobManifest,
) -> Value {
    json!({
        "total_bytes": manifest.total_bytes,
        "chunk_count": manifest.chunk_count,
        "content_hash": hex(&manifest.content_hash),
        "cold_tier": manifest.cold_tier,
        "created_at_ms": manifest.created_at_ms
    })
}

pub(in crate::engine::storage) fn bytes_value(bytes: &[u8], include_text: bool) -> Value {
    let mut value = json!({
        "hex": hex(bytes),
        "len": bytes.len()
    });
    if include_text
        && let (Some(text), Some(object)) = (std::str::from_utf8(bytes).ok(), value.as_object_mut())
    {
        object.insert("text".to_string(), Value::from(text));
    }
    value
}

pub(in crate::engine::storage) fn decode_hex(value: &str) -> EngineResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "hex input must contain an even number of characters",
            "send hexadecimal bytes without separators",
        )
        .into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Ok((hex_value(chunk[0])? << 4) | hex_value(chunk[1])?))
        .collect()
}

fn hex_value(byte: u8) -> EngineResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(storage_error(
            CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
            "hex input contains a non-hex character",
            "send hexadecimal characters 0-9, a-f, or A-F",
        )
        .into()),
    }
}

pub(in crate::engine::storage) fn hex(bytes: &[u8]) -> String {
    encode_hex(bytes)
}
