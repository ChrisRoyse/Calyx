use std::collections::BTreeSet;

use calyx_aster::cf::{ColumnFamily, prefix_range};
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldType, RetentionPolicy, Schema,
    SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
};
use calyx_aster::index::{IndexId, IndexSpec};
use calyx_aster::layers::document::{DocId, collection_id as doc_collection_id};
use calyx_aster::layers::relational::{
    RecordKey, RecordValue, Row, collection_id as rel_collection_id,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, LedgerRef, Result, Seq, VaultStore};
use serde_json::Value;

use crate::error::{CALYX_SEXTANT_QUERY_SHAPE, sextant_error};

use super::ExecState;
use crate::query::{AggSpec, FieldOp, FieldPredicate, ProvenancedRow};

pub(super) fn runtime_index(
    collection: &Collection,
    declared: &SecondaryIndexSpec,
    predicates: &[FieldPredicate],
) -> Result<IndexSpec> {
    if declared.kind != SecondaryIndexKind::Btree {
        return Err(shape("executor supports only btree relational index scans"));
    }
    let field = declared
        .fields
        .first()
        .ok_or_else(|| shape("btree index requires one field"))?;
    let ordinal = collection
        .indexes
        .iter()
        .position(|item| item == declared)
        .ok_or_else(|| shape("chosen index is not declared on the collection"))?;
    let field_type = schema_field_type(collection, field)
        .or_else(|| {
            predicates
                .iter()
                .find(|p| p.field == *field)
                .and_then(json_type)
        })
        .ok_or_else(|| shape("cannot infer btree index field type"))?;
    Ok(IndexSpec::new(
        IndexId::new((ordinal + 1) as u32),
        &declared.name,
        declared.kind,
        field,
        field_type,
    ))
}

pub(super) fn index_bounds(
    spec: &IndexSpec,
    predicates: &[FieldPredicate],
) -> Result<(Option<RecordValue>, Option<RecordValue>)> {
    let mut gte = None;
    let mut lte = None;
    for predicate in predicates.iter().filter(|p| p.field == spec.on_field) {
        let value = json_to_record_value(&predicate.value, spec.field_type)?;
        match predicate.op {
            FieldOp::Eq => {
                gte = Some(value.clone());
                lte = Some(value);
            }
            FieldOp::Gt | FieldOp::Gte => gte = Some(value),
            FieldOp::Lt | FieldOp::Lte => lte = Some(value),
            FieldOp::Ne | FieldOp::Contains => {}
        }
    }
    Ok((gte, lte))
}

pub(super) fn row_matches(row: &Row, predicates: &[FieldPredicate]) -> bool {
    predicates.iter().all(|predicate| {
        row.get(&predicate.field)
            .is_some_and(|value| predicate_matches(value, predicate))
    })
}

fn predicate_matches(value: &RecordValue, predicate: &FieldPredicate) -> bool {
    match predicate.op {
        FieldOp::Eq => compare_json(value, &predicate.value).is_some_and(|order| order.is_eq()),
        FieldOp::Ne => !compare_json(value, &predicate.value).is_some_and(|order| order.is_eq()),
        FieldOp::Gt => compare_json(value, &predicate.value).is_some_and(|o| o.is_gt()),
        FieldOp::Gte => compare_json(value, &predicate.value).is_some_and(|o| !o.is_lt()),
        FieldOp::Lt => compare_json(value, &predicate.value).is_some_and(|o| o.is_lt()),
        FieldOp::Lte => compare_json(value, &predicate.value).is_some_and(|o| !o.is_gt()),
        FieldOp::Contains => contains_json(value, &predicate.value),
    }
}

pub(super) fn compare_json(value: &RecordValue, json: &Value) -> Option<std::cmp::Ordering> {
    numeric(value)
        .zip(json.as_f64())
        .map(|(left, right)| left.total_cmp(&right))
        .or_else(|| match (value, json.as_str()) {
            (RecordValue::Text(left), Some(right)) => Some(left.as_str().cmp(right)),
            _ => None,
        })
}

pub(super) fn contains_json(value: &RecordValue, json: &Value) -> bool {
    match (value, json) {
        (RecordValue::Text(left), Value::String(right)) => left.contains(right),
        (RecordValue::Bytes(left), Value::String(right)) => left
            .windows(right.len())
            .any(|part| part == right.as_bytes()),
        _ => false,
    }
}

pub(super) fn numeric(value: &RecordValue) -> Option<f64> {
    match value {
        RecordValue::I64(value) | RecordValue::Timestamp(value) => Some(*value as f64),
        RecordValue::U64(value) => Some(*value as f64),
        RecordValue::F64(value) => Some(*value),
        _ => None,
    }
}

pub(super) fn numeric_values(rows: &[ProvenancedRow], spec: &AggSpec) -> Vec<f64> {
    rows.iter()
        .filter_map(|row| row.value.as_ref())
        .filter_map(|row| {
            spec.field
                .as_deref()
                .and_then(|field| row.get(field))
                .or_else(|| row.get("value"))
                .and_then(numeric)
        })
        .collect()
}

pub(super) fn fold_numeric(
    rows: &[ProvenancedRow],
    spec: &AggSpec,
    op: fn(f64, f64) -> f64,
) -> Result<f64> {
    numeric_values(rows, spec)
        .into_iter()
        .reduce(op)
        .ok_or_else(|| shape("aggregate has no numeric values"))
}

