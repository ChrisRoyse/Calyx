use crate::Result;

use super::error::{CALYX_STRUCTURED_SELECTOR_INVALID, structured_error};

/// Validates a non-root RFC 6901 JSON Pointer used to address one field.
pub fn validate_json_field_pointer(pointer: &str) -> Result<()> {
    if pointer.is_empty() || !pointer.starts_with('/') {
        return Err(structured_error(
            CALYX_STRUCTURED_SELECTOR_INVALID,
            format!("field pointer {pointer:?} must start with '/' and select one field"),
        ));
    }
    let bytes = pointer.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            let Some(next) = bytes.get(index + 1) else {
                return Err(invalid_escape(pointer, index));
            };
            if !matches!(next, b'0' | b'1') {
                return Err(invalid_escape(pointer, index));
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(())
}

/// Escapes one object key into an RFC 6901 reference token.
pub fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn invalid_escape(pointer: &str, index: usize) -> crate::CalyxError {
    structured_error(
        CALYX_STRUCTURED_SELECTOR_INVALID,
        format!(
            "field pointer {pointer:?} has an invalid '~' escape at byte {index}; only ~0 and ~1 are valid"
        ),
    )
}
