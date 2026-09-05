use calyx_core::{
    CALYX_STRUCTURED_FIELD_MISSING, CALYX_STRUCTURED_NON_FINITE, CALYX_STRUCTURED_SCHEMA_MISMATCH,
    CALYX_STRUCTURED_TYPE_MISMATCH, CalyxError, Input, Result, SlotVector, SparseEntry,
    StructuredValue,
};

use super::binding::{
    DateComponent, NumericDerivation, NumericScaling, NumericSourceType, StructuredBinding,
    StructuredEncoder, StructuredSelector, StructuredSourceType,
};
use super::date::{GregorianDate, parse_gregorian_date};
use super::error::{
    CALYX_STRUCTURED_CATEGORY_UNKNOWN, CALYX_STRUCTURED_DERIVATION_INVALID,
    structured_runtime_error,
};

impl StructuredBinding {
    pub fn measure(&self, input: &Input) -> Result<SlotVector> {
        self.validate()?;
        let record = input.structured().ok_or_else(|| {
            core_error(
                CALYX_STRUCTURED_FIELD_MISSING,
                format!(
                    "structured binding for schema {:?} received no typed record",
                    self.schema_id
                ),
                "attach the required canonical structured record and retry",
            )
        })?;
        record.validate()?;
        if record.schema_id() != self.schema_id {
            return Err(core_error(
                CALYX_STRUCTURED_SCHEMA_MISMATCH,
                format!(
                    "structured binding schema {:?} received record schema {:?}",
                    self.schema_id,
                    record.schema_id()
                ),
                "use the exact frozen schema for ingest, query, replay, and backfill",
            ));
        }
        let selected = select(self, record)?;
        encode_selected(self, selected)
    }
}

enum Selected<'a> {
    Number(f64),
    Boolean(bool),
    String(&'a str),
    StringSet(&'a [String]),
    Date(GregorianDate),
}

fn select<'a>(
    binding: &StructuredBinding,
    record: &'a calyx_core::StructuredRecord,
) -> Result<Selected<'a>> {
    match &binding.selector {
        StructuredSelector::Field { path } => {
            let value = record
                .get(path)
                .ok_or_else(|| missing_field(path, record.schema_id()))?;
            select_field(path, value, binding.source_type)
        }
        StructuredSelector::DerivedNumeric {
            left,
            left_type,
            right,
            right_type,
            operation,
        } => {
            let left_value = numeric_operand(record, left, *left_type)?;
            let right_value = numeric_operand(record, right, *right_type)?;
            let value = derive(left_value, right_value, *operation, left, right)?;
            Ok(Selected::Number(value))
        }
    }
}

fn select_field<'a>(
    path: &str,
    value: &'a StructuredValue,
    source_type: StructuredSourceType,
) -> Result<Selected<'a>> {
    let selected = match (source_type, value) {
        (StructuredSourceType::Number, StructuredValue::Number { bits }) => {
            Selected::Number(f64::from_bits(*bits))
        }
        (StructuredSourceType::Integer, StructuredValue::Integer(value)) => {
            Selected::Number(*value as f64)
        }
        (StructuredSourceType::Boolean, StructuredValue::Boolean(value)) => {
            Selected::Boolean(*value)
        }
        (StructuredSourceType::String, StructuredValue::String(value)) => Selected::String(value),
        (StructuredSourceType::StringSet, StructuredValue::StringSet(values)) => {
            Selected::StringSet(values)
        }
        (StructuredSourceType::Date, StructuredValue::String(value)) => {
            Selected::Date(parse_gregorian_date(value, path)?)
        }
        (expected, actual) => {
            return Err(core_error(
                CALYX_STRUCTURED_TYPE_MISMATCH,
                format!(
                    "structured field {path:?} declares {expected:?}, found {}",
                    value_kind(actual)
                ),
                "supply the exact declared source type without coercion or register a new binding",
            ));
        }
    };
    Ok(selected)
}

