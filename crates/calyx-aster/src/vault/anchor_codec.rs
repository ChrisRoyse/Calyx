use super::cursor::Cursor;
use calyx_core::{Anchor, AnchorKind, AnchorValue, CalyxError, Result};

pub fn encode_anchor(anchor: &Anchor) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    encode_anchor_kind(&anchor.kind, &mut out)?;
    encode_anchor_value(&anchor.value, &mut out)?;
    put_string(&mut out, &anchor.source)?;
    out.extend_from_slice(&anchor.observed_at.to_be_bytes());
    out.extend_from_slice(&anchor.confidence.to_bits().to_be_bytes());
    Ok(out)
}

pub fn decode_anchor(bytes: &[u8]) -> Result<Anchor> {
    let mut cursor = Cursor::new(bytes);
    let kind = decode_anchor_kind(&mut cursor)?;
    let value = decode_anchor_value(&mut cursor)?;
    let source = cursor.string()?;
    let observed_at = cursor.u64()?;
    let confidence = f32::from_bits(cursor.u32()?);
    Ok(Anchor {
        kind,
        value,
        source,
        observed_at,
        confidence,
    })
}

fn encode_anchor_kind(kind: &AnchorKind, out: &mut Vec<u8>) -> Result<()> {
    match kind {
        AnchorKind::TestPass => out.extend_from_slice(&0_u16.to_be_bytes()),
        AnchorKind::TieFormed => out.extend_from_slice(&1_u16.to_be_bytes()),
        AnchorKind::Thumbs => out.extend_from_slice(&2_u16.to_be_bytes()),
        AnchorKind::Label(value) => {
            out.extend_from_slice(&3_u16.to_be_bytes());
            put_string(out, value)?;
        }
        AnchorKind::Reward => out.extend_from_slice(&4_u16.to_be_bytes()),
        AnchorKind::SpeakerMatch => out.extend_from_slice(&5_u16.to_be_bytes()),
        AnchorKind::StyleHold => out.extend_from_slice(&6_u16.to_be_bytes()),
        AnchorKind::Recurrence => out.extend_from_slice(&7_u16.to_be_bytes()),
    }
    Ok(())
}

fn decode_anchor_kind(cursor: &mut Cursor<'_>) -> Result<AnchorKind> {
    Ok(match cursor.u16()? {
        0 => AnchorKind::TestPass,
        1 => AnchorKind::TieFormed,
        2 => AnchorKind::Thumbs,
        3 => AnchorKind::Label(cursor.string()?),
        4 => AnchorKind::Reward,
        5 => AnchorKind::SpeakerMatch,
        6 => AnchorKind::StyleHold,
        7 => AnchorKind::Recurrence,
        tag => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "unknown anchor kind {tag}"
            )));
        }
    })
}

fn encode_anchor_value(value: &AnchorValue, out: &mut Vec<u8>) -> Result<()> {
    match value {
        AnchorValue::Bool(value) => out.extend_from_slice(&[0, u8::from(*value)]),
        AnchorValue::Enum(value) => {
            out.push(1);
            put_string(out, value)?;
        }
        AnchorValue::Number(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        AnchorValue::OneHot(values) => {
            out.push(3);
            out.extend_from_slice(&(values.len() as u32).to_be_bytes());
            for value in values {
                put_string(out, value)?;
            }
        }
        AnchorValue::Text(value) => {
            out.push(4);
            put_string(out, value)?;
        }
        AnchorValue::Vector(values) => {
            out.push(5);
            out.extend_from_slice(&(values.len() as u32).to_be_bytes());
            for value in values {
                if !value.is_finite() {
                    return Err(CalyxError::aster_corrupt_shard(
                        "anchor vector contains non-finite value",
                    ));
                }
                out.extend_from_slice(&value.to_bits().to_be_bytes());
            }
        }
    }
    Ok(())
}

fn decode_anchor_value(cursor: &mut Cursor<'_>) -> Result<AnchorValue> {
    Ok(match cursor.u8()? {
        0 => AnchorValue::Bool(cursor.u8()? != 0),
        1 => AnchorValue::Enum(cursor.string()?),
        2 => AnchorValue::Number(f64::from_bits(cursor.u64()?)),
        3 => {
            let count = cursor.u32()? as usize;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(cursor.string()?);
            }
            AnchorValue::OneHot(values)
        }
        4 => AnchorValue::Text(cursor.string()?),
        5 => {
            let count = cursor.u32()? as usize;
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                let value = f32::from_bits(cursor.u32()?);
                if !value.is_finite() {
                    return Err(CalyxError::aster_corrupt_shard(
                        "anchor vector contains non-finite value",
                    ));
                }
                values.push(value);
            }
            AnchorValue::Vector(values)
        }
        tag => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "unknown anchor value {tag}"
            )));
        }
    })
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    put_bytes(out, value.as_bytes())
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| CalyxError::aster_corrupt_shard("encoded field too large"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
