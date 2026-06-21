use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Asymmetry, Modality, Placement, QuantPolicy, SlotShape, SlotVector, SparseEntry};
use calyx_registry::{LensRuntime, LensSpec, NormPolicy};

use super::measure::{compare_vectors, effective_batch};
use super::model::{
    DEFAULT_MAX_ABS_DELTA, DEFAULT_MIN_BATCH_COSINE, Flags, LensAudit, TEMPORAL_LANE_ROLE,
};
use super::probe::probe_set;
use super::report::build_report;
use super::runtime::{association_family, is_temporal_sidecar};

#[test]
fn temporal_sidecar_does_not_count_toward_content_floor() {
    let mut spec = spec(
        "temporal-as-of-sidecar",
        LensRuntime::Algorithmic {
            kind: "scalar".into(),
        },
    );
    spec.axis = Some("temporal-as-of".to_string());

    assert!(is_temporal_sidecar(&spec));
    assert_eq!(association_family(&spec), "temporal_sidecar");
    assert_eq!(
        TEMPORAL_LANE_ROLE,
        "time_manipulation_walk_forward_backward_as_of_sidecar"
    );
}

#[test]
fn effective_batch_clamps_manifest_ceiling() {
    assert_eq!(effective_batch(64, Some(1)), 1);
    assert_eq!(effective_batch(64, Some(8)), 8);
    assert_eq!(effective_batch(64, None), 64);
}

#[test]
fn panel_floor_rejects_when_gpu_count_is_short() {
    let flags = Flags {
        manifests: vec![PathBuf::from("a.json")],
        out: PathBuf::from("report.json"),
        batch_size: 8,
        min_content_lenses: 1,
        min_gpu_content_lenses: 1,
        min_effective_batch: 2,
        min_batch_cosine: DEFAULT_MIN_BATCH_COSINE,
        max_abs_delta: DEFAULT_MAX_ABS_DELTA,
        lens_timeout_secs: 30,
        probes: Vec::new(),
        probe_files: Vec::new(),
        worker: false,
    };
    let report = build_report(vec![accepted_cpu_lens()], &flags);

    assert!(!report.accepted);
    assert!(
        report
            .rejections
            .iter()
            .any(|r| r.code == "CALYX_LENS_SCALE_GPU_CONTENT_FLOOR")
    );
}

