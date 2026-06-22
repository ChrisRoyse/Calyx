use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use calyx_core::{Input, Lens, Modality, SlotShape, SlotVector};
use calyx_ward::{
    DEFAULT_WAVLM_MODEL_PATH, SpeakerEmbeddingBackend, SpeakerLens, WAVLM_DIM, WAVLM_SAMPLE_RATE,
    WardError,
};
use proptest::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn mock_backend_embedding_is_unit_norm() {
    let lens = mock_lens(WAVLM_DIM);
    let embedding = lens
        .embed_speaker(&speech_segment(), WAVLM_SAMPLE_RATE)
        .unwrap();

    assert_eq!(embedding.len(), WAVLM_DIM);
    assert_norm(&embedding, 1.0e-5);
}

#[test]
fn identical_audio_is_deterministic() {
    let lens = mock_lens(WAVLM_DIM);
    let audio = speech_segment();

    let first = lens.embed_speaker(&audio, WAVLM_SAMPLE_RATE).unwrap();
    let second = lens.embed_speaker(&audio, WAVLM_SAMPLE_RATE).unwrap();

    assert_eq!(first, second);
}

#[test]
fn edge_padding_does_not_move_embedding() {
    let lens = mock_lens(WAVLM_DIM);
    let core = speech_segment();
    let mut padded = vec![0.0; 24];
    padded.extend(core.iter().copied());
    padded.extend([0.0; 31]);

    let base = lens.embed_speaker(&core, WAVLM_SAMPLE_RATE).unwrap();
    let padded = lens.embed_speaker(&padded, WAVLM_SAMPLE_RATE).unwrap();

    assert!(cosine(&base, &padded) >= 0.99);
    assert_eq!(base, padded);
}

#[test]
fn resampled_audio_still_returns_unit_norm() {
    let lens = mock_lens(WAVLM_DIM);
    let embedding = lens.embed_speaker(&speech_segment(), 8_000).unwrap();

    assert_eq!(embedding.len(), WAVLM_DIM);
    assert_norm(&embedding, 1.0e-5);
}

#[test]
fn empty_audio_fails_closed() {
    let lens = mock_lens(WAVLM_DIM);
    let error = lens.embed_speaker(&[], WAVLM_SAMPLE_RATE).unwrap_err();

    assert!(matches!(error, WardError::InvalidInput { .. }));
    assert_eq!(error.code(), "CALYX_WARD_INVALID_INPUT");
}

#[test]
fn nonfinite_audio_fails_closed() {
    let lens = mock_lens(WAVLM_DIM);
    let error = lens
        .embed_speaker(&[0.1, f32::NAN, 0.2], WAVLM_SAMPLE_RATE)
        .unwrap_err();

    assert!(matches!(error, WardError::InvalidInput { .. }));
}

#[test]
fn absent_model_reports_model_not_found_code() {
    let missing = temp_path("missing-wavlm-model.onnx");
    let error = SpeakerLens::new_cpu_explicit(&missing).unwrap_err();

    assert!(matches!(error, WardError::ModelNotFound { .. }));
    assert_eq!(error.code(), "CALYX_WARD_MODEL_NOT_FOUND");
    assert!(error.to_string().contains("missing-wavlm-model.onnx"));
}

#[test]
fn wrong_backend_dim_fails_at_construction() {
    let error =
        SpeakerLens::from_backend(temp_path("wrong-dim.onnx"), [3; 32], MockBackend::new(128))
            .unwrap_err();

    assert_eq!(error.code(), "CALYX_WARD_MODEL_DIM_MISMATCH");
}

#[test]
fn lens_trait_measures_audio_pcm_bytes() {
    let lens = mock_lens(WAVLM_DIM);
    let bytes = pcm_bytes(&speech_segment());
    let measured = lens.measure(&Input::new(Modality::Audio, bytes)).unwrap();

    assert_eq!(lens.shape(), SlotShape::Dense(WAVLM_DIM as u32));
    match measured {
        SlotVector::Dense { dim, data } => {
            assert_eq!(dim, WAVLM_DIM as u32);
            assert_norm(&data, 1.0e-5);
        }
        other => panic!("expected dense speaker vector, got {other:?}"),
    }
}

#[test]
fn lens_trait_rejects_wrong_modality() {
    let lens = mock_lens(WAVLM_DIM);
    let error = lens
        .measure(&Input::new(Modality::Text, b"not audio".to_vec()))
        .unwrap_err();

    assert_eq!(error.code, "CALYX_WARD_INVALID_INPUT");
}

#[test]
fn lens_trait_rejects_malformed_pcm_bytes() {
    let lens = mock_lens(WAVLM_DIM);
    let error = lens
        .measure(&Input::new(Modality::Audio, vec![0, 1, 2]))
        .unwrap_err();

    assert_eq!(error.code, "CALYX_WARD_INVALID_INPUT");
}

