use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;

use super::codec::{decode_record, encode_record};
use super::error::{
    CALYX_STRUCTURED_NON_FINITE, CALYX_STRUCTURED_RECORD_INVALID,
    CALYX_STRUCTURED_RESERVED_METADATA, CALYX_STRUCTURED_SCHEMA_INVALID,
    CALYX_STRUCTURED_SCHEMA_MISMATCH, CALYX_STRUCTURED_TYPE_MISMATCH, structured_error,
};
use super::pointer::{escape_json_pointer_token, validate_json_field_pointer};

pub const STRUCTURED_METADATA_PREFIX: &str = "calyx.structured.";
pub const METADATA_STRUCTURED_SCHEMA: &str = "calyx.structured.schema.v1";
pub const METADATA_STRUCTURED_RECORD: &str = "calyx.structured.record.v1";

const MAX_SCHEMA_ID_BYTES: usize = 256;
pub(super) const MAX_FIELDS: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_VALUE_BYTES: usize = 1_048_576;
pub(super) const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_SET_VALUES: usize = 65_536;
const MAX_EXACT_F64_INTEGER: i64 = 1_i64 << 53;

/// One canonical typed explicit value. Floating-point values are stored by
/// exact IEEE-754 bits so record equality and identity remain deterministic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredValue {
    Number { bits: u64 },
    Integer(i64),
    Boolean(bool),
    String(String),
    StringSet(Vec<String>),
}

impl StructuredValue {
    pub fn number(value: f64) -> Result<Self> {
        if !value.is_finite() {
            return Err(structured_error(
                CALYX_STRUCTURED_NON_FINITE,
                "structured numeric value is NaN or infinite",
            ));
        }
        let canonical = if value == 0.0 { 0.0 } else { value };
        Ok(Self::Number {
            bits: canonical.to_bits(),
        })
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number { bits } => Some(f64::from_bits(*bits)),
            Self::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }
}

/// A versioned schema plus its independently addressable typed leaves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredRecord {
    pub(super) schema_id: String,
    pub(super) fields: BTreeMap<String, StructuredValue>,
}

impl StructuredRecord {
    /// Converts a JSON object into canonical RFC-6901-addressed typed leaves.
    pub fn from_json_value(schema_id: impl Into<String>, value: &Value) -> Result<Self> {
        let schema_id = schema_id.into();
        validate_schema_id(&schema_id)?;
        let Value::Object(object) = value else {
            return Err(structured_error(
                CALYX_STRUCTURED_TYPE_MISMATCH,
                "structured record root must be a JSON object",
            ));
        };
        let mut fields = BTreeMap::new();
        for (key, value) in object {
            let path = format!("/{}", escape_json_pointer_token(key));
            flatten(value, &path, 1, &mut fields)?;
        }
        let record = Self { schema_id, fields };
        record.validate()?;
        Ok(record)
    }

    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    pub fn fields(&self) -> &BTreeMap<String, StructuredValue> {
        &self.fields
    }

