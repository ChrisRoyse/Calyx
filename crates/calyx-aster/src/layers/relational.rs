//! Relational `(collection, pk) -> row` key-encoding layer.

use std::collections::{BTreeMap, BTreeSet};

use bincode::config;
use calyx_core::{CalyxError, Clock, Modality, Result, Seq};
use serde::{Deserialize, Serialize};

use crate::cf::{ColumnFamily, KeyRange};
use crate::collection::{
    CALYX_COLLECTION_NOT_FOUND, CALYX_INVALID_ARGUMENT, Collection, CollectionMode, FieldType,
    Schema, collection_has_lens, collection_key, decode_collection,
    ingest_collection_constellation,
};
use crate::index::IndexMaintenance;
use crate::vault::AsterVault;
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};

use super::Layer;

pub const CALYX_SCHEMA_VIOLATION: &str = "CALYX_SCHEMA_VIOLATION";
const DISC_RECORD: u8 = 0x01;
const ROW_SCHEMA_VERSION: u16 = 1;
const MAX_RECORD_KEY_BYTES: usize = u16::MAX as usize;
const MAX_ROW_VALUE_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordKey(Vec<u8>);

impl RecordKey {
    pub fn from_u64(value: u64) -> Self {
        Self(value.to_be_bytes().to_vec())
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let key = Self(bytes.into());
        key.validate()?;
        Ok(key)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn validate(&self) -> Result<()> {
        if self.0.is_empty() || self.0.len() > MAX_RECORD_KEY_BYTES {
            return Err(invalid_argument(format!(
                "record key must be 1..={MAX_RECORD_KEY_BYTES} bytes"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RecordValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    Timestamp(i64),
    Null,
    U64(u64),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub fields: BTreeMap<String, RecordValue>,
}

impl Row {
    pub fn new(fields: impl IntoIterator<Item = (impl Into<String>, RecordValue)>) -> Self {
        Self {
            fields: fields
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        }
    }

    pub fn raw_bytes(value: &[u8]) -> Self {
        Self::new([("__value", RecordValue::Bytes(value.to_vec()))])
    }

    pub fn get(&self, name: &str) -> Option<&RecordValue> {
        self.fields.get(name)
    }
}

pub struct RelationalLayer<'a, C: Clock> {
    vault: &'a AsterVault<C>,
}

impl<'a, C: Clock> RelationalLayer<'a, C> {
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }

    pub fn put_record(&self, col: &Collection, pk: &RecordKey, row: &Row) -> Result<Seq> {
        if collection_has_lens(col) {
            validate_row(col, row)?;
            let key = record_key(col, pk)?;
            let value = encode_record_value(row)?;
            let parts = [
                ("record_key", key.as_slice()),
                ("record_value", value.as_slice()),
            ];
            return ingest_collection_constellation(
                self.vault,
                col,
                "records",
                &parts,
                Modality::Structured,
            );
        }
        require_records_mode(col)?;
        validate_row(col, row)?;
        let key = record_key(col, pk)?;
        let value = encode_record_value(row)?;
        let old_row = self
            .vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::Relational, &key)?
            .map(|bytes| decode_record_value(&bytes))
            .transpose()?;
        let mut rows = vec![(ColumnFamily::Relational, key.clone(), value.clone())];
        IndexMaintenance::stage_put(self.vault, &mut rows, col, pk, old_row.as_ref(), row)?;
        let subject = ledger_subject(&key);
        let payload = ledger_payload(col, pk, &key, &value);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-relational".to_string()),
        )
    }

    pub fn get_record(&self, col: &Collection, pk: &RecordKey) -> Result<Option<Row>> {
        self.get_record_at(self.vault.latest_seq(), col, pk)
    }

    pub fn get_record_at(
        &self,
        snapshot: Seq,
        col: &Collection,
        pk: &RecordKey,
    ) -> Result<Option<Row>> {
        require_records_mode(col)?;
        let key = record_key(col, pk)?;
        self.vault
            .read_cf_at(snapshot, ColumnFamily::Relational, &key)?
            .map(|bytes| decode_record_value(&bytes))
            .transpose()
    }

    pub fn range(
        &self,
        col: &Collection,
        start: &RecordKey,
        end: &RecordKey,
        limit: usize,
    ) -> Result<Vec<Row>> {
        self.range_at(self.vault.latest_seq(), col, start, end, limit)
    }

    pub fn range_at(
        &self,
        snapshot: Seq,
        col: &Collection,
        start: &RecordKey,
        end: &RecordKey,
        limit: usize,
    ) -> Result<Vec<Row>> {
        require_records_mode(col)?;
        start.validate()?;
        end.validate()?;
        if limit == 0 || start.as_bytes() >= end.as_bytes() {
            return Ok(Vec::new());
        }
        let rows = self.vault.scan_cf_range_at(
            snapshot,
            ColumnFamily::Relational,
            &record_key_range(col, start, end)?,
        )?;
        rows.into_iter()
            .take(limit)
            .map(|(_, value)| decode_record_value(&value))
            .collect()
    }

    pub fn join_by_ref(
        &self,
        col_a: &Collection,
        pk_a: &RecordKey,
        col_b_name: &str,
        fk_field: &str,
    ) -> Result<Option<Row>> {
        self.join_by_ref_at(self.vault.latest_seq(), col_a, pk_a, col_b_name, fk_field)
    }

    pub fn join_by_ref_at(
        &self,
        snapshot: Seq,
        col_a: &Collection,
        pk_a: &RecordKey,
        col_b_name: &str,
        fk_field: &str,
    ) -> Result<Option<Row>> {
        let Some(row_a) = self.get_record_at(snapshot, col_a, pk_a)? else {
            return Ok(None);
        };
        let fk = row_a
            .get(fk_field)
            .ok_or_else(|| schema_violation(format!("missing foreign-key field `{fk_field}`")))
            .and_then(record_key_from_value)?;
        let col_b = collection_at(self.vault, snapshot, col_b_name)?;
        self.get_record_at(snapshot, &col_b, &fk)
    }
}

impl<C> Layer for RelationalLayer<'_, C>
where
    C: Clock + Send + Sync,
{
    fn collection_mode() -> CollectionMode {
        CollectionMode::Records
    }

    fn put(&self, col: &Collection, key: &[u8], value: &[u8]) -> Result<()> {
        self.put_record(
            col,
            &RecordKey::from_bytes(key.to_vec())?,
            &Row::raw_bytes(value),
        )?;
        Ok(())
    }

    fn get(&self, col: &Collection, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.get_record(col, &RecordKey::from_bytes(key.to_vec())?)?
            .map(|row| encode_row_bytes(&row))
            .transpose()
    }

    fn range(
        &self,
        col: &Collection,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<Vec<u8>>> {
        self.range(
            col,
            &RecordKey::from_bytes(start.to_vec())?,
            &RecordKey::from_bytes(end.to_vec())?,
            limit,
        )?
        .into_iter()
        .map(|row| encode_row_bytes(&row))
        .collect()
    }
}

pub fn collection_id(col: &Collection) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx:relational:collection:v1");
    hasher.update(&col.tenant.0.to_be_bytes());
    hasher.update(&(col.name.len() as u16).to_be_bytes());
    hasher.update(col.name.as_bytes());
    u64::from_be_bytes(hasher.finalize().as_bytes()[0..8].try_into().unwrap())
}

pub fn record_key(col: &Collection, pk: &RecordKey) -> Result<Vec<u8>> {
    pk.validate()?;
    let mut key = record_prefix(col);
    key.extend_from_slice(&(pk.as_bytes().len() as u16).to_be_bytes());
    key.extend_from_slice(pk.as_bytes());
    Ok(key)
}

pub fn encode_record_value(row: &Row) -> Result<Vec<u8>> {
    let row_bytes = encode_row_bytes(row)?;
    let mut out = Vec::with_capacity(2 + row_bytes.len());
    out.extend_from_slice(&ROW_SCHEMA_VERSION.to_be_bytes());
    out.extend_from_slice(&row_bytes);
    Ok(out)
}

pub fn decode_record_value(bytes: &[u8]) -> Result<Row> {
    if bytes.len() < 2 {
        return Err(corrupt_row("relational row is shorter than schema version"));
    }
    let version = u16::from_be_bytes([bytes[0], bytes[1]]);
    if version != ROW_SCHEMA_VERSION {
        return Err(corrupt_row(format!(
            "unsupported relational row schema version {version}"
        )));
    }
    let (row, read): (Row, usize) =
        bincode::serde::decode_from_slice(&bytes[2..], config::standard())
            .map_err(|error| corrupt_row(format!("decode relational row: {error}")))?;
    if read != bytes.len() - 2 {
        return Err(corrupt_row("relational row has trailing bytes"));
    }
    validate_field_names(&row)?;
    Ok(row)
}

fn record_prefix(col: &Collection) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    key.push(DISC_RECORD);
    key.extend_from_slice(&collection_id(col).to_be_bytes());
    key
}

fn record_key_range(col: &Collection, start: &RecordKey, end: &RecordKey) -> Result<KeyRange> {
    Ok(KeyRange {
        start: record_key(col, start)?,
        end: Some(record_key(col, end)?),
    })
}

fn validate_row(col: &Collection, row: &Row) -> Result<()> {
    validate_field_names(row)?;
    for value in row.fields.values() {
        validate_value(value)?;
    }
    if let Some(Schema::SchemaFull(fields)) = &col.schema {
        let declared = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<BTreeSet<_>>();
        for field in fields {
            match row.fields.get(&field.name) {
                Some(RecordValue::Null) if field.nullable => {}
                Some(value) if value_matches_type(value, field.ty) => {}
                Some(RecordValue::Null) => {
                    return Err(schema_violation(format!(
                        "field `{}` is null but not nullable",
                        field.name
                    )));
                }
                Some(value) => {
                    return Err(schema_violation(format!(
                        "field `{}` expected {:?}, got {:?}",
                        field.name, field.ty, value
                    )));
                }
                None if field.nullable => {}
                None => return Err(schema_violation(format!("missing field `{}`", field.name))),
            }
        }
        for name in row.fields.keys() {
            if !declared.contains(name.as_str()) {
                return Err(schema_violation(format!("unexpected field `{name}`")));
            }
        }
    }
    Ok(())
}

fn validate_field_names(row: &Row) -> Result<()> {
    for name in row.fields.keys() {
        if name.is_empty() || name.len() > 128 {
            return Err(schema_violation(
                "row field names must be non-empty and <=128 bytes",
            ));
        }
    }
    Ok(())
}

fn validate_value(value: &RecordValue) -> Result<()> {
    if let RecordValue::F64(value) = value
        && !value.is_finite()
    {
        return Err(schema_violation("F64 row value must be finite"));
    }
    Ok(())
}

fn value_matches_type(value: &RecordValue, ty: FieldType) -> bool {
    matches!(
        (value, ty),
        (RecordValue::Bool(_), FieldType::Bool)
            | (RecordValue::I64(_), FieldType::I64)
            | (RecordValue::F64(_), FieldType::F64)
            | (RecordValue::Text(_), FieldType::Text)
            | (RecordValue::Bytes(_), FieldType::Bytes)
            | (RecordValue::Timestamp(_), FieldType::Timestamp)
            | (RecordValue::U64(_), FieldType::U64)
    )
}

fn record_key_from_value(value: &RecordValue) -> Result<RecordKey> {
    match value {
        RecordValue::Bytes(bytes) => RecordKey::from_bytes(bytes.clone()),
        RecordValue::I64(value) | RecordValue::Timestamp(value) if *value >= 0 => {
            Ok(RecordKey::from_u64(*value as u64))
        }
        RecordValue::U64(value) => Ok(RecordKey::from_u64(*value)),
        RecordValue::Text(value) => RecordKey::from_bytes(value.as_bytes().to_vec()),
        _ => Err(schema_violation(
            "foreign-key field must be Bytes, non-negative I64/Timestamp, U64, or Text",
        )),
    }
}

fn collection_at<C: Clock>(vault: &AsterVault<C>, snapshot: Seq, name: &str) -> Result<Collection> {
    let key = collection_key(name)?;
    let bytes = vault
        .read_cf_at(snapshot, ColumnFamily::Collections, &key)?
        .ok_or_else(|| collection_not_found(name))?;
    decode_collection(&bytes)
}

fn require_records_mode(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::Records {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "relational layer requires Records collection, got {:?}",
            col.mode
        )))
    }
}

