use std::collections::BTreeSet;

use calyx_core::Result;
use serde::{Deserialize, Serialize};

use super::{MAX_NAME_BYTES, invalid_argument};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Bool,
    I64,
    F64,
    Text,
    Bytes,
    Timestamp,
    U64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub ty: FieldType,
    pub nullable: bool,
}

impl FieldDef {
    pub fn new(name: impl Into<String>, ty: FieldType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            ty,
            nullable,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        validate_name("field name", &self.name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Schema {
    SchemaFull(Vec<FieldDef>),
    SchemaLess,
}

impl Schema {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::SchemaLess => Ok(()),
            Self::SchemaFull(fields) => {
                if fields.is_empty() {
                    return Err(invalid_argument("SchemaFull requires at least one field"));
                }
                let mut seen = BTreeSet::new();
                for field in fields {
                    field.validate()?;
                    if !seen.insert(field.name.as_str()) {
                        return Err(invalid_argument(format!(
                            "duplicate field name `{}` in schema",
                            field.name
                        )));
                    }
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn validate_name(kind: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(invalid_argument(format!("{kind} must be non-empty")));
    }
    if value.len() > MAX_NAME_BYTES {
        return Err(invalid_argument(format!(
            "{kind} must be <= {MAX_NAME_BYTES} UTF-8 bytes, got {}",
            value.len()
        )));
    }
    Ok(())
}