    pub fn get(&self, pointer: &str) -> Option<&StructuredValue> {
        self.fields.get(pointer)
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema_id(&self.schema_id)?;
        if self.fields.is_empty() || self.fields.len() > MAX_FIELDS {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_INVALID,
                format!(
                    "structured record has {} fields; expected 1..={MAX_FIELDS}",
                    self.fields.len()
                ),
            ));
        }
        for (path, value) in &self.fields {
            validate_json_field_pointer(path)?;
            validate_value(path, value)?;
        }
        let encoded = encode_record(self);
        if encoded.len() > MAX_RECORD_BYTES {
            return Err(structured_error(
                CALYX_STRUCTURED_RECORD_INVALID,
                format!(
                    "canonical structured record is {} bytes; maximum is {MAX_RECORD_BYTES}",
                    encoded.len()
                ),
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(encode_record(self))
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(structured_error(
                CALYX_STRUCTURED_RECORD_INVALID,
                format!(
                    "canonical structured record is {} bytes; maximum is {MAX_RECORD_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let record = decode_record(bytes)?;
        record.validate()?;
        if encode_record(&record) != bytes {
            return Err(structured_error(
                CALYX_STRUCTURED_RECORD_INVALID,
                "structured record bytes are not in canonical form",
            ));
        }
        Ok(record)
    }

    /// Writes source scalars and the exact canonical record into Base columns.
    pub fn persist_into(
        &self,
        scalars: &mut BTreeMap<String, f64>,
        metadata: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        let bytes = self.canonical_bytes()?;
        if metadata
            .keys()
            .any(|key| key.starts_with(STRUCTURED_METADATA_PREFIX))
        {
            return Err(structured_error(
                CALYX_STRUCTURED_RESERVED_METADATA,
                "constellation metadata already contains calyx.structured.* keys",
            ));
        }
        let source_scalars = self.source_scalars()?;
        if let Some(key) = source_scalars.keys().find(|key| scalars.contains_key(*key)) {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_MISMATCH,
                format!("structured source scalar {key:?} collides with an existing scalar"),
            ));
        }
        scalars.extend(source_scalars);
        metadata.insert(
            METADATA_STRUCTURED_SCHEMA.to_string(),
            self.schema_id.clone(),
        );
        metadata.insert(METADATA_STRUCTURED_RECORD.to_string(), hex(&bytes));
        Ok(())
    }

    /// Reconstructs and verifies structured state from authoritative Base bytes.
    pub fn from_persisted(
        scalars: &BTreeMap<String, f64>,
        metadata: &BTreeMap<String, String>,
    ) -> Result<Option<Self>> {
        if let Some(key) = metadata.keys().find(|key| {
            key.starts_with(STRUCTURED_METADATA_PREFIX)
                && key.as_str() != METADATA_STRUCTURED_SCHEMA
                && key.as_str() != METADATA_STRUCTURED_RECORD
        }) {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_MISMATCH,
                format!("Base contains unknown reserved structured metadata {key:?}"),
            ));
        }
        let schema = metadata.get(METADATA_STRUCTURED_SCHEMA);
        let encoded = metadata.get(METADATA_STRUCTURED_RECORD);
        let (Some(schema), Some(encoded)) = (schema, encoded) else {
            if schema.is_none() && encoded.is_none() {
                return Ok(None);
            }
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_MISMATCH,
                "Base contains only one of the required structured schema/record fields",
            ));
        };
        let bytes = unhex(encoded)?;
        let record = Self::from_canonical_bytes(&bytes)?;
        if record.schema_id != *schema {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_MISMATCH,
                format!(
                    "Base schema metadata {schema:?} does not match record {:?}",
                    record.schema_id
                ),
            ));
        }
        for (path, expected) in record.source_scalars()? {
            let Some(actual) = scalars.get(&path) else {
                return Err(structured_error(
                    CALYX_STRUCTURED_SCHEMA_MISMATCH,
                    format!("Base is missing structured source scalar {path:?}"),
                ));
            };
            if actual.to_bits() != expected.to_bits() {
                return Err(structured_error(
                    CALYX_STRUCTURED_SCHEMA_MISMATCH,
                    format!(
                        "Base structured scalar {path:?} bits {:016x} != expected {:016x}",
                        actual.to_bits(),
                        expected.to_bits()
                    ),
                ));
            }
        }
        Ok(Some(record))
    }

    fn source_scalars(&self) -> Result<BTreeMap<String, f64>> {
        let mut scalars = BTreeMap::new();
        for (path, value) in &self.fields {
            let scalar = match value {
                StructuredValue::Number { bits } => Some(f64::from_bits(*bits)),
                StructuredValue::Integer(value) => Some(exact_integer(*value)?),
                StructuredValue::Boolean(value) => Some(if *value { 1.0 } else { 0.0 }),
                StructuredValue::String(_) | StructuredValue::StringSet(_) => None,
            };
            if let Some(value) = scalar {
                scalars.insert(path.clone(), value);
            }
        }
        Ok(scalars)
    }
}

