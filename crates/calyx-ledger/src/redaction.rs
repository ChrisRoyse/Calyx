//! Ledger payload redaction and secret guardrails.

use calyx_core::{CalyxError, InputRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::entry::ActorId;

const SECRET_TOKEN_MIN: usize = 40;
const MAX_HASH_OR_ID_LEN: usize = 64;
const MAX_QUANT_SLOT_METADATA_LEN: usize = 4096;
const MAX_SOURCE_METADATA_LEN: usize = 128;

/// Per-vault ledger redaction policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactionPolicy {
    pub store_raw_input: bool,
    pub redact_actor_name: bool,
}

impl RedactionPolicy {
    pub const fn new(store_raw_input: bool, redact_actor_name: bool) -> Self {
        Self {
            store_raw_input,
            redact_actor_name,
        }
    }

    /// Rejects payloads that contain secret-like fields or token material.
    pub fn check_payload(payload: &[u8]) -> Result<()> {
        Self::default().check_payload_with_policy(payload)
    }

    /// Rejects payloads using this policy's scanner settings.
    pub fn check_payload_with_policy(&self, payload: &[u8]) -> Result<()> {
        if payload.is_empty() {
            return Ok(());
        }
        match serde_json::from_slice::<Value>(payload) {
            Ok(value) => check_json_value(&value, None),
            Err(_) => check_text_tokens(&String::from_utf8_lossy(payload), None),
        }
    }

    /// Redacts the raw input pointer while preserving the stable content hash.
    pub const fn redact_input_ref(&self, input_ref: &InputRef) -> RedactedInput {
        RedactedInput {
            hash: input_ref.hash,
            redacted: true,
            pointer: None,
        }
    }

    /// Builds a hash/id-only payload from a richer payload builder.
    pub fn apply_to_payload(&self, raw: &PayloadBuilder) -> Vec<u8> {
        let filtered = filter_payload_value(raw.value(), self.store_raw_input);
        serde_json::to_vec(&filtered).expect("serde_json::Value serializes")
    }

    pub fn apply_to_actor(&self, actor: ActorId) -> ActorId {
        if !self.redact_actor_name {
            return actor;
        }
        match actor {
            ActorId::Agent(_) => ActorId::Agent("redacted".to_string()),
            ActorId::Service(_) => ActorId::Service("redacted".to_string()),
            ActorId::System => ActorId::System,
        }
    }
}

/// Hash-only input reference safe for ledger payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedInput {
    pub hash: [u8; 32],
    pub redacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

/// Small JSON payload builder for redaction before append.
#[derive(Clone, Debug, PartialEq)]
pub struct PayloadBuilder {
    value: Value,
}

impl Default for PayloadBuilder {
    fn default() -> Self {
        Self::object()
    }
}

impl PayloadBuilder {
    pub fn object() -> Self {
        Self {
            value: Value::Object(Map::new()),
        }
    }

    pub fn from_value(value: Value) -> Self {
        Self { value }
    }

    pub fn insert_value(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        if !self.value.is_object() {
            self.value = Value::Object(Map::new());
        }
        self.value
            .as_object_mut()
            .expect("value was just normalized to object")
            .insert(key.into(), value);
        self
    }

    pub fn insert_str(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.insert_value(key, Value::String(value.into()))
    }

    pub fn insert_u64(&mut self, key: impl Into<String>, value: u64) -> &mut Self {
        self.insert_value(key, Value::Number(value.into()))
    }

    pub const fn value(&self) -> &Value {
        &self.value
    }
}

fn check_json_value(value: &Value, field: Option<&str>) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_secret_field(key) {
                    return Err(secret_error(format!(
                        "ledger payload field `{key}` is secret-like"
                    )));
                }
                check_json_value(child, Some(key))?;
            }
            Ok(())
        }
        Value::Array(values) => {
            for child in values {
                check_json_value(child, field)?;
            }
            Ok(())
        }
        Value::String(text) => check_text_tokens(text, field),
        _ => Ok(()),
    }
}

fn check_text_tokens(text: &str, field: Option<&str>) -> Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if text_has_no_space_printable_run(text) && !allowed_stable_identifier(text, field) {
        return Err(secret_error(
            "ledger payload contains a long non-whitespace token",
        ));
    }
    for token in token_candidates(text) {
        if token.len() >= SECRET_TOKEN_MIN && !allowed_stable_identifier(token, field) {
            return Err(secret_error("ledger payload contains a token-like secret"));
        }
    }
    Ok(())
}

fn token_candidates(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if is_token_char(ch) {
            start.get_or_insert(index);
            continue;
        }
        if let Some(begin) = start.take() {
            out.push(&text[begin..index]);
        }
    }
    if let Some(begin) = start {
        out.push(&text[begin..]);
    }
    out
}

