//! Deterministic lowercase whitespace/punctuation tokenizer.

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

pub fn encode_varint_deltas(ids: &[u32]) -> Vec<u8> {
    let mut last = 0;
    let mut out = Vec::new();
    for id in ids {
        let delta = id - last;
        last = *id;
        write_varint(delta, &mut out);
    }
    out
}

pub fn decode_varint_deltas(bytes: &[u8]) -> Option<Vec<u32>> {
    let mut ids = Vec::new();
    let mut pos = 0;
    let mut last = 0;
    while pos < bytes.len() {
        let (delta, next) = read_varint(bytes, pos)?;
        last += delta;
        ids.push(last);
        pos = next;
    }
    Some(ids)
}

fn write_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(bytes: &[u8], mut pos: usize) -> Option<(u32, usize)> {
    let mut shift = 0;
    let mut value = 0_u32;
    loop {
        let byte = *bytes.get(pos)?;
        pos += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, pos));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