fn numeric_operand(
    record: &calyx_core::StructuredRecord,
    path: &str,
    source_type: NumericSourceType,
) -> Result<f64> {
    let value = record
        .get(path)
        .ok_or_else(|| missing_field(path, record.schema_id()))?;
    match (source_type, value) {
        (NumericSourceType::Number, StructuredValue::Number { bits }) => Ok(f64::from_bits(*bits)),
        (NumericSourceType::Integer, StructuredValue::Integer(value)) => Ok(*value as f64),
        (expected, actual) => Err(core_error(
            CALYX_STRUCTURED_TYPE_MISMATCH,
            format!(
                "derived operand {path:?} declares {expected:?}, found {}",
                value_kind(actual)
            ),
            "supply the exact declared numeric operand type or register a new binding",
        )),
    }
}

fn derive(
    left: f64,
    right: f64,
    operation: NumericDerivation,
    left_path: &str,
    right_path: &str,
) -> Result<f64> {
    if operation == NumericDerivation::Divide && right == 0.0 {
        return Err(structured_runtime_error(
            CALYX_STRUCTURED_DERIVATION_INVALID,
            format!("cannot divide derived operand {left_path:?} by zero-valued {right_path:?}"),
        ));
    }
    let value = match operation {
        NumericDerivation::Add => left + right,
        NumericDerivation::Subtract => left - right,
        NumericDerivation::Multiply => left * right,
        NumericDerivation::Divide => left / right,
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(structured_runtime_error(
            CALYX_STRUCTURED_DERIVATION_INVALID,
            format!(
                "derived operation {operation:?} over {left_path:?}/{right_path:?} produced a non-finite result"
            ),
        ))
    }
}

fn encode_selected(binding: &StructuredBinding, selected: Selected<'_>) -> Result<SlotVector> {
    match (&binding.encoder, selected) {
        (StructuredEncoder::Numeric { scaling }, Selected::Number(value)) => {
            dense(vec![scaled(value, *scaling)?], binding)
        }
        (StructuredEncoder::Boolean, Selected::Boolean(value)) => {
            dense(vec![if value { 1.0 } else { 0.0 }], binding)
        }
        (StructuredEncoder::OneHot { categories }, Selected::String(value)) => {
            let index = category_index(categories, value, binding)?;
            let mut data = vec![0.0; categories.len()];
            data[index] = 1.0;
            dense(data, binding)
        }
        (StructuredEncoder::StableHash { dim }, Selected::String(value)) => {
            let hash = blake3::hash(&[b"CXSTRUCTHASH1\0".as_slice(), value.as_bytes()].concat());
            let index = u64::from_be_bytes(hash.as_bytes()[0..8].try_into().map_err(|_| {
                structured_runtime_error(
                    CALYX_STRUCTURED_DERIVATION_INVALID,
                    "stable hash digest length is invalid",
                )
            })?) % u64::from(*dim);
            let sign = if hash.as_bytes()[8] & 1 == 0 {
                1.0
            } else {
                -1.0
            };
            Ok(SlotVector::Sparse {
                dim: *dim,
                entries: vec![SparseEntry {
                    idx: index as u32,
                    val: sign,
                }],
            })
        }
        (StructuredEncoder::SetMultiHot { vocabulary }, Selected::StringSet(values)) => {
            let mut entries = Vec::with_capacity(values.len());
            for value in values {
                entries.push(SparseEntry {
                    idx: category_index(vocabulary, value, binding)? as u32,
                    val: 1.0,
                });
            }
            entries.sort_by_key(|entry| entry.idx);
            Ok(SlotVector::Sparse {
                dim: vocabulary.len() as u32,
                entries,
            })
        }
        (StructuredEncoder::Ordinal { categories }, Selected::String(value)) => {
            let index = category_index(categories, value, binding)?;
            dense(vec![index as f32 / (categories.len() - 1) as f32], binding)
        }
        (StructuredEncoder::Cyclic { period, offset }, Selected::Number(value)) => {
            cycle(value - offset, *period, binding)
        }
        (StructuredEncoder::DateCyclic { component }, Selected::Date(date)) => {
            let (value, period) = match component {
                DateComponent::Month => (f64::from(date.month - 1), 12.0),
                DateComponent::Weekday => (f64::from(date.weekday_monday_zero()), 7.0),
                DateComponent::DayOfYear => (
                    f64::from(date.day_of_year() - 1),
                    f64::from(date.days_in_year()),
                ),
            };
            cycle(value, period, binding)
        }
        (encoder, _) => Err(core_error(
            CALYX_STRUCTURED_TYPE_MISMATCH,
            format!(
                "structured encoder {encoder:?} received a selector value incompatible with its frozen source type"
            ),
            "repair the frozen binding and register a new LensId",
        )),
    }
}

