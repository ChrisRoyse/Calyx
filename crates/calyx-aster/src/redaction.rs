//! PII input redaction modes for privacy-preserving ingest.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

/// Module-local error for raw PII at a hash-only ingest boundary.
pub const CALYX_PII_REDACTION_REQUIRED: &str = "CALYX_PII_REDACTION_REQUIRED";

/// How an ingest caller allows Calyx to persist source input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Persist the raw input text.
    Full(String),
    /// Persist only the BLAKE3 hash of the raw bytes.
    HashOnly([u8; 32]),
    /// Persist neither raw bytes nor a source hash; vectors only.
    Redacted,
}

/// Converts raw input text to deterministic hash-only mode.
pub fn redact_to_hash(raw: &str) -> InputMode {
    InputMode::HashOnly(*blake3::hash(raw.as_bytes()).as_bytes())
}

/// Ensures a value is safe for a hash-only ingest boundary.
pub fn assert_hash_only_mode(mode: &InputMode) -> Result<()> {
    match mode {
        InputMode::HashOnly(_) | InputMode::Redacted => Ok(()),
        InputMode::Full(_) => Err(pii_redaction_required(
            "raw input text is not allowed when hash-only input is required",
        )),
    }
}

/// Builds the input mode expected by the ingest boundary.
pub fn pii_input_for_ingest(raw: &str, require_redacted: bool) -> InputMode {
    if require_redacted {
        redact_to_hash(raw)
    } else {
        InputMode::Full(raw.to_string())
    }
}

fn pii_redaction_required(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_PII_REDACTION_REQUIRED,
        message: message.into(),
        remediation: "use HashOnly or Redacted input mode before crossing the ingest boundary",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HELLO_WORLD_BLAKE3: [u8; 32] = [
        0xd7, 0x49, 0x81, 0xef, 0xa7, 0x0a, 0x0c, 0x88, 0x0b, 0x8d, 0x8c, 0x19, 0x85, 0xd0, 0x75,
        0xdb, 0xcb, 0xf6, 0x79, 0xb9, 0x9a, 0x5f, 0x99, 0x14, 0xe5, 0xaa, 0xf9, 0x6b, 0x83, 0x1a,
        0x9e, 0x24,
    ];

    #[test]
    fn redact_to_hash_matches_hello_world_golden() {
        assert_eq!(redact_to_hash("hello world"), hello_world_hash_only());
    }

    #[test]
    fn assert_hash_only_mode_rejects_full_and_accepts_safe_modes() {
        let full = InputMode::Full("pii".to_string());
        let hash_only = hello_world_hash_only();
        let redacted = InputMode::Redacted;
        let error = assert_hash_only_mode(&full).unwrap_err();
        println!("EDGE_FULL_BEFORE mode=Full(\"pii\")");
        println!("EDGE_FULL_AFTER Err({})", error.code);
        assert_eq!(error.code, CALYX_PII_REDACTION_REQUIRED);

        println!("EDGE_HASH_ONLY_BEFORE mode=HashOnly(hello_world)");
        println!(
            "EDGE_HASH_ONLY_AFTER {:?}",
            assert_hash_only_mode(&hash_only)
        );
        assert!(assert_hash_only_mode(&hash_only).is_ok());

        println!("EDGE_REDACTED_BEFORE mode=Redacted");
        println!("EDGE_REDACTED_AFTER {:?}", assert_hash_only_mode(&redacted));
        assert!(assert_hash_only_mode(&redacted).is_ok());
    }

    #[test]
    fn pii_input_for_ingest_selects_expected_mode() {
        assert_eq!(
            pii_input_for_ingest("hello world", true),
            hello_world_hash_only()
        );
        assert_eq!(
            pii_input_for_ingest("hello world", false),
            InputMode::Full("hello world".to_string())
        );
    }

    proptest! {
        #[test]
        fn redact_to_hash_is_hash_only_and_deterministic(raw in ".{0,1024}") {
            let first = redact_to_hash(&raw);
            let second = redact_to_hash(&raw);
            prop_assert!(matches!(first, InputMode::HashOnly(_)));
            prop_assert_eq!(first, second);
        }
    }

    #[test]
    fn redaction_fsv_readback_prints_known_outcomes() {
        let mode = redact_to_hash("hello world");
        let hash = match mode {
            InputMode::HashOnly(hash) => hash,
            InputMode::Full(_) | InputMode::Redacted => panic!("expected hash-only"),
        };
        println!("redact_to_hash(\"hello world\") = HashOnly({})", hex(&hash));

        let full_error =
            assert_hash_only_mode(&InputMode::Full("hello world".to_string())).unwrap_err();
        println!("assert_hash_only_mode(Full) = Err({})", full_error.code);

        assert_eq!(hash, HELLO_WORLD_BLAKE3);
        assert_eq!(full_error.code, CALYX_PII_REDACTION_REQUIRED);
    }

    fn hello_world_hash_only() -> InputMode {
        InputMode::HashOnly(HELLO_WORLD_BLAKE3)
    }

    fn hex(bytes: &[u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
