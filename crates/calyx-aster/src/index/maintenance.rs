//! Atomic secondary-index maintenance staged into layer write batches (PH54 T04).

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Clock, Result};
use serde_json::Value;

use super::{
    BtreeIndex, FieldValue, IndexId, IndexKind, IndexSpec, InvertedIndex, SecondaryIndex,
    field_value_type, invalid_index_input,
};
use crate::cf::ColumnFamily;
use crate::collection::{
    CALYX_INVALID_ARGUMENT, Collection, FieldType, Schema, SecondaryIndexKind, SecondaryIndexSpec,
};
use crate::layers::relational::{CALYX_SCHEMA_VIOLATION, RecordKey, RecordValue, Row};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;

pub const CALYX_INDEX_STALE_ENTRY: &str = "CALYX_INDEX_STALE_ENTRY";

pub struct IndexMaintenance {
    pub indexes: Vec<(IndexSpec, Box<dyn SecondaryIndex>)>,
}

impl IndexMaintenance {
    pub fn for_row(col: &Collection, row: &Row) -> Result<Self> {
        let mut indexes = Vec::new();
        for (ordinal, declared) in col.indexes.iter().enumerate() {
            let Some((spec, index)) = build_index(col, row, declared, ordinal)? else {
                continue;
            };
            indexes.push((spec, index));
        }
        Ok(Self { indexes })
    }

    pub fn is_empty(&self) -> bool {
        self.indexes.is_empty()
    }

    pub fn stage_put<C: Clock>(
        vault: &AsterVault<C>,
        rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        col: &Collection,
        pk: &RecordKey,
        old_row: Option<&Row>,
        new_row: &Row,
    ) -> Result<()> {
        if let Some(old_row) = old_row {
            for (ordinal, declared) in col.indexes.iter().enumerate() {
                if !is_maintained_kind(declared.kind) {
                    continue;
                }
                let field = single_index_field(declared)?;
                let old_value = old_row.get(field).ok_or_else(|| {
                    index_schema_violation(format!("missing indexed field `{field}`"))
                })?;
                let new_value = new_row.get(field).ok_or_else(|| {
                    index_schema_violation(format!("missing indexed field `{field}`"))
                })?;
                if old_value == new_value {
                    continue;
                }
                stage_delete_one(rows, col, pk, declared, ordinal, old_value)?;
                stage_put_one(vault, rows, col, pk, declared, ordinal, new_value)?;
            }
            return Ok(());
        }

        Self::for_row(col, new_row)?.on_put(vault, rows, col, pk, new_row)
    }

    pub fn stage_delete<C: Clock>(
        _vault: &AsterVault<C>,
        rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        col: &Collection,
        pk: &RecordKey,
        old_row: &Row,
    ) -> Result<()> {
        Self::for_row(col, old_row)?.on_delete(rows, col, pk, old_row)
    }