fn encode_row_bytes(row: &Row) -> Result<Vec<u8>> {
    validate_field_names(row)?;
    let bytes = bincode::serde::encode_to_vec(row, config::standard())
        .map_err(|error| corrupt_row(format!("encode relational row: {error}")))?;
    if bytes.len() > MAX_ROW_VALUE_BYTES {
        return Err(invalid_argument(format!(
            "encoded relational row exceeds {MAX_ROW_VALUE_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn ledger_subject(record_key: &[u8]) -> SubjectId {
    SubjectId::Query(blake3::hash(record_key).as_bytes().to_vec())
}

fn ledger_payload(col: &Collection, pk: &RecordKey, key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("collection_id", format!("{:016x}", collection_id(col)))
        .insert_str("pk_hash", blake3::hash(pk.as_bytes()).to_hex().to_string())
        .insert_str("record_hash", blake3::hash(key).to_hex().to_string())
        .insert_str("value_hash", blake3::hash(value).to_hex().to_string());
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn schema_violation(message: impl Into<String>) -> CalyxError {
    relational_error(
        CALYX_SCHEMA_VIOLATION,
        message,
        "submit a row matching the collection SchemaFull definition",
    )
}

fn invalid_argument(message: impl Into<String>) -> CalyxError {
    relational_error(CALYX_INVALID_ARGUMENT, message, "fix the relational input")
}

fn collection_not_found(name: &str) -> CalyxError {
    relational_error(
        CALYX_COLLECTION_NOT_FOUND,
        format!("collection `{name}` was not found"),
        "create the referenced collection before joining by reference",
    )
}

fn corrupt_row(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

fn relational_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

#[cfg(test)]
mod tests;
