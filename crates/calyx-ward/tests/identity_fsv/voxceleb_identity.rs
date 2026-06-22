#[path = "voxceleb_identity/artifact.rs"]
mod artifact;
#[path = "voxceleb_identity/codec.rs"]
mod codec;
#[path = "voxceleb_identity/data.rs"]
mod data;
#[path = "voxceleb_identity/edge.rs"]
mod edge;
#[path = "voxceleb_identity/evaluation.rs"]
mod evaluation;
#[path = "voxceleb_identity/math.rs"]
mod math;

use std::fs;

use calyx_ward::{SpeakerLens, WAVLM_DIM};
use serde_json::json;

use artifact::{embedding_json, file_state, write_blake3_manifest, write_json};
use codec::decode_wav_pcm16_mono;
use data::{
    display, env_path, load_voxceleb_clips, overlap_clips, required_path_env, sha256_file_hex,
    speaker_count, speaker_provider_policy, synthetic_clip,
};
use edge::write_edge_readbacks;
use evaluation::evaluate_embeddings;

const DEFAULT_DATASET_ROOT: &str = "/zfs/archive/calyx/datasets/voxceleb1_mini_issue608";
const DEFAULT_WAVLM_MODEL_PATH: &str = "/var/lib/calyx/models/wavlm/wavlm-base-plus-sv.onnx";
const GUARD_UUID: &str = "60800000-7070-7000-8000-000000000608";
const SPEAKER_SLOT: u16 = 8;
const CALIBRATION_TS: i64 = 1_786_147_200;
const MIN_SAMPLES: usize = 50;
const MIN_SPEAKERS: usize = 2;
const MI_K: usize = 3;
const SPEAKER_MI_THRESHOLD_BITS: f32 = 0.05;

const CALYX_WARD_VOXCELEB_EMPTY_DATASET: &str = "CALYX_WARD_VOXCELEB_EMPTY_DATASET";
const CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR: &str = "CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR";
const CALYX_WARD_VOXCELEB_INSUFFICIENT_SAMPLES: &str = "CALYX_WARD_VOXCELEB_INSUFFICIENT_SAMPLES";
const CALYX_WARD_VOXCELEB_BAD_WAV: &str = "CALYX_WARD_VOXCELEB_BAD_WAV";
const CALYX_WARD_VOXCELEB_TAU_OVERLAP: &str = "CALYX_WARD_VOXCELEB_TAU_OVERLAP";
const CALYX_WARD_VOXCELEB_VERDICT_MISMATCH: &str = "CALYX_WARD_VOXCELEB_VERDICT_MISMATCH";
const CALYX_WARD_VOXCELEB_MI_BELOW_THRESHOLD: &str = "CALYX_WARD_VOXCELEB_MI_BELOW_THRESHOLD";

#[test]
fn issue608_voxceleb_edges_fail_closed_with_codes() {
    assert_eq!(
        evaluate_embeddings(&[]).unwrap_err().code,
        CALYX_WARD_VOXCELEB_EMPTY_DATASET
    );
    assert_eq!(
        evaluate_embeddings(&[synthetic_clip("speaker-a", 0, [1.0, 0.0])])
            .unwrap_err()
            .code,
        CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR
    );
    assert_eq!(
        decode_wav_pcm16_mono(b"not a wav").unwrap_err().code,
        CALYX_WARD_VOXCELEB_BAD_WAV
    );
    assert_eq!(
        evaluate_embeddings(&overlap_clips()).unwrap_err().code,
        CALYX_WARD_VOXCELEB_TAU_OVERLAP
    );
}

