use calyx_core::Result;

use super::binding::{
    DateComponent, MAX_BINDING_BYTES, MAX_CATEGORIES, NumericDerivation, NumericScaling,
    NumericSourceType, StructuredBinding, StructuredEncoder, StructuredSelector,
    StructuredSourceType,
};
use super::error::{CALYX_STRUCTURED_BINDING_INVALID, structured_runtime_error};

const MAGIC: &[u8; 5] = b"CXSB1";

pub(super) fn encode_binding(binding: &StructuredBinding) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_string(&mut out, &binding.schema_id);
    encode_selector(&mut out, &binding.selector);
    out.push(source_type_tag(binding.source_type));
    encode_encoder(&mut out, &binding.encoder);
    out
}

pub(super) fn decode_binding(bytes: &[u8]) -> Result<StructuredBinding> {
    if bytes.len() > MAX_BINDING_BYTES {
        return Err(invalid(
            "structured binding exceeds the canonical byte limit",
        ));
    }
    let mut reader = Reader::new(bytes);
    reader.expect(MAGIC)?;
    let schema_id = reader.string()?;
    let selector = decode_selector(&mut reader)?;
    let source_type = decode_source_type(reader.u8()?)?;
    let encoder = decode_encoder(&mut reader)?;
    reader.finish()?;
    Ok(StructuredBinding {
        schema_id,
        selector,
        source_type,
        encoder,
    })
}

fn encode_selector(out: &mut Vec<u8>, selector: &StructuredSelector) {
    match selector {
        StructuredSelector::Field { path } => {
            out.push(0);
            put_string(out, path);
        }
        StructuredSelector::DerivedNumeric {
            left,
            left_type,
            right,
            right_type,
            operation,
        } => {
            out.push(1);
            put_string(out, left);
            out.push(numeric_source_tag(*left_type));
            put_string(out, right);
            out.push(numeric_source_tag(*right_type));
            out.push(derivation_tag(*operation));
        }
    }
}

fn decode_selector(reader: &mut Reader<'_>) -> Result<StructuredSelector> {
    Ok(match reader.u8()? {
        0 => StructuredSelector::Field {
            path: reader.string()?,
        },
        1 => StructuredSelector::DerivedNumeric {
            left: reader.string()?,
            left_type: decode_numeric_source(reader.u8()?)?,
            right: reader.string()?,
            right_type: decode_numeric_source(reader.u8()?)?,
            operation: decode_derivation(reader.u8()?)?,
        },
        tag => return Err(invalid(format!("unknown structured selector tag {tag}"))),
    })
}

fn encode_encoder(out: &mut Vec<u8>, encoder: &StructuredEncoder) {
    match encoder {
        StructuredEncoder::Numeric { scaling } => {
            out.push(0);
            encode_scaling(out, *scaling);
        }
        StructuredEncoder::Boolean => out.push(1),
        StructuredEncoder::OneHot { categories } => {
            out.push(2);
            put_strings(out, categories);
        }
        StructuredEncoder::StableHash { dim } => {
            out.push(3);
            put_u32(out, *dim);
        }
        StructuredEncoder::SetMultiHot { vocabulary } => {
            out.push(4);
            put_strings(out, vocabulary);
        }
        StructuredEncoder::Ordinal { categories } => {
            out.push(5);
            put_strings(out, categories);
        }
        StructuredEncoder::Cyclic { period, offset } => {
            out.push(6);
            put_f64(out, *period);
            put_f64(out, *offset);
        }
        StructuredEncoder::DateCyclic { component } => {
            out.push(7);
            out.push(date_component_tag(*component));
        }
    }
}

fn decode_encoder(reader: &mut Reader<'_>) -> Result<StructuredEncoder> {
    Ok(match reader.u8()? {
        0 => StructuredEncoder::Numeric {
            scaling: decode_scaling(reader)?,
        },
        1 => StructuredEncoder::Boolean,
        2 => StructuredEncoder::OneHot {
            categories: reader.strings()?,
        },
        3 => StructuredEncoder::StableHash { dim: reader.u32()? },
        4 => StructuredEncoder::SetMultiHot {
            vocabulary: reader.strings()?,
        },
        5 => StructuredEncoder::Ordinal {
            categories: reader.strings()?,
        },
        6 => StructuredEncoder::Cyclic {
            period: reader.f64()?,
            offset: reader.f64()?,
        },
        7 => StructuredEncoder::DateCyclic {
            component: decode_date_component(reader.u8()?)?,
        },
        tag => return Err(invalid(format!("unknown structured encoder tag {tag}"))),
    })
}

fn encode_scaling(out: &mut Vec<u8>, scaling: NumericScaling) {
    match scaling {
        NumericScaling::Raw => out.push(0),
        NumericScaling::MinMax { min, max } => {
            out.push(1);
            put_f64(out, min);
            put_f64(out, max);
        }
        NumericScaling::ZScore { mean, stddev } => {
            out.push(2);
            put_f64(out, mean);
            put_f64(out, stddev);
        }
        NumericScaling::Log1pSigned => out.push(3),
    }
}

