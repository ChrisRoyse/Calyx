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
fn check_payload_allows_only_exact_hex_blake3_digests() {
    let valid = serde_json::to_vec(&json!({
        "tuple_plan_blake3": "ab".repeat(32),
    }))
    .unwrap();
    assert!(RedactionPolicy::check_payload(&valid).is_ok());

    let non_hex = serde_json::to_vec(&json!({
        "tuple_plan_blake3": "g".repeat(64),
    }))
    .unwrap();
    assert_secret(non_hex);

    let overlong = serde_json::to_vec(&json!({
        "tuple_plan_blake3": "ab".repeat(33),
    }))
    .unwrap();
    assert_secret(overlong);
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
fn check_payload_handles_discovery_manifest_tokens() {
    let benign = serde_json::to_vec(&json!({
        "run_id": "issue1221-biomedical-discovery-run-20260704T065206Z",
        "corpus_vault_id": "issue1221-biomedical-discovery-fsv-vault",
        "stages": [{
            "stage_id": "bridge-falsification-to-evaluate",
            "upstream_stage_id": "hypothesis-falsification-sweep",
            "command": "calyx-discovery-run/bridge-falsification-to-evaluate",
            "git_sha": "faac5d1a6b391c584ab8264c33fcbee4892bd53a",
            "input_sha256": INPUT_HASH,
            "output_sha256": INPUT_HASH
        }]
    }))
    .unwrap();
    assert!(RedactionPolicy::check_payload(&benign).is_ok());
    let secret = serde_json::to_vec(&json!({
        "run_id": "mF9zK4sQ7xP2nT8vB3cD6eG1hJ5lR0uW9yA2bC4dE6",
        "git_sha": "not-a-git-sha-secret-token-material-1234567890",
    }))
    .unwrap();

    assert_secret(secret);
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

#[test]
fn check_payload_allows_stable_calyx_code_fields_only() {
    let payload = serde_json::to_vec(&json!({
        "code": "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY",
        "nested": {
            "code": "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_MAKER_CONCENTRATION"
        }
    }))
    .unwrap();
    assert!(RedactionPolicy::check_payload(&payload).is_ok());

    let same_token_wrong_field = serde_json::to_vec(&json!({
        "reason": "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY"
    }))
    .unwrap();
    assert_secret(same_token_wrong_field);
}

fn assert_secret(payload: Vec<u8>) {
    let error = RedactionPolicy::check_payload(&payload).unwrap_err();
    assert_eq!(error.code, "CALYX_LEDGER_SECRET_IN_PAYLOAD");
}
