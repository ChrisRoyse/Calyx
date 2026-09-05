use calyx_core::Result;

use super::reconciliation_error;

pub(super) fn hash_from_blob_name(name: &str) -> Result<[u8; 32]> {
    let hash = name.strip_suffix(".bin").ok_or_else(|| {
        reconciliation_error(format!(
            "pending retained input {name:?} is not a .bin object"
        ))
    })?;
    if hash.len() != 64 {
        return Err(reconciliation_error(format!(
            "pending retained input {name:?} does not contain a 64-hex digest"
        )));
    }
    if !hash
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(reconciliation_error(format!(
            "pending retained input {name:?} is not a canonical lowercase digest name"
        )));
    }
    let mut out = [0_u8; 32];
    for (index, pair) in hash.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0]).ok_or_else(|| invalid_digest_name(name))?;
        let low = hex_nibble(pair[1]).ok_or_else(|| invalid_digest_name(name))?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

pub(super) fn is_pending_temp_name(name: &str) -> bool {
    let Some((hash, suffix)) = name.split_once(".bin.tmp-") else {
        return false;
    };
    let mut parts = suffix.split('-');
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.next().is_none()
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn invalid_digest_name(name: &str) -> calyx_core::CalyxError {
    reconciliation_error(format!(
        "pending retained input {name:?} is not a canonical lowercase digest name"
    ))
}
