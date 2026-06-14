use std::time::Duration;

use calyx_core::{CalyxError, Result};

use crate::collection::{
    CALYX_INVALID_ARGUMENT, Collection, CollectionMode, FieldType, Schema, collection_has_lens,
};
use crate::layers::relational::{RecordValue, Row};

pub(super) fn expires_at(now: u64, ttl: Option<Duration>) -> Result<u64> {
    match ttl {
        None => Ok(0),
        Some(duration) => {
            let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
            if millis == 0 {
                return Err(invalid_argument("kv ttl must be >= 1ms"));
            }
            Ok(now.saturating_add(millis))
        }
    }
}

pub(super) fn reject_lens_collection(col: &Collection) -> Result<()> {
    if collection_has_lens(col) {
        Err(invalid_argument(
            "cross-model txn T01 supports plain collection rows only",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn require_mode(col: &Collection, expected: CollectionMode, layer: &str) -> Result<()> {
    if col.mode == expected {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "{layer} transaction write expected {expected:?}, got {:?}",
            col.mode
        )))
    }
}

pub(super) fn validate_row(col: &Collection, row: &Row) -> Result<()> {
    for (name, value) in &row.fields {
        if name.is_empty() || name.len() > 128 {
            return Err(schema_violation(
                "row field names must be non-empty and <=128 bytes",
            ));
        }
        if let RecordValue::F64(value) = value
            && !value.is_finite()
        {
            return Err(schema_violation("F64 row value must be finite"));
        }
    }
    if let Some(Schema::SchemaFull(fields)) = &col.schema {
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
            if !fields.iter().any(|field| field.name == *name) {
                return Err(schema_violation(format!("unexpected field `{name}`")));
            }
        }
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

fn schema_violation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_SCHEMA_VIOLATION",
        message: message.into(),
        remediation: "submit a row matching the collection schema",
    }
}

pub(super) fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "fix the transaction input",
    }
}
