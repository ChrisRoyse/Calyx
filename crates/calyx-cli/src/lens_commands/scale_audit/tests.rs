use std::path::PathBuf;

use calyx_core::{Asymmetry, Modality, Placement, QuantPolicy, SlotShape, SlotVector, SparseEntry};
use calyx_registry::{LensRuntime, LensSpec, NormPolicy};

use super::measure::{compare_vectors, effective_batch};
use super::model::{
    DEFAULT_MAX_ABS_DELTA, DEFAULT_MIN_BATCH_COSINE, Flags, LensAudit, TEMPORAL_LANE_ROLE,
};
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
        rows_per_sec: Some(1.0),
        batch_stability: None,
        accepted: true,
        rejections: Vec::new(),
    }
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
