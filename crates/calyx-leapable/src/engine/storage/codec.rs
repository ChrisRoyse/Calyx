use std::time::Duration;

use calyx_aster::collection::{
    CALYX_COLLECTION_NOT_FOUND, Collection, CollectionMode, DedupPolicy, FieldDef, FieldType,
    RetentionPolicy, Schema, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId,
    TxnPolicy, create_collection, get_collection,
};
use calyx_aster::index::{IndexId, IndexKind, IndexSpec};
use calyx_aster::layers::relational::{CALYX_SCHEMA_VIOLATION, RecordKey, RecordValue, Row};
use calyx_core::CalyxError;
use calyx_ledger::{ActorId, EntryKind};

use super::params::{
    CollectionSpecParam, FieldTypeParam, IndexKindParam, IndexParam, RetentionParam, TxnOpParam,
};
use super::{
    CALYX_LEAPABLE_COLLECTION_MISMATCH, CALYX_LEAPABLE_INDEX_NOT_FOUND,
    CALYX_LEAPABLE_REL_CONFLICT, CALYX_LEAPABLE_STORAGE_INPUT_INVALID, EngineResult, VaultHandle,
};

pub(super) use support::{
    served_write_collection, unserved_capability_error, unserved_index_kind, warn_stranded_indexes,
};
pub(super) use values::*;

pub(super) fn stage_txn_op(
    handle: &VaultHandle,
    txn: &mut calyx_aster::txn::CrossModelTxn<'_>,
    op: TxnOpParam,
) -> EngineResult<()> {
    match op {
        TxnOpParam::RelInsert {
            collection_name,
            collection,
            pk,
            row,
        } => {
            let pk = record_key_from_param(pk)?;
            let row = row_from_param(row)?;
            let col = ensure_record_collection_for_row(handle, &collection_name, collection, &row)?;
            if txn.get_record(&handle.vault, &col, &pk)?.is_some() {
                return Err(storage_error(
                    CALYX_LEAPABLE_REL_CONFLICT,
                    "txn rel.insert refuses to overwrite an existing row",
                    "use a non-conflicting primary key",
                )
                .into());
            }
            let write_col = served_write_collection(&col);
            txn.put_record(&handle.vault, &write_col, &pk, &row)?;
        }
        TxnOpParam::KvSet {
            collection_name,
            collection,
            ns,
            key,
            value,
            ttl_ms,
        } => {
            let key = bytes_from_param(key)?;
            let value = bytes_from_param(value)?;
            let col = ensure_collection(handle, &collection_name, CollectionMode::KV, collection)?;
            let write_col = served_write_collection(&col);
            txn.kv_set(
                &handle.vault,
                &write_col,
                ns,
                &key,
                &value,
                ttl_ms.map(Duration::from_millis),
            )?;
        }
        TxnOpParam::TsWrite {
            collection_name,
            collection,
            series,
            point_ts,
            value,
        } => {
            validate_ts_value(value)?;
            let col = ensure_collection(
                handle,
                &collection_name,
                CollectionMode::TimeSeries,
                collection,
            )?;
            let write_col = served_write_collection(&col);
            txn.ts_write(&handle.vault, &write_col, series, point_ts, value)?;
        }
    }
    Ok(())
}

