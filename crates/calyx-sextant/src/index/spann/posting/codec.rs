use calyx_core::Result;

use super::{PostingMember, corrupt, invalid, validate_sparse_vector};

/// Block layout (v2, #701): `[u32 count]` then per member
/// `[varint delta_cx_id][varint nnz]` followed by `nnz × ([varint idx][f32 val])`.
/// cx_ids are stored as strictly-ascending deltas; each member carries its own
/// sparse vector so search ranks by true query distance, not a static scalar.
pub fn encode_posting_block(entries: &[PostingMember]) -> Result<Vec<u8>> {
    let mut previous = 0_u32;
    let mut raw = Vec::with_capacity(4 + entries.len() * 8);
    raw.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (i, member) in entries.iter().enumerate() {
        if i > 0 && member.cx_id <= previous {
            return Err(invalid("posting cx_ids must be strictly ascending"));
        }
        validate_sparse_vector(&member.vector)?;
        write_varint(member.cx_id.saturating_sub(previous), &mut raw);
        write_varint(member.vector.len() as u32, &mut raw);
        for (idx, val) in &member.vector {
            write_varint(*idx, &mut raw);
            raw.extend_from_slice(&val.to_le_bytes());
        }
        previous = member.cx_id;
    }
    Ok(raw)
}

pub fn decode_posting_block(raw: &[u8]) -> Result<Vec<PostingMember>> {
    if raw.len() < 4 {
        return Err(corrupt(format!("raw posting block is {} B", raw.len())));
    }
    let count = u32::from_le_bytes(raw[0..4].try_into().expect("4B")) as usize;
    let mut cursor = 4;
    let mut previous = 0_u32;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let delta = read_varint(raw, &mut cursor)?;
        let cx_id = previous
            .checked_add(delta)
            .ok_or_else(|| corrupt("posting cx_id delta overflow"))?;
        let nnz = read_varint(raw, &mut cursor)? as usize;
        let mut vector = Vec::with_capacity(nnz);
        for _ in 0..nnz {
            let idx = read_varint(raw, &mut cursor)?;
            let val_bytes = raw
                .get(cursor..cursor + 4)
                .ok_or_else(|| corrupt("truncated posting value"))?;
            cursor += 4;
            let val = f32::from_le_bytes(val_bytes.try_into().expect("4B"));
            if !val.is_finite() {
                return Err(corrupt(format!("posting {cx_id} has non-finite value")));
            }
            vector.push((idx, val));
        }
        entries.push(PostingMember { cx_id, vector });
        previous = cx_id;
    }
    if cursor != raw.len() {
        return Err(corrupt(format!(
            "{} trailing posting bytes",
            raw.len() - cursor
        )));
    }
    Ok(entries)
}

fn write_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(raw: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut value = 0_u32;
    let mut shift = 0;
    loop {
        let byte = *raw
            .get(*cursor)
            .ok_or_else(|| corrupt("truncated posting varint"))?;
        *cursor += 1;
        if shift == 28 && byte > 0x0f {
            return Err(corrupt("posting varint exceeds u32"));
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 28 {
            return Err(corrupt("posting varint exceeds u32"));
        }
    }
}
