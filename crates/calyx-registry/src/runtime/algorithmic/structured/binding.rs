use std::collections::BTreeSet;

use calyx_core::{
    CALYX_STRUCTURED_SCHEMA_INVALID, CALYX_STRUCTURED_SELECTOR_INVALID, CalyxError, Result,
    SlotShape, validate_json_field_pointer,
};
use serde::{Deserialize, Serialize};

use super::codec::{decode_binding, encode_binding, hex, unhex};
use super::error::{CALYX_STRUCTURED_BINDING_INVALID, structured_runtime_error};

pub(super) const RUNTIME_PREFIX: &str = "structured-v1:";
pub(super) const MAX_BINDING_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_SCHEMA_ID_BYTES: usize = 256;
pub(super) const MAX_CATEGORIES: usize = 65_536;
pub(super) const MAX_CATEGORY_BYTES: usize = 1_048_576;
pub(super) const MAX_VECTOR_DIM: u32 = 1_048_576;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredBinding {
    pub(super) schema_id: String,
    pub(super) selector: StructuredSelector,
    pub(super) source_type: StructuredSourceType,
    pub(super) encoder: StructuredEncoder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredSelector {
    Field {
        path: String,
    },
    DerivedNumeric {
        left: String,
        left_type: NumericSourceType,
        right: String,
        right_type: NumericSourceType,
        operation: NumericDerivation,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericSourceType {
    Number,
    Integer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericDerivation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredSourceType {
    Number,
    Integer,
    Boolean,
    String,
    StringSet,
    Date,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredEncoder {
    Numeric {
        scaling: NumericScaling,
    },
    Boolean,
    OneHot {
        categories: Vec<String>,
    },
    StableHash {
        dim: u32,
    },
    SetMultiHot {
        vocabulary: Vec<String>,
    },
    Ordinal {
        categories: Vec<String>,
    },
    Cyclic {
        period: f64,
        #[serde(default)]
        offset: f64,
    },
    DateCyclic {
        component: DateComponent,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NumericScaling {
    Raw,
    MinMax { min: f64, max: f64 },
    ZScore { mean: f64, stddev: f64 },
    Log1pSigned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateComponent {
    Month,
    Weekday,
    DayOfYear,
}

impl StructuredBinding {
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(binding_invalid(format!(
                "structured binding JSON is {} bytes; maximum is {MAX_BINDING_BYTES}",
                bytes.len()
            )));
        }
        let binding: Self = serde_json::from_slice(bytes).map_err(|error| {
            structured_runtime_error(
                CALYX_STRUCTURED_BINDING_INVALID,
                format!("structured binding JSON is invalid: {error}"),
            )
        })?;
        binding.validate()?;
        Ok(binding)
    }

    pub fn from_runtime_kind(kind: &str) -> Result<Self> {
        let encoded = kind.strip_prefix(RUNTIME_PREFIX).ok_or_else(|| {
            structured_runtime_error(
                CALYX_STRUCTURED_BINDING_INVALID,
                format!("structured runtime kind must start with {RUNTIME_PREFIX:?}"),
            )
        })?;
        if encoded.len() > MAX_BINDING_BYTES.saturating_mul(2) {
            return Err(binding_invalid(format!(
                "structured runtime binding hex is {} bytes; maximum is {}",
                encoded.len(),
                MAX_BINDING_BYTES * 2
            )));
        }
        let bytes = unhex(encoded)?;
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(binding_invalid(format!(
                "canonical structured binding is {} bytes; maximum is {MAX_BINDING_BYTES}",
                bytes.len()
            )));
        }
        let binding = decode_binding(&bytes)?;
        binding.validate()?;
        if encode_binding(&binding) != bytes {
            return Err(binding_invalid(
                "structured runtime binding bytes are not canonical",
            ));
        }
        Ok(binding)
    }

    pub fn is_runtime_kind(kind: &str) -> bool {
        kind.starts_with(RUNTIME_PREFIX)
    }

    pub fn runtime_kind(&self) -> Result<String> {
        Ok(format!("{RUNTIME_PREFIX}{}", hex(&self.canonical_bytes()?)))
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = encode_binding(self);
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(binding_invalid(format!(
                "canonical structured binding is {} bytes; maximum is {MAX_BINDING_BYTES}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    pub fn selector(&self) -> &StructuredSelector {
        &self.selector
    }

    pub const fn source_type(&self) -> StructuredSourceType {
        self.source_type
    }

    pub fn encoder(&self) -> &StructuredEncoder {
        &self.encoder
    }

    pub fn shape(&self) -> SlotShape {
        match &self.encoder {
            StructuredEncoder::Numeric { .. }
            | StructuredEncoder::Boolean
            | StructuredEncoder::Ordinal { .. } => SlotShape::Dense(1),
            StructuredEncoder::OneHot { categories } => SlotShape::Dense(categories.len() as u32),
            StructuredEncoder::StableHash { dim } => SlotShape::Sparse(*dim),
            StructuredEncoder::SetMultiHot { vocabulary } => {
                SlotShape::Sparse(vocabulary.len() as u32)
            }
            StructuredEncoder::Cyclic { .. } | StructuredEncoder::DateCyclic { .. } => {
                SlotShape::Dense(2)
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_schema_id(&self.schema_id)?;
        validate_selector(&self.selector, self.source_type)?;
        validate_encoder(&self.encoder, self.source_type)
    }
}

fn validate_schema_id(schema_id: &str) -> Result<()> {
    if schema_id.is_empty()
        || schema_id.trim() != schema_id
        || schema_id.len() > MAX_SCHEMA_ID_BYTES
    {
        return Err(CalyxError {
            code: CALYX_STRUCTURED_SCHEMA_INVALID,
            message: format!(
                "binding schema id must contain 1..={MAX_SCHEMA_ID_BYTES} UTF-8 bytes without surrounding whitespace"
            ),
            remediation: "declare the exact stable record schema and register a new LensId",
        });
    }
    Ok(())
}

fn validate_selector(
    selector: &StructuredSelector,
    source_type: StructuredSourceType,
) -> Result<()> {
    match selector {
        StructuredSelector::Field { path } => validate_json_field_pointer(path),
        StructuredSelector::DerivedNumeric { left, right, .. } => {
            validate_json_field_pointer(left)?;
            validate_json_field_pointer(right)?;
            if left == right {
                return Err(CalyxError {
                    code: CALYX_STRUCTURED_SELECTOR_INVALID,
                    message: format!(
                        "derived selector operands both name {left:?}; declare independent atoms"
                    ),
                    remediation: "declare distinct typed operands and register a new LensId",
                });
            }
            if source_type != StructuredSourceType::Number {
                return Err(binding_invalid(
                    "derived numeric selectors must declare source_type=number",
                ));
            }
            Ok(())
        }
    }
}

fn validate_encoder(encoder: &StructuredEncoder, source_type: StructuredSourceType) -> Result<()> {
    match encoder {
        StructuredEncoder::Numeric { scaling } => {
            require_numeric(source_type, "numeric")?;
            validate_scaling(*scaling)
        }
        StructuredEncoder::Boolean => {
            require_type(source_type, &[StructuredSourceType::Boolean], "boolean")
        }
        StructuredEncoder::OneHot { categories } => {
            require_type(source_type, &[StructuredSourceType::String], "one_hot")?;
            validate_vocabulary(categories, 1, "one_hot")
        }
        StructuredEncoder::StableHash { dim } => {
            require_type(source_type, &[StructuredSourceType::String], "stable_hash")?;
            validate_dim(*dim)
        }
        StructuredEncoder::SetMultiHot { vocabulary } => {
            require_type(
                source_type,
                &[StructuredSourceType::StringSet],
                "set_multi_hot",
            )?;
            validate_vocabulary(vocabulary, 1, "set_multi_hot")
        }
        StructuredEncoder::Ordinal { categories } => {
            require_type(source_type, &[StructuredSourceType::String], "ordinal")?;
            validate_vocabulary(categories, 2, "ordinal")
        }
        StructuredEncoder::Cyclic { period, offset } => {
            require_numeric(source_type, "cyclic")?;
            if !period.is_finite() || *period <= 0.0 || !offset.is_finite() {
                return Err(binding_invalid(
                    "cyclic period must be finite and >0 and offset must be finite",
                ));
            }
            Ok(())
        }
        StructuredEncoder::DateCyclic { .. } => {
            require_type(source_type, &[StructuredSourceType::Date], "date_cyclic")
        }
    }
}

fn validate_scaling(scaling: NumericScaling) -> Result<()> {
    match scaling {
        NumericScaling::Raw | NumericScaling::Log1pSigned => Ok(()),
        NumericScaling::MinMax { min, max } if min.is_finite() && max.is_finite() && max > min => {
            Ok(())
        }
        NumericScaling::ZScore { mean, stddev }
            if mean.is_finite() && stddev.is_finite() && stddev > 0.0 =>
        {
            Ok(())
        }
        other => Err(binding_invalid(format!(
            "numeric scaling parameters are invalid: {other:?}"
        ))),
    }
}

fn validate_vocabulary(values: &[String], minimum: usize, name: &str) -> Result<()> {
    if values.len() < minimum || values.len() > MAX_CATEGORIES {
        return Err(binding_invalid(format!(
            "{name} vocabulary has {} entries; expected {minimum}..={MAX_CATEGORIES}",
            values.len()
        )));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if value.len() > MAX_CATEGORY_BYTES || !unique.insert(value) {
            return Err(binding_invalid(format!(
                "{name} vocabulary contains a duplicate or a value larger than {MAX_CATEGORY_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

fn validate_dim(dim: u32) -> Result<()> {
    if (1..=MAX_VECTOR_DIM).contains(&dim) {
        Ok(())
    } else {
        Err(binding_invalid(format!(
            "stable_hash dim {dim} is outside 1..={MAX_VECTOR_DIM}"
        )))
    }
}

fn require_numeric(source_type: StructuredSourceType, encoder: &str) -> Result<()> {
    require_type(
        source_type,
        &[StructuredSourceType::Number, StructuredSourceType::Integer],
        encoder,
    )
}

fn require_type(
    actual: StructuredSourceType,
    expected: &[StructuredSourceType],
    encoder: &str,
) -> Result<()> {
    if expected.contains(&actual) {
        Ok(())
    } else {
        Err(binding_invalid(format!(
            "{encoder} encoder is incompatible with declared source type {actual:?}"
        )))
    }
}

fn binding_invalid(message: impl Into<String>) -> CalyxError {
    structured_runtime_error(CALYX_STRUCTURED_BINDING_INVALID, message)
}
