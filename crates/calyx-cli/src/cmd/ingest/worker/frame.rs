use super::*;

pub(crate) fn encode_binary(value: &impl Serialize) -> Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, config::standard()).map_err(|error| {
        CalyxError::lens_unreachable(format!("encode binary frame failed: {error}"))
    })
}

pub(crate) fn decode_binary<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let (value, consumed) =
        bincode::serde::decode_from_slice(bytes, config::standard()).map_err(|error| {
            CalyxError::lens_unreachable(format!("decode binary frame failed: {error}"))
        })?;
    if consumed != bytes.len() {
        return Err(CalyxError::lens_unreachable(format!(
            "decode binary frame consumed {consumed} of {} bytes",
            bytes.len()
        )));
    }
    Ok(value)
}

pub(crate) fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_RESIDENT_FRAME_BYTES {
        return Err(CalyxError::lens_unreachable(format!(
            "resident binary frame {} bytes exceeds max {}",
            bytes.len(),
            MAX_RESIDENT_FRAME_BYTES
        )));
    }
    let len = u64::try_from(bytes.len()).map_err(|_| {
        CalyxError::lens_unreachable(format!(
            "resident binary frame {} bytes overflows u64",
            bytes.len()
        ))
    })?;
    writer
        .write_all(&len.to_be_bytes())
        .and_then(|_| writer.write_all(bytes))
        .map_err(|error| {
            CalyxError::lens_unreachable(format!("write binary frame failed: {error}"))
        })
}

pub(crate) fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>> {
    read_frame_or_eof(reader)?.ok_or_else(|| {
        CalyxError::lens_unreachable("read binary frame failed: stream closed before header")
    })
}

pub(crate) fn read_frame_or_eof(reader: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let Some(header) = read_header_or_eof(reader)? else {
        return Ok(None);
    };
    let len = u64::from_be_bytes(header);
    let len = usize::try_from(len).map_err(|_| {
        CalyxError::lens_unreachable(format!(
            "resident binary frame length {len} overflows usize"
        ))
    })?;
    if len > MAX_RESIDENT_FRAME_BYTES {
        return Err(CalyxError::lens_unreachable(format!(
            "resident binary frame {len} bytes exceeds max {MAX_RESIDENT_FRAME_BYTES}"
        )));
    }
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "read binary frame body ({len} bytes) failed: {error}"
        ))
    })?;
    Ok(Some(body))
}

fn read_header_or_eof(reader: &mut impl Read) -> Result<Option<[u8; 8]>> {
    let mut header = [0_u8; 8];
    let mut offset = 0;
    while offset < header.len() {
        match reader.read(&mut header[offset..]) {
            Ok(0) if offset == 0 => return Ok(None),
            Ok(0) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "read binary frame header failed: stream closed after {offset} of 8 bytes"
                )));
            }
            Ok(n) => offset += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "read binary frame header failed: {error}"
                )));
            }
        }
    }
    Ok(Some(header))
}
