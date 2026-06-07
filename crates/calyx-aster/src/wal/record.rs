//! WAL record framing.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

pub(super) const MAGIC: u32 = u32::from_le_bytes(*b"CXW1");
pub(super) const HEADER_LEN: usize = 20;
pub(super) const MAX_RECORD_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DecodeStatus {
    Complete(DecodedRecord),
    Eof,
    Torn { offset: u64, message: String },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct DecodedRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
    pub start_offset: u64,
    pub end_offset: u64,
}

pub(super) fn encode(seq: u64, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_RECORD_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("WAL payload exceeds max record size {MAX_RECORD_BYTES}"),
        ));
    }

    let len = payload.len() as u32;
    let crc = payload_crc(seq, len, payload);
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&MAGIC.to_le_bytes());
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

pub(super) fn decode_at(file: &mut File, offset: u64) -> io::Result<DecodeStatus> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; HEADER_LEN];
    let read = file.read(&mut header)?;
    if read == 0 {
        return Ok(DecodeStatus::Eof);
    }
    if read < HEADER_LEN {
        return Ok(DecodeStatus::Torn {
            offset,
            message: format!("partial WAL header: {read}/{HEADER_LEN} bytes"),
        });
    }

    let magic = u32::from_le_bytes(header[0..4].try_into().expect("magic width"));
    if magic != MAGIC {
        return Ok(DecodeStatus::Torn {
            offset,
            message: format!("bad WAL magic 0x{magic:08x}"),
        });
    }

    let seq = u64::from_le_bytes(header[4..12].try_into().expect("seq width"));
    let len = u32::from_le_bytes(header[12..16].try_into().expect("len width"));
    let expected_crc = u32::from_le_bytes(header[16..20].try_into().expect("crc width"));
    if len > MAX_RECORD_BYTES {
        return Ok(DecodeStatus::Torn {
            offset,
            message: format!("record length {len} exceeds max {MAX_RECORD_BYTES}"),
        });
    }

    let mut payload = vec![0u8; len as usize];
    if let Err(error) = file.read_exact(&mut payload) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(DecodeStatus::Torn {
                offset,
                message: format!("partial WAL payload for seq {seq}: wanted {len} bytes"),
            });
        }
        return Err(error);
    }

    let actual_crc = payload_crc(seq, len, &payload);
    if actual_crc != expected_crc {
        return Ok(DecodeStatus::Torn {
            offset,
            message: format!(
                "crc mismatch for seq {seq}: expected {expected_crc:08x}, got {actual_crc:08x}"
            ),
        });
    }

    Ok(DecodeStatus::Complete(DecodedRecord {
        seq,
        payload,
        start_offset: offset,
        end_offset: offset + HEADER_LEN as u64 + len as u64,
    }))
}

fn payload_crc(seq: u64, len: u32, payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(&len.to_le_bytes());
    hasher.update(payload);
    hasher.finalize()
}
