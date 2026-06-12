use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::artifact::file_state;
use super::codec::decode_wav_pcm16_mono;
use super::data::{EmbeddedClip, FsvError, overlap_clips, synthetic_clip};
use super::evaluation::evaluate_embeddings;

pub(super) fn write_edge_readbacks(root: &Path) -> Value {
    let edge_root = root.join("edges");
    fs::create_dir_all(&edge_root).expect("create edge root");
    json!({
        "empty_dataset": edge_eval(&edge_root, "empty_dataset", &[]),
        "single_speaker": edge_eval(&edge_root, "single_speaker", &[synthetic_clip("speaker-a", 0, [1.0, 0.0])]),
        "bad_wav": edge_bad_wav(&edge_root),
        "tau_overlap": edge_eval(&edge_root, "tau_overlap", &overlap_clips()),
    })
}

fn edge_eval(root: &Path, label: &str, clips: &[EmbeddedClip]) -> Value {
    let out = root.join(label).join("artifact.json");
    fs::create_dir_all(out.parent().unwrap()).expect("create edge");
    let before = file_state(&out);
    let result = evaluate_embeddings(clips);
    let after = file_state(&out);
    edge_result(before, after, result.map(|_| ()))
}

fn edge_bad_wav(root: &Path) -> Value {
    let out = root.join("bad_wav").join("artifact.json");
    fs::create_dir_all(out.parent().unwrap()).expect("create bad wav edge");
    let before = file_state(&out);
    let result = decode_wav_pcm16_mono(b"not a wav").map(|_| ());
    let after = file_state(&out);
    edge_result(before, after, result)
}

fn edge_result(before: Value, after: Value, result: Result<(), FsvError>) -> Value {
    match result {
        Ok(()) => json!({"before": before, "after": after, "success": true}),
        Err(error) => json!({
            "before": before,
            "after": after,
            "success": false,
            "error_code": error.code,
            "message": error.message,
        }),
    }
}
