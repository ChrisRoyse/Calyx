use crate::server::{ToolError, ToolResult};

pub(super) fn parse_answer_id(raw: &str) -> ToolResult<Vec<u8>> {
    if raw.is_empty() {
        return Err(ToolError::invalid_params("answer_id must not be empty"));
    }
    if raw.len().is_multiple_of(2)
        && raw.bytes().all(|byte| byte.is_ascii_hexdigit())
        && let Some(bytes) = decode_hex(raw)
    {
        return Ok(bytes);
    }
    Ok(raw.as_bytes().to_vec())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Some((hex_value(chunk[0])? << 4) | hex_value(chunk[1])?))
        .collect()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