fn text_has_no_space_printable_run(text: &str) -> bool {
    text.chars().count() >= SECRET_TOKEN_MIN
        && text.chars().all(|ch| ch.is_ascii_graphic())
        && text.chars().all(|ch| !ch.is_whitespace())
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=' | '_' | '-' | '.')
}

fn allowed_stable_identifier(token: &str, field: Option<&str>) -> bool {
    let Some(field) = field else {
        return false;
    };
    let field = normalized_field(field);
    if is_source_metadata_field(&field) {
        return allowed_source_metadata_value(token);
    }
    if field == "signature" {
        return token.len() == 128 && is_hex(token);
    }
    if is_public_key_field(&field) {
        return token.len() == MAX_HASH_OR_ID_LEN && is_hex(token);
    }
    if field.starts_with("quant_slot_") {
        return token.len() <= MAX_QUANT_SLOT_METADATA_LEN && is_hex(token);
    }
    if !field_allows_stable_identifier(&field) || token.len() > MAX_HASH_OR_ID_LEN {
        return false;
    }
    is_hex(token) || is_base58(token) || is_uuid(token)
}

fn field_allows_stable_identifier(field: &str) -> bool {
    let field = normalized_field(field);
    field == "hash"
        || field == "metadata"
        || is_source_metadata_field(&field)
        || field == "input_hash"
        || field == "root"
        || field == "signature"
        || field == "weights_sha256"
        || is_public_key_field(&field)
        || field.ends_with("_hash")
        || field.ends_with("_id")
        || field.ends_with("_sha256")
        || field.ends_with("_digest")
}

fn is_source_metadata_field(field: &str) -> bool {
    matches!(field, METADATA_CHUNK_ID | METADATA_DATABASE_NAME)
}

fn allowed_source_metadata_value(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_SOURCE_METADATA_LEN
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '/'))
}

fn is_secret_field(field: &str) -> bool {
    let field = normalized_field(field);
    if is_public_key_field(&field) {
        return false;
    }
    matches!(
        field.as_str(),
        "password" | "passwd" | "token" | "secret" | "key"
    ) || field.ends_with("_password")
        || field.ends_with("_passwd")
        || field.ends_with("_token")
        || field.ends_with("_secret")
        || field.ends_with("_key")
}

fn is_public_key_field(field: &str) -> bool {
    matches!(field, "signer_pubkey" | "public_key" | "verifying_key")
}

