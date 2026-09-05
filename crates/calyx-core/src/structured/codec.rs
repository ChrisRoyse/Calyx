use std::collections::BTreeMap;

use crate::Result;

use super::error::{CALYX_STRUCTURED_RECORD_INVALID, structured_error};
use super::record::{MAX_FIELDS, MAX_SET_VALUES, StructuredRecord, StructuredValue};

const MAGIC: &[u8; 6] = b"CXREC1";

pub(crate) fn encode_record(record: &StructuredRecord) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_string(&mut out, &record.schema_id);
    put_u32(&mut out, record.fields.len() as u32);
    for (path, value) in &record.fields {
        put_string(&mut out, path);
        match value {
            StructuredValue::Number { bits } => {
                out.push(0);
                out.extend_from_slice(&bits.to_be_bytes());
            }
            StructuredValue::Integer(value) => {
                out.push(1);
                out.extend_from_slice(&value.to_be_bytes());
            }
            StructuredValue::Boolean(value) => {
                out.push(2);
                out.push(u8::from(*value));
            }
            StructuredValue::String(value) => {
                out.push(3);
                put_string(&mut out, value);
            }
            StructuredValue::StringSet(values) => {
                out.push(4);
                put_u32(&mut out, values.len() as u32);
                for value in values {
                    put_string(&mut out, value);
                }
            }
        }
    }
    out
}

pub(crate) fn decode_record(bytes: &[u8]) -> Result<StructuredRecord> {
    let mut reader = Reader::new(bytes);
    reader.expect(MAGIC)?;
    let schema_id = reader.string()?;
    let count = reader.u32()? as usize;
    if count > MAX_FIELDS {
        return Err(invalid(format!(
            "structured record declares {count} fields; maximum is {MAX_FIELDS}"
        )));
    }
    let mut fields = BTreeMap::new();
    for _ in 0..count {
        let path = reader.string()?;
        let tag = reader.u8()?;
        let value = match tag {
            0 => StructuredValue::number(f64::from_bits(reader.u64()?))?,
            1 => StructuredValue::Integer(reader.i64()?),
            2 => match reader.u8()? {
                0 => StructuredValue::Boolean(false),
                1 => StructuredValue::Boolean(true),
                other => return Err(invalid(format!("boolean tag value {other} is invalid"))),
            },
            3 => StructuredValue::String(reader.string()?),
            4 => {
                let value_count = reader.u32()? as usize;
                if value_count > MAX_SET_VALUES {
                    return Err(invalid(format!(
                        "structured set declares {value_count} values; maximum is {MAX_SET_VALUES}"
                    )));
                }
                let mut values = Vec::with_capacity(value_count);
                for _ in 0..value_count {
                    values.push(reader.string()?);
                }
                StructuredValue::StringSet(values)
            }
            other => return Err(invalid(format!("structured value tag {other} is unknown"))),
        };
        if fields.insert(path.clone(), value).is_some() {
            return Err(invalid(format!(
                "duplicate canonical field pointer {path:?}"
            )));
        }
    }
    reader.finish()?;
    Ok(StructuredRecord { schema_id, fields })
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
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
            Err(invalid("structured record magic/version is invalid"))
        }
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| invalid("u32 decode length"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| invalid("u64 decode length"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn i64(&mut self) -> Result<i64> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| invalid("i64 decode length"))?;
        Ok(i64::from_be_bytes(bytes))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|error| invalid(format!("structured UTF-8 is invalid: {error}")))
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| invalid("structured record length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("structured record is truncated"))?;
        self.offset = end;
        Ok(value)
    }

    fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid(format!(
                "structured record has {} trailing bytes",
                self.bytes.len() - self.offset
            )))
        }
    }
}

fn invalid(message: impl Into<String>) -> crate::CalyxError {
    structured_error(CALYX_STRUCTURED_RECORD_INVALID, message)
}