pub(super) fn json_to_record_value(value: &Value, ty: FieldType) -> Result<RecordValue> {
    match ty {
        FieldType::Bool => value.as_bool().map(RecordValue::Bool),
        FieldType::I64 => value.as_i64().map(RecordValue::I64),
        FieldType::U64 => value.as_u64().map(RecordValue::U64),
        FieldType::F64 => value.as_f64().map(RecordValue::F64),
        FieldType::Text => value.as_str().map(|v| RecordValue::Text(v.to_string())),
        FieldType::Bytes => value
            .as_str()
            .map(|v| RecordValue::Bytes(v.as_bytes().to_vec())),
        FieldType::Timestamp => value.as_i64().map(RecordValue::Timestamp),
    }
    .ok_or_else(|| shape("predicate value does not match index field type"))
}

pub(super) fn json_type(predicate: &FieldPredicate) -> Option<FieldType> {
    match &predicate.value {
        Value::Bool(_) => Some(FieldType::Bool),
        Value::Number(number) if number.as_i64().is_some() => Some(FieldType::I64),
        Value::Number(number) if number.as_u64().is_some() => Some(FieldType::U64),
        Value::Number(_) => Some(FieldType::F64),
        Value::String(_) => Some(FieldType::Text),
        _ => None,
    }
}

pub(super) fn schema_field_type(collection: &Collection, field: &str) -> Option<FieldType> {
    let Some(Schema::SchemaFull(fields)) = &collection.schema else {
        return None;
    };
    fields
        .iter()
        .find(|declared| declared.name == field)
        .map(|declared| declared.ty)
}

pub(super) fn scan_doc_ids<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    collection: &Collection,
    state: &mut ExecState,
) -> Result<Vec<DocId>>
where
    C: Clock,
{
    let prefix = doc_prefix(collection);
    let rows = vault.scan_cf_range_at(snapshot, ColumnFamily::Document, &prefix_range(&prefix))?;
    state.total_scanned += rows.len() as u64;
    let mut ids = BTreeSet::new();
    for (key, _) in rows {
        if key.len() >= 25 {
            ids.insert(DocId::from_slice(&key[9..25])?);
        }
    }
    Ok(ids.into_iter().collect())
}

pub(super) fn doc_value_matches(actual: Option<&Value>, expected: Option<&Value>) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => actual == expected,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

pub(super) fn ledger_ref<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    cx_id: CxId,
) -> Option<LedgerRef> {
    vault.get(cx_id, snapshot).ok().map(|cx| cx.provenance)
}

pub(super) fn scoped_u64(value: &str, default_collection: &str) -> (String, u64) {
    let (collection, raw) = value
        .split_once(':')
        .map_or((default_collection, value), |(left, right)| (left, right));
    let parsed = raw.parse::<u64>().unwrap_or_else(|_| {
        let hash = blake3::hash(raw.as_bytes());
        u64::from_be_bytes(hash.as_bytes()[0..8].try_into().unwrap())
    });
    (collection.to_string(), parsed)
}

pub(super) fn default_collection(name: &str, mode: CollectionMode) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

pub(super) fn plain_row(key: RecordKey, row: Row) -> ProvenancedRow {
    ProvenancedRow {
        key,
        value: Some(row),
        score: None,
        ledger_ref: None,
    }
}

pub(super) fn json_row(field: &str, value: Value) -> Result<Row> {
    Ok(Row::new([(
        field,
        json_to_record_value(&value, json_type_value(&value))?,
    )]))
}

fn json_type_value(value: &Value) -> FieldType {
    match value {
        Value::Bool(_) => FieldType::Bool,
        Value::Number(number) if number.as_i64().is_some() => FieldType::I64,
        Value::Number(number) if number.as_u64().is_some() => FieldType::U64,
        Value::Number(_) => FieldType::F64,
        Value::String(_) => FieldType::Text,
        _ => FieldType::Text,
    }
}

pub(super) fn relational_prefix(collection: &Collection) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(0x01);
    prefix.extend_from_slice(&rel_collection_id(collection).to_be_bytes());
    prefix
}

pub(super) fn doc_prefix(collection: &Collection) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(0x02);
    prefix.extend_from_slice(&doc_collection_id(collection).to_be_bytes());
    prefix
}

pub(super) fn parse_record_pk(key: &[u8]) -> Result<RecordKey> {
    let len = key
        .get(9..11)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]) as usize)
        .ok_or_else(|| shape("relational key missing pk length"))?;
    let pk = key
        .get(11..11 + len)
        .filter(|_| key.len() == 11 + len)
        .ok_or_else(|| shape("relational key has malformed pk bytes"))?;
    RecordKey::from_bytes(pk.to_vec())
}

pub(super) fn cx_from_key(key: &RecordKey) -> Option<CxId> {
    let bytes: [u8; 16] = key.as_bytes().try_into().ok()?;
    Some(CxId::from_bytes(bytes))
}

pub(super) fn require_mode(
    collection: &Collection,
    expected: CollectionMode,
    label: &str,
) -> Result<()> {
    if collection.mode == expected {
        Ok(())
    } else {
        Err(shape(format!(
            "{label} step requires {expected:?} collection, got {:?}",
            collection.mode
        )))
    }
}

pub(super) fn shape(message: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_QUERY_SHAPE, message)
}
