//! Deterministic lowercase whitespace/punctuation tokenizer.

use std::collections::BTreeMap;

use calyx_core::Result;
use calyx_core::SparseEntry;

use crate::error::{
    CALYX_SEXTANT_POSTINGS_CORRUPT, CALYX_SEXTANT_POSTINGS_NOT_SORTED, sextant_error,
};

pub const TEXT_SPARSE_DIM: u32 = 1_000_000;

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub fn token_sparse_idx(token: &str) -> u32 {
    let hash = blake3::hash(token.as_bytes());
    let raw = u32::from_be_bytes(
        hash.as_bytes()[0..4]
            .try_into()
            .expect("4-byte hash prefix"),
    );
    raw % TEXT_SPARSE_DIM
}

pub fn token_sparse_key(token: &str) -> String {
    format!("t{}", token_sparse_idx(token))
}

pub fn text_sparse_entries(text: &str) -> Vec<SparseEntry> {
    let mut counts = BTreeMap::<u32, f32>::new();
    for token in tokenize(text) {
        *counts.entry(token_sparse_idx(&token)).or_default() += 1.0;
    }
    counts
        .into_iter()
        .map(|(idx, val)| SparseEntry { idx, val })
        .collect()
}

pub fn encode_varint_deltas(ids: &[u32]) -> Result<Vec<u8>> {
    let mut last = 0;
    let mut out = Vec::new();
    for id in ids {
        if *id < last {
            return Err(sextant_error(
                CALYX_SEXTANT_POSTINGS_NOT_SORTED,
                format!("posting id {id} is smaller than previous id {last}"),
            ));
        }
        let delta = id - last;
        last = *id;
        write_varint(delta, &mut out);
    }
    Ok(out)
}

pub fn decode_varint_deltas(bytes: &[u8]) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    let mut pos = 0;
    let mut last = 0_u32;
    while pos < bytes.len() {
        let (delta, next) = read_varint(bytes, pos)?;
        last = last.checked_add(delta).ok_or_else(|| {
            sextant_error(
                CALYX_SEXTANT_POSTINGS_CORRUPT,
                "posting delta overflowed u32 document id",
            )
        })?;
        ids.push(last);
        pos = next;
    }
    Ok(ids)
}

fn write_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(bytes: &[u8], mut pos: usize) -> Result<(u32, usize)> {
    let mut shift = 0;
    let mut value = 0_u32;
    loop {
        let byte = *bytes.get(pos).ok_or_else(|| {
            sextant_error(
                CALYX_SEXTANT_POSTINGS_CORRUPT,
                "truncated varint postings block",
            )
        })?;
        pos += 1;
        if shift == 28 && byte > 0x0f {
            return Err(sextant_error(
                CALYX_SEXTANT_POSTINGS_CORRUPT,
                "varint postings value exceeds u32",
            ));
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
        shift += 7;
        if shift > 28 {
            return Err(sextant_error(
                CALYX_SEXTANT_POSTINGS_CORRUPT,
                "varint postings value exceeds u32",
            ));
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{CALYX_SEXTANT_POSTINGS_CORRUPT, CALYX_SEXTANT_POSTINGS_NOT_SORTED};

    #[test]
    fn postings_roundtrip_and_empty_are_byte_exact() {
        let encoded = encode_varint_deltas(&[1, 3, 7]).unwrap();

        assert_eq!(hex(&encoded), "010204");
        assert_eq!(decode_varint_deltas(&encoded).unwrap(), vec![1, 3, 7]);
        assert_eq!(encode_varint_deltas(&[]).unwrap(), Vec::<u8>::new());
        assert_eq!(decode_varint_deltas(&[]).unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn postings_fail_closed_unsorted_and_corrupt() {
        let unsorted = encode_varint_deltas(&[3, 1]).unwrap_err();
        let truncated = decode_varint_deltas(&[0x80]).unwrap_err();
        let overflow = decode_varint_deltas(&[0xff, 0xff, 0xff, 0xff, 0x10]).unwrap_err();

        assert_eq!(unsorted.code, CALYX_SEXTANT_POSTINGS_NOT_SORTED);
        assert_eq!(truncated.code, CALYX_SEXTANT_POSTINGS_CORRUPT);
        assert_eq!(overflow.code, CALYX_SEXTANT_POSTINGS_CORRUPT);
    }
}