#[test]
fn default_image_audio_probes_are_binary_and_hashed() {
    let flags = flags();

    let image = probe_set(&flags, Modality::Image, 2).expect("image probes");
    assert_eq!(image.inputs.len(), 2);
    assert!(image.inputs[0].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(image.evidence[0].source, "builtin:image/png-1x1-rgb");
    assert_eq!(image.evidence[0].bytes, image.inputs[0].bytes.len());
    assert_eq!(image.evidence[0].sha256.len(), 64);

    let audio = probe_set(&flags, Modality::Audio, 2).expect("audio probes");
    assert_eq!(audio.inputs.len(), 2);
    assert!(audio.inputs[0].bytes.starts_with(b"RIFF"));
    assert_eq!(&audio.inputs[0].bytes[8..12], b"WAVE");
    assert_eq!(audio.evidence[0].source, "builtin:audio/wav-16khz-1s-sine");
    assert_eq!(audio.evidence[0].bytes, audio.inputs[0].bytes.len());
    assert_eq!(audio.evidence[0].sha256.len(), 64);
}

#[test]
fn probe_files_are_selected_by_media_magic() {
    let root = temp_root("probe-files");
    let image_path = root.join("probe.png");
    let audio_path = root.join("probe.wav");
    fs::write(
        &image_path,
        probe_set(&flags(), Modality::Image, 1).unwrap().inputs[0]
            .bytes
            .clone(),
    )
    .unwrap();
    fs::write(
        &audio_path,
        probe_set(&flags(), Modality::Audio, 1).unwrap().inputs[0]
            .bytes
            .clone(),
    )
    .unwrap();
    let mut flags = flags();
    flags.probe_files = vec![image_path.clone(), audio_path.clone()];

    let image = probe_set(&flags, Modality::Image, 1).expect("image file probe");
    assert_eq!(image.evidence.len(), 1);
    assert_eq!(image.evidence[0].path.as_ref(), Some(&image_path));

    let audio = probe_set(&flags, Modality::Audio, 1).expect("audio file probe");
    assert_eq!(audio.evidence.len(), 1);
    assert_eq!(audio.evidence[0].path.as_ref(), Some(&audio_path));
}

#[test]
fn unsupported_probe_file_fails_closed() {
    let root = temp_root("bad-probe");
    let bad_path = root.join("bad.bin");
    fs::write(&bad_path, b"not-media").unwrap();
    let mut flags = flags();
    flags.probe_files = vec![bad_path];

    let error = probe_set(&flags, Modality::Image, 1).unwrap_err();
    assert_eq!(error.code, "CALYX_LENS_SCALE_PROBE_UNSUPPORTED");
}

#[test]
fn sparse_batch_stability_accepts_identical_sparse_vectors() {
    let vector = SlotVector::Sparse {
        dim: 64,
        entries: vec![
            SparseEntry { idx: 3, val: 0.25 },
            SparseEntry { idx: 17, val: 0.75 },
        ],
    };

    let stability = compare_vectors(
        std::slice::from_ref(&vector),
        std::slice::from_ref(&vector),
        0.999,
        0.0,
    )
    .expect("sparse batch compare");

    assert!(stability.acceptable);
    assert_eq!(stability.sample_rows, 1);
    assert!((stability.min_cosine - 1.0).abs() <= f32::EPSILON);
    assert_eq!(stability.max_abs_delta, 0.0);
}

#[test]
fn multi_batch_stability_accepts_identical_token_vectors() {
    let vector = SlotVector::Multi {
        token_dim: 4,
        tokens: vec![vec![0.5, 0.0, 0.0, 0.5], vec![0.0, 0.25, 0.75, 0.0]],
    };

    let stability = compare_vectors(
        std::slice::from_ref(&vector),
        std::slice::from_ref(&vector),
        0.999,
        0.0,
    )
    .expect("multi batch compare");

    assert!(stability.acceptable);
    assert_eq!(stability.sample_rows, 1);
    assert!((stability.min_cosine - 1.0).abs() <= f32::EPSILON);
    assert_eq!(stability.max_abs_delta, 0.0);
}

fn accepted_cpu_lens() -> LensAudit {
    LensAudit {
        manifest: PathBuf::from("a.json"),
        lens_id: "01".to_string(),
        name: "cpu-static".to_string(),
        modality: Modality::Text,
        runtime: "static_lookup".to_string(),
        runtime_detail: "static_lookup_mmap;cpu_explicit".to_string(),
        provider: "cpu_explicit".to_string(),
        placement: Placement::Cpu,
        association_family: "static_lookup_semantic".to_string(),
        temporal_sidecar: false,
        counts_toward_content_floor: true,
        weights_sha256: "00".repeat(32),
        dim: 8,
        max_batch: Some(1),
        requested_batch_size: 8,
        effective_batch_size: 1,
        native_batching: false,
        provider_placement_proof: "cpu_runtime_not_gpu_claim".to_string(),
        gpu_process_observed: None,
        probe_evidence: Vec::new(),
        rows_per_sec: Some(1.0),
        batch_stability: None,
        accepted: true,
        rejections: Vec::new(),
    }
}

fn flags() -> Flags {
    Flags {
        manifests: vec![PathBuf::from("a.json")],
        out: PathBuf::from("report.json"),
        batch_size: 8,
        min_content_lenses: 1,
        min_gpu_content_lenses: 0,
        min_effective_batch: 2,
        min_batch_cosine: DEFAULT_MIN_BATCH_COSINE,
        max_abs_delta: DEFAULT_MAX_ABS_DELTA,
        lens_timeout_secs: 30,
        probes: Vec::new(),
        probe_files: Vec::new(),
        worker: false,
    }
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("calyx-scale-audit-{label}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn spec(name: &str, runtime: LensRuntime) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output: SlotShape::Dense(8),
        modality: Modality::Text,
        weights_sha256: [1; 32],
        corpus_hash: [2; 32],
        norm_policy: NormPolicy::unit(),
        max_batch: None,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}
