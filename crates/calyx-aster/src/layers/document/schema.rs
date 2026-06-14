use std::collections::BTreeSet;

use calyx_core::Result;
use serde_json::Value;

use crate::collection::{Collection, FieldType, Schema};

use super::codec::validate_segment;
use super::errors::schema_violation;

pub(super) fn validate_document(col: &Collection, doc: &Value) -> Result<()> {
    validate_path_names(doc)?;
    if let Some(Schema::SchemaFull(fields)) = &col.schema {
        let Value::Object(map) = doc else {
            return Err(schema_violation(
                "SchemaFull document must be a JSON object",
            ));
        };
        let declared = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<BTreeSet<_>>();
        for field in fields {
            match map.get(&field.name) {
                Some(Value::Null) if field.nullable => {}
                Some(value) if value_matches_type(value, field.ty) => {}
                Some(Value::Null) => {
                    return Err(schema_violation(format!(
                        "field `{}` is null but not nullable",
                        field.name
                    )));
                }
                Some(value) => {
                    return Err(schema_violation(format!(
                        "field `{}` expected {:?}, got {}",
                        field.name, field.ty, value
                    )));
                }
                None if field.nullable => {}
                None => return Err(schema_violation(format!("missing field `{}`", field.name))),
            }
        }
        for name in map.keys() {
            if !declared.contains(name.as_str()) {
                return Err(schema_violation(format!("unexpected field `{name}`")));
            }
        }
    }
    Ok(())
}

fn validate_path_names(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (name, child) in map {
                validate_segment(name)?;
                validate_path_names(child)?;
            }
            Ok(())
        }
        Value::Array(_) => Ok(()),
        _ => Ok(()),
    }
}

fn value_matches_type(value: &Value, ty: FieldType) -> bool {
    match ty {
        FieldType::Bool => value.is_boolean(),
        FieldType::I64 | FieldType::Timestamp => value.as_i64().is_some(),
        FieldType::U64 => value.as_u64().is_some(),
        FieldType::F64 => value.as_f64().is_some(),
        FieldType::Text | FieldType::Bytes => value.as_str().is_some(),
    }
}
