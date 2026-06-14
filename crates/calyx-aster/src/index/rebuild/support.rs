use calyx_core::{CalyxError, Result};

use crate::cf::ColumnFamily;
use crate::collection::{CALYX_INVALID_ARGUMENT, Collection, CollectionMode};
use crate::index::{BtreeIndex, IndexKind, IndexSpec, InvertedIndex};
use crate::layers::relational::{CALYX_SCHEMA_VIOLATION, collection_id};
use crate::layers::{RecordValue, Row};

use super::types::MAX_BATCH_SIZE;

pub(super) fn btree_index(col: &Collection, spec: &IndexSpec) -> Result<BtreeIndex> {
    spec.validate()?;
    if spec.kind != IndexKind::Btree {
        return Err(invalid_argument("btree rebuild requires kind Btree"));
    }
    Ok(BtreeIndex::new(collection_id(col), spec.clone()))
}

pub(super) fn inverted_index(col: &Collection, spec: &IndexSpec) -> Result<InvertedIndex> {
    spec.validate()?;
    if spec.kind != IndexKind::Inverted {
        return Err(invalid_argument("inverted rebuild requires kind Inverted"));
    }
    Ok(InvertedIndex::new(collection_id(col), spec.clone()))
}

pub(super) fn is_active_spec(col: &Collection, spec: &IndexSpec) -> Result<bool> {
    spec.validate()?;
    if col.indexes.is_empty() {
        return Ok(false);
    }
    if !matches!(spec.kind, IndexKind::Btree | IndexKind::Inverted) {
        return Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        ));
    }
    let found = col.indexes.iter().any(|declared| {
        declared.name == spec.name
            && declared.kind == spec.kind
            && declared.fields.len() == 1
            && declared.fields[0] == spec.on_field
    });
    if found {
        Ok(true)
    } else {
        Err(invalid_argument(format!(
            "index spec `{}` is not declared on collection `{}`",
            spec.name, col.name
        )))
    }
}

pub(super) fn index_cf(spec: &IndexSpec) -> Result<ColumnFamily> {
    match spec.kind {
        IndexKind::Btree => Ok(ColumnFamily::IndexBtree),
        IndexKind::Inverted => Ok(ColumnFamily::IndexInverted),
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
}

pub(super) fn index_prefix(col: &Collection, spec: &IndexSpec) -> Result<Vec<u8>> {
    match spec.kind {
        IndexKind::Btree => Ok(btree_index(col, spec)?.index_key_prefix()),
        IndexKind::Inverted => Ok(inverted_index(col, spec)?.index_key_prefix()),
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
}

pub(super) fn require_records_collection(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::Records {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "index rebuild currently scans Records collections, got {:?}",
            col.mode
        )))
    }
}

pub(super) fn effective_batch_size(batch_size: usize) -> Result<usize> {
    let effective = if batch_size == 0 {
        MAX_BATCH_SIZE
    } else {
        batch_size
    };
    if effective > MAX_BATCH_SIZE {
        return Err(invalid_argument(format!(
            "index rebuild batch_size {effective} exceeds max {MAX_BATCH_SIZE}"
        )));
    }
    Ok(effective)
}

pub(super) fn is_inverted_stats_key(prefix: &[u8], key: &[u8]) -> bool {
    key.starts_with(prefix) && key.len() == prefix.len() + 8
}

pub(super) fn indexed_value<'a>(row: &'a Row, field: &str) -> Result<&'a RecordValue> {
    row.get(field)
        .ok_or_else(|| index_schema_violation(format!("missing indexed field `{field}`")))
}

pub(super) fn index_schema_violation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_SCHEMA_VIOLATION,
        message: message.into(),
        remediation: "submit a row containing every indexed field",
    }
}

pub(super) fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "correct the index rebuild input",
    }
}

pub(super) fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