fn normalized_field(field: &str) -> String {
    field
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn is_hex(token: &str) -> bool {
    token.len().is_multiple_of(2) && token.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_base58(token: &str) -> bool {
    token
        .chars()
        .all(|ch| "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(ch))
}

fn is_uuid(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 36
        && matches!(bytes[8], b'-')
        && matches!(bytes[13], b'-')
        && matches!(bytes[18], b'-')
        && matches!(bytes[23], b'-')
        && token
            .chars()
            .filter(|ch| *ch != '-')
            .all(|ch| ch.is_ascii_hexdigit())
}

fn filter_payload_value(value: &Value, store_raw_input: bool) -> Value {
    match value {
        Value::Object(map) => {
            let mut filtered = Map::new();
            for (key, child) in map {
                if key == "input_ref" {
                    filtered.insert(key.clone(), filter_input_ref(child));
                    continue;
                }
                if keep_payload_field(key, store_raw_input) {
                    filtered.insert(key.clone(), filter_payload_value(child, store_raw_input));
                }
            }
            Value::Object(filtered)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|child| filter_payload_value(child, store_raw_input))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn filter_input_ref(value: &Value) -> Value {
    let mut filtered = Map::new();
    if let Some(hash) = value.get("hash") {
        filtered.insert("hash".to_string(), hash.clone());
    }
    filtered.insert("redacted".to_string(), Value::Bool(true));
    Value::Object(filtered)
}

fn keep_payload_field(field: &str, store_raw_input: bool) -> bool {
    let field = normalized_field(field);
    if is_secret_field(&field) {
        return false;
    }
    if is_raw_field(&field) {
        return store_raw_input;
    }
    field == "ts" || field == "redacted" || field_allows_stable_identifier(&field)
}

fn is_raw_field(field: &str) -> bool {
    matches!(
        field,
        "raw" | "raw_bytes" | "raw_input" | "input_bytes" | "plaintext"
    ) || field.ends_with("_raw")
        || field.ends_with("_bytes")
}

fn secret_error(message: impl Into<String>) -> CalyxError {
    CalyxError::ledger_secret_in_payload(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const INPUT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn check_payload_allows_hash_and_ids() {
        let payload = json!({
            "input_hash": INPUT_HASH,
            "cx_id": "0123456789abcdef0123456789abcdef",
            "lens_id": "abcdef0123456789abcdef0123456789",
        });
        let bytes = serde_json::to_vec(&payload).unwrap();

        assert!(RedactionPolicy::check_payload(&bytes).is_ok());
    }

    #[test]
    fn check_payload_allows_public_checkpoint_signature_fields() {
        let payload = json!({
            "tag": "checkpoint_v1",
            "root": INPUT_HASH,
            "signature": "a".repeat(128),
            "signer_pubkey": INPUT_HASH,
        });
        let bytes = serde_json::to_vec(&payload).unwrap();

        assert!(RedactionPolicy::check_payload(&bytes).is_ok());

        let secret_key = serde_json::to_vec(&json!({"api_key": INPUT_HASH})).unwrap();
        assert_secret(secret_key);
    }

    #[test]
    fn check_payload_rejects_secret_fields_and_tokens() {
        let password = serde_json::to_vec(&json!({"password": "hunter2"})).unwrap();
        assert_secret(password);

        let bearer = b"mF9zK4sQ7xP2nT8vB3cD6eG1hJ5lR0uW9yA2bC4dE6";
        assert_secret(bearer.to_vec());
    }

    #[test]
    fn check_payload_edges_are_fail_closed() {
        assert!(RedactionPolicy::check_payload(b"").is_ok());

        let hash_payload = serde_json::to_vec(&json!({"input_hash": INPUT_HASH})).unwrap();
        assert!(RedactionPolicy::check_payload(&hash_payload).is_ok());

        assert_secret(b"0123456789ABCDEFGHIJabcdefghij!@#$%^&*()".to_vec());
    }

    #[test]
    fn check_payload_allows_quant_slot_hex_metadata() {
        let bytes = serde_json::to_vec(
            &json!({"restore":{"candidate":{"metadata":{"quant_slot_0":"ab".repeat(128)}}}}),
        )
        .unwrap();
        assert!(RedactionPolicy::check_payload(&bytes).is_ok());
    }

    #[test]
    fn redacted_input_ref_omits_pointer() {
        let input = InputRef {
            hash: [7; 32],
            pointer: Some("s3://vault/raw/password-path".to_string()),
            redacted: false,
        };

        let redacted = RedactionPolicy::default().redact_input_ref(&input);
        let bytes = serde_json::to_vec(&redacted).unwrap();

        assert_eq!(redacted.hash, [7; 32]);
        assert!(redacted.redacted);
        assert_eq!(redacted.pointer, None);
        assert!(!String::from_utf8(bytes).unwrap().contains("password-path"));
    }

    #[test]
    fn apply_to_payload_keeps_ids_hashes_and_strips_raw_material() {
        let mut builder = PayloadBuilder::default();
        builder
            .insert_str("cx_id", "0123456789abcdef0123456789abcdef")
            .insert_str("lens_id", "abcdef0123456789abcdef0123456789")
            .insert_str("input_hash", INPUT_HASH)
            .insert_str("raw_bytes", "raw password text")
            .insert_str("api_key", "do-not-keep")
            .insert_u64("ts", 123);

        let bytes = RedactionPolicy::default().apply_to_payload(&builder);
        let value: Value = serde_json::from_slice(&bytes).unwrap();

        assert!(value.get("cx_id").is_some());
        assert!(value.get("lens_id").is_some());
        assert!(value.get("input_hash").is_some());
        assert_eq!(value.get("ts"), Some(&json!(123)));
        assert!(value.get("raw_bytes").is_none());
        assert!(value.get("api_key").is_none());
        assert!(RedactionPolicy::check_payload(&bytes).is_ok());
    }

    #[test]
    fn apply_to_payload_keeps_source_metadata_identifiers() {
        let mut builder = PayloadBuilder::default();
        builder.insert_value(
            "metadata",
            json!({
                "chunk_id": "chunk-source-20260614-long-but-bounded",
                "database_name": "production-db/main-source-20260614-long-but-bounded",
                "raw_bytes": "do not keep",
            }),
        );

        let bytes = RedactionPolicy::default().apply_to_payload(&builder);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let metadata = value.get("metadata").unwrap();

        assert_eq!(
            metadata.get(METADATA_CHUNK_ID),
            Some(&json!("chunk-source-20260614-long-but-bounded"))
        );
        assert_eq!(
            metadata.get(METADATA_DATABASE_NAME),
            Some(&json!(
                "production-db/main-source-20260614-long-but-bounded"
            ))
        );
        assert!(metadata.get("raw_bytes").is_none());
        assert!(RedactionPolicy::check_payload(&bytes).is_ok());
    }

    fn assert_secret(payload: Vec<u8>) {
        let error = RedactionPolicy::check_payload(&payload).unwrap_err();
        assert_eq!(error.code, "CALYX_LEDGER_SECRET_IN_PAYLOAD");
    }
}