fn decode_scaling(reader: &mut Reader<'_>) -> Result<NumericScaling> {
    Ok(match reader.u8()? {
        0 => NumericScaling::Raw,
        1 => NumericScaling::MinMax {
            min: reader.f64()?,
            max: reader.f64()?,
        },
        2 => NumericScaling::ZScore {
            mean: reader.f64()?,
            stddev: reader.f64()?,
        },
        3 => NumericScaling::Log1pSigned,
        tag => return Err(invalid(format!("unknown numeric scaling tag {tag}"))),
    })
}

fn put_strings(out: &mut Vec<u8>, values: &[String]) {
    put_u32(out, values.len() as u32);
    for value in values {
        put_string(out, value);
    }
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_f64(out: &mut Vec<u8>, value: f64) {
    let canonical = if value == 0.0 { 0.0 } else { value };
    out.extend_from_slice(&canonical.to_bits().to_be_bytes());
}

const fn source_type_tag(value: StructuredSourceType) -> u8 {
    match value {
        StructuredSourceType::Number => 0,
        StructuredSourceType::Integer => 1,
        StructuredSourceType::Boolean => 2,
        StructuredSourceType::String => 3,
        StructuredSourceType::StringSet => 4,
        StructuredSourceType::Date => 5,
    }
}

fn decode_source_type(tag: u8) -> Result<StructuredSourceType> {
    Ok(match tag {
        0 => StructuredSourceType::Number,
        1 => StructuredSourceType::Integer,
        2 => StructuredSourceType::Boolean,
        3 => StructuredSourceType::String,
        4 => StructuredSourceType::StringSet,
        5 => StructuredSourceType::Date,
        _ => return Err(invalid(format!("unknown structured source-type tag {tag}"))),
    })
}

const fn numeric_source_tag(value: NumericSourceType) -> u8 {
    match value {
        NumericSourceType::Number => 0,
        NumericSourceType::Integer => 1,
    }
}

fn decode_numeric_source(tag: u8) -> Result<NumericSourceType> {
    Ok(match tag {
        0 => NumericSourceType::Number,
        1 => NumericSourceType::Integer,
        _ => return Err(invalid(format!("unknown numeric source-type tag {tag}"))),
    })
}

const fn derivation_tag(value: NumericDerivation) -> u8 {
    match value {
        NumericDerivation::Add => 0,
        NumericDerivation::Subtract => 1,
        NumericDerivation::Multiply => 2,
        NumericDerivation::Divide => 3,
    }
}

fn decode_derivation(tag: u8) -> Result<NumericDerivation> {
    Ok(match tag {
        0 => NumericDerivation::Add,
        1 => NumericDerivation::Subtract,
        2 => NumericDerivation::Multiply,
        3 => NumericDerivation::Divide,
        _ => return Err(invalid(format!("unknown numeric derivation tag {tag}"))),
    })
}

const fn date_component_tag(value: DateComponent) -> u8 {
    match value {
        DateComponent::Month => 0,
        DateComponent::Weekday => 1,
        DateComponent::DayOfYear => 2,
    }
}

fn decode_date_component(tag: u8) -> Result<DateComponent> {
    Ok(match tag {
        0 => DateComponent::Month,
        1 => DateComponent::Weekday,
        2 => DateComponent::DayOfYear,
        _ => return Err(invalid(format!("unknown date component tag {tag}"))),
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn expect(&mut self, expected: &[u8]) -> Result<()> {
        if self.take(expected.len())? == expected {
            Ok(())
        } else {
            Err(invalid("structured binding magic/version is invalid"))
        }
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let value: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| invalid("u32 decode length"))?;
        Ok(u32::from_be_bytes(value))
    }

    fn f64(&mut self) -> Result<f64> {
        let value: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| invalid("f64 decode length"))?;
        Ok(f64::from_bits(u64::from_be_bytes(value)))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|error| invalid(format!("structured binding UTF-8 is invalid: {error}")))
    }

    fn strings(&mut self) -> Result<Vec<String>> {
        let count = self.u32()? as usize;
        if count > MAX_CATEGORIES {
            return Err(invalid(format!(
                "structured binding declares {count} categories; maximum is {MAX_CATEGORIES}"
            )));
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.string()?);
        }
        Ok(values)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| invalid("structured binding length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("structured binding is truncated"))?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid(format!(
                "structured binding has {} trailing bytes",
                self.bytes.len() - self.offset
            )))
        }
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn unhex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(invalid("structured binding hex has odd length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let high = hex_value(pair[0])
                .ok_or_else(|| invalid(format!("invalid hex at byte {}", index * 2)))?;
            let low = hex_value(pair[1])
                .ok_or_else(|| invalid(format!("invalid hex at byte {}", index * 2 + 1)))?;
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

fn invalid(message: impl Into<String>) -> calyx_core::CalyxError {
    structured_runtime_error(CALYX_STRUCTURED_BINDING_INVALID, message)
}