pub(super) fn ensure_collection(
    handle: &VaultHandle,
    name: &str,
    mode: CollectionMode,
    spec: Option<CollectionSpecParam>,
) -> EngineResult<Collection> {
    if let Some(existing) = handle.cached_collection(name) {
        require_mode(&existing, mode)?;
        if let Some(spec) = spec {
            assert_collection_matches(&existing, &build_collection(name, mode, spec)?)?;
        }
        return Ok((*existing).clone());
    }
    match get_collection(&handle.vault, name) {
        Ok(existing) => {
            require_mode(&existing, mode)?;
            if let Some(spec) = spec {
                assert_collection_matches(&existing, &build_collection(name, mode, spec)?)?;
            }
            Ok((*handle.cache_collection(existing)).clone())
        }
        Err(error) if error.code == CALYX_COLLECTION_NOT_FOUND && spec.is_some() => {
            let collection = build_collection(name, mode, spec.expect("checked"))?;
            create_collection(&handle.vault, collection.clone())?;
            Ok((*handle.cache_collection(collection)).clone())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn ensure_record_collection_for_row(
    handle: &VaultHandle,
    name: &str,
    spec: Option<CollectionSpecParam>,
    row: &Row,
) -> EngineResult<Collection> {
    if let Some(existing) = handle.cached_collection(name) {
        require_mode(&existing, CollectionMode::Records)?;
        if let Some(spec) = spec {
            assert_collection_matches(
                &existing,
                &build_collection(name, CollectionMode::Records, spec)?,
            )?;
        }
        validate_row_shape(&existing, row)?;
        return Ok((*existing).clone());
    }
    match get_collection(&handle.vault, name) {
        Ok(existing) => {
            require_mode(&existing, CollectionMode::Records)?;
            if let Some(spec) = spec {
                assert_collection_matches(
                    &existing,
                    &build_collection(name, CollectionMode::Records, spec)?,
                )?;
            }
            validate_row_shape(&existing, row)?;
            Ok((*handle.cache_collection(existing)).clone())
        }
        Err(error) if error.code == CALYX_COLLECTION_NOT_FOUND && spec.is_some() => {
            let collection =
                build_collection(name, CollectionMode::Records, spec.expect("checked"))?;
            validate_row_shape(&collection, row)?;
            create_collection(&handle.vault, collection.clone())?;
            Ok((*handle.cache_collection(collection)).clone())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn require_collection(
    handle: &VaultHandle,
    name: &str,
    mode: CollectionMode,
) -> EngineResult<Collection> {
    if let Some(collection) = handle.cached_collection(name) {
        require_mode(&collection, mode)?;
        return Ok((*collection).clone());
    }
    let collection = get_collection(&handle.vault, name)?;
    require_mode(&collection, mode)?;
    Ok((*handle.cache_collection(collection)).clone())
}

pub(super) fn validate_ts_value(value: f64) -> EngineResult<()> {
    if value.is_finite() {
        return Ok(());
    }
    Err(storage_error(
        CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
        "time-series value must be finite",
        "send a finite f64 value",
    )
    .into())
}

fn require_mode(collection: &Collection, expected: CollectionMode) -> EngineResult<()> {
    if collection.mode == expected {
        return Ok(());
    }
    Err(storage_error(
        CALYX_LEAPABLE_COLLECTION_MISMATCH,
        format!(
            "collection {:?} has mode {:?}, expected {:?}",
            collection.name, collection.mode, expected
        ),
        "use a collection created for this storage layer",
    )
    .into())
}

fn assert_collection_matches(existing: &Collection, requested: &Collection) -> EngineResult<()> {
    if existing.mode == requested.mode
        && existing.schema == requested.schema
        && existing.indexes == requested.indexes
        && existing.retention == requested.retention
    {
        return Ok(());
    }
    Err(storage_error(
        CALYX_LEAPABLE_COLLECTION_MISMATCH,
        format!(
            "collection {:?} already exists with a different descriptor",
            existing.name
        ),
        "reuse the existing descriptor or choose a different collection name",
    )
    .into())
}

fn build_collection(
    name: &str,
    mode: CollectionMode,
    spec: CollectionSpecParam,
) -> EngineResult<Collection> {
    Ok(Collection {
        name: name.to_string(),
        mode,
        schema: match spec.schema {
            Some(fields) => Some(Schema::SchemaFull(
                fields
                    .into_iter()
                    .map(|field| FieldDef::new(field.name, field_type(field.ty), field.nullable))
                    .collect(),
            )),
            None => Some(Schema::SchemaLess),
        },
        panel: None,
        indexes: spec
            .indexes
            .into_iter()
            .map(|index| secondary_index_spec(mode, index))
            .collect::<EngineResult<_>>()?,
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: match spec.retention {
            None | Some(RetentionParam::Forever) => RetentionPolicy::Forever,
            Some(RetentionParam::RollupOnly) => RetentionPolicy::RollupOnly,
            Some(RetentionParam::DropAfterMs(ms)) => {
                RetentionPolicy::DropAfter(Duration::from_millis(ms))
            }
        },
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    })
}

fn field_type(value: FieldTypeParam) -> FieldType {
    match value {
        FieldTypeParam::Bool => FieldType::Bool,
        FieldTypeParam::I64 => FieldType::I64,
        FieldTypeParam::U64 => FieldType::U64,
        FieldTypeParam::F64 => FieldType::F64,
        FieldTypeParam::Text => FieldType::Text,
        FieldTypeParam::Bytes => FieldType::Bytes,
        FieldTypeParam::Timestamp => FieldType::Timestamp,
    }
}

fn secondary_index_spec(
    mode: CollectionMode,
    index: IndexParam,
) -> EngineResult<SecondaryIndexSpec> {
    if mode != CollectionMode::Records {
        return Err(unserved_capability_error(
            format!(
                "Leapable only serves secondary indexes through rel.query on Records collections; index {:?} was declared for {mode:?}",
                index.name
            ),
            "omit indexes on non-Records collections until Leapable exposes a query surface for that model",
        )
        .into());
    }
    let kind = index_kind(index.kind, &index.name)?;
    if index.fields.len() != 1 {
        return Err(unserved_capability_error(
            format!(
                "Leapable only serves single-field btree indexes; index {:?} declared {} fields",
                index.name,
                index.fields.len()
            ),
            "declare exactly one indexed field or omit the unserved index",
        )
        .into());
    }
    Ok(SecondaryIndexSpec {
        name: index.name,
        kind,
        fields: index.fields,
    })
}

fn index_kind(value: IndexKindParam, name: &str) -> EngineResult<SecondaryIndexKind> {
    match value {
        IndexKindParam::Btree => Ok(SecondaryIndexKind::Btree),
        IndexKindParam::Inverted => Err(unserved_index_kind("inverted", name).into()),
        IndexKindParam::Ann => Err(unserved_index_kind("ann", name).into()),
        IndexKindParam::Kernel => Err(unserved_index_kind("kernel", name).into()),
    }
}

pub(super) fn runtime_btree_spec(
    col: &Collection,
    index_name: &str,
    sample_value: Option<&RecordValue>,
) -> EngineResult<IndexSpec> {
    let (ordinal, declared) = col
        .indexes
        .iter()
        .enumerate()
        .find(|(_, index)| index.name == index_name)
        .ok_or_else(|| {
            storage_error(
                CALYX_LEAPABLE_INDEX_NOT_FOUND,
                format!("collection {:?} has no index {:?}", col.name, index_name),
                "query an index declared on the collection descriptor",
            )
        })?;
    if declared.kind != SecondaryIndexKind::Btree || declared.fields.len() != 1 {
        return Err(unserved_capability_error(
            "Leapable rel.query only serves single-field btree indexes",
            "query a single-field btree index or omit unserved ANN/inverted/kernel declarations",
        )
        .into());
    }
    let field = &declared.fields[0];
    let ty = schema_field_type(col, field)
        .or_else(|| sample_value.and_then(record_field_type))
        .ok_or_else(|| {
            storage_error(
                CALYX_LEAPABLE_STORAGE_INPUT_INVALID,
                "schema-less rel.query needs at least one typed bound",
                "provide gte or lte so the btree key type is known",
            )
        })?;
    Ok(IndexSpec::new(
        IndexId::new((ordinal + 1) as u32),
        &declared.name,
        IndexKind::Btree,
        field,
        ty,
    ))
}

fn schema_field_type(col: &Collection, field: &str) -> Option<FieldType> {
    let Some(Schema::SchemaFull(fields)) = &col.schema else {
        return None;
    };
    fields
        .iter()
        .find(|declared| declared.name == field)
        .map(|declared| declared.ty)
}

fn validate_row_shape(col: &Collection, row: &Row) -> EngineResult<()> {
    for (name, value) in &row.fields {
        if name.is_empty() || name.len() > 128 {
            return Err(
                schema_violation("row field names must be non-empty and <=128 bytes").into(),
            );
        }
        if let RecordValue::F64(value) = value
            && !value.is_finite()
        {
            return Err(schema_violation("F64 row value must be finite").into());
        }
    }
    if let Some(Schema::SchemaFull(fields)) = &col.schema {
        for field in fields {
            match row.fields.get(&field.name) {
                Some(RecordValue::Null) if field.nullable => {}
                Some(value) if record_field_type(value) == Some(field.ty) => {}
                Some(RecordValue::Null) => {
                    return Err(schema_violation(format!(
                        "field `{}` is null but not nullable",
                        field.name
                    ))
                    .into());
                }
                Some(value) => {
                    return Err(schema_violation(format!(
                        "field `{}` expected {:?}, got {:?}",
                        field.name, field.ty, value
                    ))
                    .into());
                }
                None if field.nullable => {}
                None => {
                    return Err(schema_violation(format!("missing field `{}`", field.name)).into());
                }
            }
        }
        for name in row.fields.keys() {
            if !fields.iter().any(|field| field.name == *name) {
                return Err(schema_violation(format!("unexpected field `{name}`")).into());
            }
        }
    }
    Ok(())
}

fn schema_violation(message: impl Into<String>) -> CalyxError {
    storage_error(
        CALYX_SCHEMA_VIOLATION,
        message,
        "submit a row matching the collection SchemaFull definition",
    )
}

pub(super) fn storage_error(
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

pub(super) fn write_rel_delete(
    handle: &VaultHandle,
    col: &Collection,
    pk: &RecordKey,
    old_row: &Row,
) -> EngineResult<u64> {
    let key = calyx_aster::layers::relational::record_key(col, pk)?;
    let rows = rel_delete_rows(handle, col, pk, old_row)?;
    Ok(handle.vault.write_cf_batch_with_ledger_entry(
        rows,
        EntryKind::Ingest,
        ledger_subject(&key),
        rel_delete_payload(col, pk, old_row)?,
        ActorId::Service("calyx-leapable-rel-delete".to_string()),
    )?)
}

mod support;
mod values;