#[test]
#[ignore = "manual FSV for #608 VoxCeleb Ward speaker identity-lock"]
fn issue608_voxceleb_speaker_identity_fsv_writes_readbacks() {
    let root = required_path_env("CALYX_ISSUE608_FSV_ROOT");
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE608_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create fsv root");
    let artifact_path = root.join("voxceleb-speaker-identity-readback.json");
    let before = file_state(&artifact_path);
    let edges = write_edge_readbacks(&root);

    let dataset_root = env_path("CALYX_ISSUE608_VOXCELEB_ROOT", DEFAULT_DATASET_ROOT);
    let model_path = env_path("CALYX_WARD_WAVLM_MODEL", DEFAULT_WAVLM_MODEL_PATH);
    let lens = SpeakerLens::new_with_provider_policy(&model_path, speaker_provider_policy())
        .expect("load WavLM speaker lens");
    let clips = load_voxceleb_clips(&dataset_root, &lens).expect("load VoxCeleb clips");
    let evaluation = evaluate_embeddings(&clips).expect("evaluate speaker identity");

    let fixture_readback = write_json(
        &root.join("fixture-readback.json"),
        &json!({
            "dataset_root": display(&dataset_root),
            "dataset_repo": "s3prl/mini_voxceleb1",
            "selected_files": display(&dataset_root.join("selected-files.txt")),
            "selected_files_sha256": sha256_file_hex(&dataset_root.join("selected-files.txt")),
            "dataset_sha256_manifest": display(&dataset_root.join("SHA256SUMS.txt")),
            "dataset_sha256_manifest_sha256": sha256_file_hex(&dataset_root.join("SHA256SUMS.txt")),
            "sample_count": clips.len(),
            "speaker_count": speaker_count(&clips),
            "wavlm_model": display(&model_path),
            "wavlm_model_sha256": sha256_file_hex(&model_path),
            "provider_policy": lens.provider_policy(),
            "input_names": lens.input_names(),
            "output_names": lens.output_names(),
        }),
    );
    let embeddings_readback = write_json(
        &root.join("embeddings-readback.json"),
        &json!({
            "slot": SPEAKER_SLOT,
            "dim": WAVLM_DIM,
            "samples": clips.iter().map(embedding_json).collect::<Vec<_>>(),
        }),
    );
    let guard_readback = write_json(
        &root.join("guard-verdicts-readback.json"),
        &evaluation["identity_lock"],
    );
    let mi_readback = write_json(
        &root.join("speaker-mi-readback.json"),
        &evaluation["speaker_mi"],
    );
    let artifact = json!({
        "artifact_kind": "ph70.ward-voxceleb-speaker-identity.v1",
        "source_of_truth": "real VoxCeleb WAV bytes, WavLM speaker embeddings, per-slot Ward guard verdicts, and speaker-MI readback JSON files",
        "trigger": {
            "operation": "calyx-ward issue608 VoxCeleb identity FSV",
            "event": "embed real VoxCeleb clips, calibrate speaker tau, guard genuine and impostor pairs, compute MI(label;speaker_slot)",
            "intended_outcome": "genuine pairs pass, impostor pairs fail, speaker MI exceeds load-bearing threshold",
        },
        "before": before,
        "fixture": fixture_readback,
        "evaluation": evaluation,
        "edges": edges,
    });
    write_json(&artifact_path, &artifact);
    let after = file_state(&artifact_path);
    let manifest = write_blake3_manifest(&root);

    assert_eq!(
        artifact["evaluation"]["checks"]["all_genuine_pass"],
        json!(true)
    );
    assert_eq!(
        artifact["evaluation"]["checks"]["all_impostor_fail"],
        json!(true)
    );
    assert_eq!(
        artifact["evaluation"]["checks"]["speaker_mi_pass"],
        json!(true)
    );
    assert_eq!(after["exists"], json!(true));

    println!("ISSUE608_FSV_ROOT={}", root.display());
    println!("ISSUE608_ARTIFACT={}", artifact_path.display());
    println!("ISSUE608_FIXTURE={}", fixture_readback.display());
    println!("ISSUE608_EMBEDDINGS={}", embeddings_readback.display());
    println!("ISSUE608_GUARD={}", guard_readback.display());
    println!("ISSUE608_MI={}", mi_readback.display());
    println!("ISSUE608_BLAKE3={}", manifest.display());
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}