proptest! {
    #[test]
    fn proptest_embeddings_are_unit_norm(samples in prop::collection::vec(-1.0f32..1.0, 1..128)) {
        let lens = mock_lens(WAVLM_DIM);
        let embedding = lens.embed_speaker(&samples, WAVLM_SAMPLE_RATE).unwrap();
        prop_assert_eq!(embedding.len(), WAVLM_DIM);
        let norm = embedding.iter().map(|value| value * value).sum::<f32>().sqrt();
        prop_assert!((norm - 1.0).abs() < 1.0e-5);
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_SPEAKER_LENS_FSV_DIR"]
fn issue270_speaker_lens_fsv_writes_readbacks() {
    let root = PathBuf::from(
        std::env::var("CALYX_WARD_SPEAKER_LENS_FSV_DIR")
            .expect("CALYX_WARD_SPEAKER_LENS_FSV_DIR is required"),
    );
    std::fs::create_dir_all(&root).expect("create FSV root");
    let model = PathBuf::from(
        std::env::var("CALYX_WARD_WAVLM_MODEL").unwrap_or_else(|_| DEFAULT_WAVLM_MODEL_PATH.into()),
    );
    let missing = root.join("missing-wavlm.onnx");
    let missing_error = SpeakerLens::new_cpu_explicit(&missing).unwrap_err();

    let cpu = SpeakerLens::new_cpu_explicit(&model).expect("load CPU WavLM speaker lens");
    let cuda = SpeakerLens::new(&model).expect("load CUDA WavLM speaker lens");
    let silence = vec![0.0; WAVLM_SAMPLE_RATE as usize];
    let first = cpu
        .embed_speaker(&silence, WAVLM_SAMPLE_RATE)
        .expect("embed silence once");
    let second = cpu
        .embed_speaker(&silence, WAVLM_SAMPLE_RATE)
        .expect("embed silence twice");
    let cuda_vec = cuda
        .embed_speaker(&silence, WAVLM_SAMPLE_RATE)
        .expect("embed silence on CUDA");
    let deterministic_delta = max_abs_diff(&first, &second);
    let cpu_cuda_delta = max_abs_diff(&first, &cuda_vec);
    assert!(deterministic_delta <= 1.0e-7);
    assert!(cpu_cuda_delta <= 1.0e-3);

    let model_hash = sha256_file_hex(&model);
    let files = [
        write_json(
            &root,
            "model-readback.json",
            &json!({
                "model_path": model,
                "model_sha256": model_hash,
                "cpu_policy": cpu.provider_policy(),
                "cuda_policy": cuda.provider_policy(),
                "input_names": cpu.input_names(),
                "output_names": cpu.output_names(),
                "lens_id": cpu.id().to_string(),
            }),
        ),
        write_json(
            &root,
            "speaker-embedding.json",
            &json!({ "dim": first.len(), "embedding": first }),
        ),
        write_json(
            &root,
            "norm-determinism.json",
            &json!({
                "norm": norm(&second),
                "deterministic_max_abs_diff": deterministic_delta,
                "cpu_cuda_max_abs_diff": cpu_cuda_delta,
            }),
        ),
        write_json(
            &root,
            "model-missing-error.json",
            &json!({
                "code": missing_error.code(),
                "message": missing_error.to_string(),
            }),
        ),
    ];
    write_manifest(&root, &files);
}

#[derive(Debug)]
struct MockBackend {
    dim: usize,
}

impl MockBackend {
    const fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl SpeakerEmbeddingBackend for MockBackend {
    fn embed_16khz(&self, audio_pcm: &[f32]) -> Result<Vec<f32>, WardError> {
        let sum = audio_pcm.iter().sum::<f32>();
        let energy = audio_pcm.iter().map(|value| value * value).sum::<f32>();
        let len = audio_pcm.len() as f32;
        Ok((0..self.dim)
            .map(|idx| {
                let k = idx as f32 + 1.0;
                (k * 0.013 + sum * 0.07 + energy * 0.003 + len * 0.000_01).sin()
            })
            .collect())
    }

    fn output_dim(&self) -> usize {
        self.dim
    }
}

fn mock_lens(dim: usize) -> SpeakerLens {
    SpeakerLens::from_backend(temp_path("mock-wavlm.onnx"), [7; 32], MockBackend::new(dim))
        .expect("construct mock speaker lens")
}

fn speech_segment() -> Vec<f32> {
    (0..96)
        .map(|idx| ((idx as f32) * 0.11).sin() * 0.25)
        .collect()
}

fn pcm_bytes(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("calyx-ward-{name}"))
}

fn assert_norm(data: &[f32], eps: f32) {
    assert!((norm(data) - 1.0).abs() <= eps);
}

fn norm(data: &[f32]) -> f32 {
    data.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>()
}

fn max_abs_diff(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f32::max)
}

fn write_json(root: &Path, name: &str, value: &serde_json::Value) -> PathBuf {
    let path = root.join(name);
    let file = File::create(&path).expect("create FSV JSON");
    serde_json::to_writer_pretty(file, value).expect("write FSV JSON");
    path
}

fn write_manifest(root: &Path, files: &[PathBuf]) {
    let mut manifest = File::create(root.join("SHA256SUMS.txt")).expect("create manifest");
    for path in files {
        writeln!(
            manifest,
            "{}  {}",
            sha256_file_hex(path),
            path.file_name().unwrap().to_string_lossy()
        )
        .expect("write manifest row");
    }
}

fn sha256_file_hex(path: &Path) -> String {
    let mut file = File::open(path).expect("open file for hash");
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).expect("read file for hash");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
