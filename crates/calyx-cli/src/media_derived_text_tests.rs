use std::collections::BTreeMap;

use calyx_core::{CxId, Input, LEDGER_FIELD_MODEL_ID, LEDGER_FIELD_RUNTIME_ID, Modality};

use crate::media_derived_text::{DerivedTextArtifact, derivation_ledger_payload};
use crate::raw_media::{MediaProbe, RetainedMediaInput};

#[test]
fn derivation_ledger_payload_hashes_runtime_model_metadata() {
    let retained = RetainedMediaInput {
        input: Input::new(Modality::Audio, b"audio".to_vec()),
        pointer: "calyx-vault://inputs/media/audio/source.wav".to_string(),
        source_sha256: "aa".repeat(32),
        input_blake3: [0x11; 32],
        bytes: 5,
        extension: "wav".to_string(),
        probe: MediaProbe {
            codec: "pcm_s16le".to_string(),
            container: "wav".to_string(),
            duration_seconds: Some(1.0),
            sample_rate_hz: Some(16_000),
            channels: Some(1),
            width: None,
            height: None,
            frame_count: None,
            fps: None,
        },
    };
    let derived = DerivedTextArtifact {
        artifact_id: "artifact_a".to_string(),
        input: Input::new(Modality::Text, b"transcript".to_vec()),
        metadata: BTreeMap::new(),
        pointer: "calyx-vault://inputs/derived_text/transcript/t.txt".to_string(),
        text_sha256: "bb".repeat(32),
        kind: "transcript",
        runtime: "whisper.cpp@6fc7c33b4c3a".to_string(),
        model: format!("ggml-tiny.en@sha256:{}", "cc".repeat(32)),
        language: Some("en".to_string()),
        confidence: None,
    };
    let payload = derivation_ledger_payload(
        &retained,
        &derived,
        CxId::from_bytes([0x22; 16]),
        CxId::from_bytes([0x33; 16]),
    )
    .unwrap();

    calyx_ledger::RedactionPolicy::check_payload(&payload).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(value[LEDGER_FIELD_MODEL_ID].as_str().unwrap().len(), 64);
    assert_eq!(value[LEDGER_FIELD_RUNTIME_ID].as_str().unwrap().len(), 64);
    assert!(value.get("model").is_none());
    assert!(value.get("runtime").is_none());
}