fn scaled(value: f64, scaling: NumericScaling) -> Result<f32> {
    let scaled = match scaling {
        NumericScaling::Raw => value,
        NumericScaling::MinMax { min, max } => (value - min) / (max - min),
        NumericScaling::ZScore { mean, stddev } => (value - mean) / stddev,
        NumericScaling::Log1pSigned => value.signum() * value.abs().ln_1p(),
    };
    finite_f32(scaled, "numeric structured encoding")
}

fn cycle(value: f64, period: f64, binding: &StructuredBinding) -> Result<SlotVector> {
    let angle = std::f64::consts::TAU * value.rem_euclid(period) / period;
    dense(
        vec![
            finite_f32(angle.sin(), "cyclic sine")?,
            finite_f32(angle.cos(), "cyclic cosine")?,
        ],
        binding,
    )
}

fn dense(data: Vec<f32>, binding: &StructuredBinding) -> Result<SlotVector> {
    if data.iter().any(|value| !value.is_finite()) {
        return Err(non_finite("structured encoder emitted NaN or infinity"));
    }
    let dim = match binding.shape() {
        calyx_core::SlotShape::Dense(dim) => dim,
        other => {
            return Err(structured_runtime_error(
                CALYX_STRUCTURED_DERIVATION_INVALID,
                format!("dense structured encoder declared non-dense shape {other:?}"),
            ));
        }
    };
    if data.len() != dim as usize {
        return Err(structured_runtime_error(
            CALYX_STRUCTURED_DERIVATION_INVALID,
            format!(
                "dense structured encoder emitted {} values for declared dimension {dim}",
                data.len()
            ),
        ));
    }
    Ok(SlotVector::Dense { dim, data })
}

fn category_index(
    categories: &[String],
    value: &str,
    binding: &StructuredBinding,
) -> Result<usize> {
    categories
        .iter()
        .position(|category| category == value)
        .ok_or_else(|| {
            structured_runtime_error(
                CALYX_STRUCTURED_CATEGORY_UNKNOWN,
                format!(
                    "structured value {value:?} is not in the frozen vocabulary for selector {:?}",
                    binding.selector
                ),
            )
        })
}

fn finite_f32(value: f64, context: &str) -> Result<f32> {
    let encoded = value as f32;
    if value.is_finite() && encoded.is_finite() {
        Ok(encoded)
    } else {
        Err(non_finite(format!(
            "{context} produced value {value:?} outside finite f32 range"
        )))
    }
}

fn missing_field(path: &str, schema_id: &str) -> CalyxError {
    core_error(
        CALYX_STRUCTURED_FIELD_MISSING,
        format!("structured schema {schema_id:?} is missing required field {path:?}"),
        "supply the required selected field under the exact frozen schema",
    )
}

fn non_finite(message: impl Into<String>) -> CalyxError {
    core_error(
        CALYX_STRUCTURED_NON_FINITE,
        message,
        "supply finite representable source values or register a different encoding",
    )
}

fn core_error(
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

const fn value_kind(value: &StructuredValue) -> &'static str {
    match value {
        StructuredValue::Number { .. } => "number",
        StructuredValue::Integer(_) => "integer",
        StructuredValue::Boolean(_) => "boolean",
        StructuredValue::String(_) => "string",
        StructuredValue::StringSet(_) => "string_set",
    }
}