    pub fn on_put<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        col: &Collection,
        pk: &RecordKey,
        row: &Row,
    ) -> Result<()> {
        for (spec, index) in &self.indexes {
            let field_val = indexed_value(row, &spec.on_field)?;
            match spec.kind {
                IndexKind::Btree => {
                    rows.push((
                        ColumnFamily::IndexBtree,
                        index.encode_index_key(field_val, pk)?,
                        Vec::new(),
                    ));
                }
                IndexKind::Inverted => {
                    let inverted = InvertedIndex::new(
                        crate::layers::relational::collection_id(col),
                        spec.clone(),
                    );
                    let stats = inverted.read_stats_at(vault, vault.latest_seq())?;
                    for (key, value) in inverted.encode_put_entries(field_val, pk, stats)? {
                        rows.push((ColumnFamily::IndexInverted, key, value));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn on_delete(
        &self,
        rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        col: &Collection,
        pk: &RecordKey,
        old_row: &Row,
    ) -> Result<()> {
        for (spec, index) in &self.indexes {
            let field_val = indexed_value(old_row, &spec.on_field)?;
            match spec.kind {
                IndexKind::Btree => {
                    rows.push((
                        ColumnFamily::IndexBtree,
                        index.encode_index_key(field_val, pk)?,
                        tombstone_value(),
                    ));
                }
                IndexKind::Inverted => {
                    let inverted = InvertedIndex::new(
                        crate::layers::relational::collection_id(col),
                        spec.clone(),
                    );
                    for (key, value) in inverted.encode_delete_entries(field_val, pk)? {
                        rows.push((ColumnFamily::IndexInverted, key, value));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn document_row(col: &Collection, doc: &Value) -> Result<Row> {
        let mut fields = BTreeMap::new();
        for declared in &col.indexes {
            if !is_maintained_kind(declared.kind) {
                continue;
            }
            let field = single_index_field(declared)?;
            let json = json_path(doc, field).ok_or_else(|| {
                index_schema_violation(format!("missing indexed field `{field}`"))
            })?;
            let expected = schema_field_type(col, field)?;
            fields.insert(field.to_string(), json_to_record_value(json, expected)?);
        }
        Ok(Row { fields })
    }
}

fn stage_put_one<C: Clock>(
    vault: &AsterVault<C>,
    rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    col: &Collection,
    pk: &RecordKey,
    declared: &SecondaryIndexSpec,
    ordinal: usize,
    value: &FieldValue,
) -> Result<()> {
    let row = Row::new([(single_index_field(declared)?, value.clone())]);
    let Some((spec, index)) = build_index(col, &row, declared, ordinal)? else {
        return Ok(());
    };
    IndexMaintenance {
        indexes: vec![(spec, index)],
    }
    .on_put(vault, rows, col, pk, &row)?;
    Ok(())
}

fn stage_delete_one(
    rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    col: &Collection,
    pk: &RecordKey,
    declared: &SecondaryIndexSpec,
    ordinal: usize,
    value: &FieldValue,
) -> Result<()> {
    let row = Row::new([(single_index_field(declared)?, value.clone())]);
    let Some((spec, index)) = build_index(col, &row, declared, ordinal)? else {
        return Ok(());
    };
    IndexMaintenance {
        indexes: vec![(spec, index)],
    }
    .on_delete(rows, col, pk, &row)?;
    Ok(())
}

fn build_index(
    col: &Collection,
    row: &Row,
    declared: &SecondaryIndexSpec,
    ordinal: usize,
) -> Result<Option<(IndexSpec, Box<dyn SecondaryIndex>)>> {
    if !is_maintained_kind(declared.kind) {
        return Ok(None);
    }
    let field = single_index_field(declared)?;
    let field_val = indexed_value(row, field)?;
    let field_type = schema_field_type(col, field)?
        .or_else(|| field_value_type(field_val))
        .ok_or_else(|| invalid_index_input(format!("indexed field `{field}` is NULL")))?;
    let index_id = IndexId::new(u32::try_from(ordinal + 1).map_err(|_| invalid_index_ordinal())?);
    let spec = IndexSpec::new(index_id, &declared.name, declared.kind, field, field_type);
    spec.validate()?;
    let collection_id = crate::layers::relational::collection_id(col);
    let index: Box<dyn SecondaryIndex> = match spec.kind {
        SecondaryIndexKind::Btree => Box::new(BtreeIndex::new(collection_id, spec.clone())),
        SecondaryIndexKind::Inverted => Box::new(InvertedIndex::new(collection_id, spec.clone())),
        _ => return Ok(None),
    };
    Ok(Some((spec, index)))
}

fn is_maintained_kind(kind: SecondaryIndexKind) -> bool {
    matches!(
        kind,
        SecondaryIndexKind::Btree | SecondaryIndexKind::Inverted
    )
}

/// True when `col` declares at least one secondary index this module maintains
/// inline (btree/inverted). Paradigm layers gate **all** index-row construction
/// on this so a write to an unindexed collection costs nothing extra and never
/// coerces its keys into the index value domain. This mirrors the FoundationDB
/// Record Layer rule that index maintenance is driven solely by the indexes
/// that actually exist for the record type.
pub fn collection_has_maintained_index(col: &Collection) -> bool {
    col.indexes.iter().any(|spec| is_maintained_kind(spec.kind))
}

/// True when a maintained index references `field`. Layers building synthetic
/// index rows (KV `ns`/`key`/`value`, TS `series`/`ts`/`value`) use this to
/// include — and type-coerce — only the fields some index will actually read,
/// so an index on one field never fails a write because of an out-of-domain
/// value in an *un*indexed field.
pub fn field_is_indexed(col: &Collection, field: &str) -> bool {
    col.indexes
        .iter()
        .any(|spec| is_maintained_kind(spec.kind) && spec.fields.iter().any(|name| name == field))
}

fn single_index_field(declared: &SecondaryIndexSpec) -> Result<&str> {
    if declared.fields.len() != 1 {
        return Err(invalid_argument(format!(
            "secondary index `{}` must declare exactly one field for PH54 maintenance",
            declared.name
        )));
    }
    Ok(&declared.fields[0])
}

fn indexed_value<'a>(row: &'a Row, field: &str) -> Result<&'a FieldValue> {
    row.get(field)
        .ok_or_else(|| index_schema_violation(format!("missing indexed field `{field}`")))
}

fn schema_field_type(col: &Collection, field: &str) -> Result<Option<FieldType>> {
    let Some(Schema::SchemaFull(fields)) = &col.schema else {
        return Ok(None);
    };
    fields
        .iter()
        .find(|declared| declared.name == field)
        .map(|declared| Some(declared.ty))
        .ok_or_else(|| index_schema_violation(format!("indexed field `{field}` is not in schema")))
}

fn json_path<'a>(doc: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = doc;
    for segment in field.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

fn json_to_record_value(value: &Value, expected: Option<FieldType>) -> Result<RecordValue> {
    match expected {
        Some(FieldType::Bool) => value.as_bool().map(RecordValue::Bool),
        Some(FieldType::I64) => value.as_i64().map(RecordValue::I64),
        Some(FieldType::U64) => value.as_u64().map(RecordValue::U64),
        Some(FieldType::Timestamp) => value.as_i64().map(RecordValue::Timestamp),
        Some(FieldType::F64) => value.as_f64().map(RecordValue::F64),
        Some(FieldType::Text) => value
            .as_str()
            .map(|value| RecordValue::Text(value.to_string())),
        Some(FieldType::Bytes) => value
            .as_str()
            .map(|value| RecordValue::Bytes(value.as_bytes().to_vec())),
        None => infer_json_record_value(value),
    }
    .ok_or_else(|| {
        index_schema_violation(format!(
            "document value `{value}` does not match index field type"
        ))
    })
}

fn infer_json_record_value(value: &Value) -> Option<RecordValue> {
    match value {
        Value::Null => Some(RecordValue::Null),
        Value::Bool(value) => Some(RecordValue::Bool(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(RecordValue::I64)
            .or_else(|| value.as_u64().map(RecordValue::U64))
            .or_else(|| value.as_f64().map(RecordValue::F64)),
        Value::String(value) => Some(RecordValue::Text(value.clone())),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn index_schema_violation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_SCHEMA_VIOLATION,
        message: message.into(),
        remediation: "submit a row/document containing every indexed field",
    }
}

fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "correct the secondary index declaration",
    }
}

fn invalid_index_ordinal() -> CalyxError {
    invalid_argument("secondary index ordinal exceeds u32")
}

#[cfg(test)]
#[path = "maintenance_tests.rs"]
mod tests;