fn flatten(
    value: &Value,
    path: &str,
    depth: usize,
    fields: &mut BTreeMap<String, StructuredValue>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(structured_error(
            CALYX_STRUCTURED_SCHEMA_INVALID,
            format!("structured record exceeds maximum nesting depth {MAX_DEPTH}"),
        ));
    }
    if fields.len() >= MAX_FIELDS {
        return Err(structured_error(
            CALYX_STRUCTURED_SCHEMA_INVALID,
            format!("structured record exceeds maximum field count {MAX_FIELDS}"),
        ));
    }
    match value {
        Value::Object(object) if object.is_empty() => Err(structured_error(
            CALYX_STRUCTURED_TYPE_MISMATCH,
            format!("structured field {path:?} is an empty object with no measurable atom"),
        )),
        Value::Object(object) => {
            for (key, value) in object {
                let child = format!("{path}/{}", escape_json_pointer_token(key));
                flatten(value, &child, depth + 1, fields)?;
            }
            Ok(())
        }
        Value::Array(values) => {
            if values.len() > MAX_SET_VALUES {
                return Err(structured_error(
                    CALYX_STRUCTURED_RECORD_INVALID,
                    format!(
                        "structured set {path:?} has {} values; maximum is {MAX_SET_VALUES}",
                        values.len()
                    ),
                ));
            }
            let mut set = values
                .iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value.clone()),
                    _ => Err(structured_error(
                        CALYX_STRUCTURED_TYPE_MISMATCH,
                        format!("structured set {path:?} must contain only strings"),
                    )),
                })
                .collect::<Result<Vec<_>>>()?;
            set.sort();
            set.dedup();
            fields.insert(path.to_string(), StructuredValue::StringSet(set));
            Ok(())
        }
        Value::Null => Err(structured_error(
            CALYX_STRUCTURED_TYPE_MISMATCH,
            format!(
                "structured field {path:?} is null; absence must be explicit at the record boundary"
            ),
        )),
        Value::Bool(value) => {
            fields.insert(path.to_string(), StructuredValue::Boolean(*value));
            Ok(())
        }
        Value::Number(value) => {
            let parsed = if let Some(value) = value.as_i64() {
                exact_integer(value)?;
                StructuredValue::Integer(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    structured_error(
                        CALYX_STRUCTURED_NON_FINITE,
                        format!("structured integer {path:?} exceeds signed 64-bit range"),
                    )
                })?;
                exact_integer(value)?;
                StructuredValue::Integer(value)
            } else {
                StructuredValue::number(value.as_f64().ok_or_else(|| {
                    structured_error(
                        CALYX_STRUCTURED_NON_FINITE,
                        format!("structured number {path:?} cannot be represented as finite f64"),
                    )
                })?)?
            };
            fields.insert(path.to_string(), parsed);
            Ok(())
        }
        Value::String(value) => {
            fields.insert(path.to_string(), StructuredValue::String(value.clone()));
            Ok(())
        }
    }
}

fn validate_schema_id(schema_id: &str) -> Result<()> {
    if schema_id.is_empty()
        || schema_id.trim() != schema_id
        || schema_id.len() > MAX_SCHEMA_ID_BYTES
    {
        return Err(structured_error(
            CALYX_STRUCTURED_SCHEMA_INVALID,
            format!(
                "schema id must contain 1..={MAX_SCHEMA_ID_BYTES} UTF-8 bytes without surrounding whitespace"
            ),
        ));
    }
    Ok(())
}

fn validate_value(path: &str, value: &StructuredValue) -> Result<()> {
    match value {
        StructuredValue::Number { bits } => {
            let value = f64::from_bits(*bits);
            if !value.is_finite() || (value == 0.0 && *bits != 0) {
                return Err(structured_error(
                    CALYX_STRUCTURED_NON_FINITE,
                    format!("structured number {path:?} is non-finite or non-canonical zero"),
                ));
            }
        }
        StructuredValue::Integer(value) => {
            exact_integer(*value)?;
        }
        StructuredValue::String(value) => validate_string(path, value)?,
        StructuredValue::StringSet(values) => {
            if values.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(structured_error(
                    CALYX_STRUCTURED_RECORD_INVALID,
                    format!("structured set {path:?} must be strictly sorted and unique"),
                ));
            }
            for value in values {
                validate_string(path, value)?;
            }
        }
        StructuredValue::Boolean(_) => {}
    }
    Ok(())
}

fn validate_string(path: &str, value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(structured_error(
            CALYX_STRUCTURED_RECORD_INVALID,
            format!(
                "structured string {path:?} is {} bytes; maximum is {MAX_VALUE_BYTES}",
                value.len()
            ),
        ));
    }
    Ok(())
}

fn exact_integer(value: i64) -> Result<f64> {
    if !(-MAX_EXACT_F64_INTEGER..=MAX_EXACT_F64_INTEGER).contains(&value) {
        return Err(structured_error(
            CALYX_STRUCTURED_NON_FINITE,
            format!(
                "structured integer {value} exceeds exact f64 scalar range ±{MAX_EXACT_F64_INTEGER}"
            ),
        ));
    }
    Ok(value as f64)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.len() > MAX_RECORD_BYTES.saturating_mul(2) {
        return Err(structured_error(
            CALYX_STRUCTURED_RECORD_INVALID,
            format!(
                "structured record hex is {} bytes; maximum is {}",
                value.len(),
                MAX_RECORD_BYTES * 2
            ),
        ));
    }
    if !value.len().is_multiple_of(2) {
        return Err(structured_error(
            CALYX_STRUCTURED_RECORD_INVALID,
            "structured record hex has odd length",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let high = hex_value(pair[0]).ok_or_else(|| {
                structured_error(
                    CALYX_STRUCTURED_RECORD_INVALID,
                    format!("structured record hex is invalid at byte {}", index * 2),
                )
            })?;
            let low = hex_value(pair[1]).ok_or_else(|| {
                structured_error(
                    CALYX_STRUCTURED_RECORD_INVALID,
                    format!("structured record hex is invalid at byte {}", index * 2 + 1),
                )
            })?;
            Ok((high << 4) | low)
        })
        .collect()
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}
