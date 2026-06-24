use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{LensCost, Placement};
use calyx_registry::{LensHealth, LensRuntime, LensSpec, PlacementBudget};

use super::*;

#[test]
fn catalog_replacement_removes_prior_name_manifest_or_id() {
    let mut catalog = LensCatalog {
        lenses: vec![
            entry("same-id", "old", "old.json"),
            entry("other-id", "same-name", "other.json"),
            entry("third-id", "third", "same-path.json"),
            entry("keep-id", "keep", "keep.json"),
        ],
    };

    retain_unrelated_entries(
        &mut catalog,
        "same-id",
        "same-name",
        Path::new("same-path.json"),
    );

    assert_eq!(catalog.lenses.len(), 1);
    assert_eq!(catalog.lenses[0].lens_id, "keep-id");
}

#[test]
fn multimodal_adapter_cost_counts_files_as_cpu_ram() {
    let root = temp_root("multimodal-cost");
    let (_model, adapter, files) = multimodal_fixture(&root, "cpu_explicit");
    let expected_bytes = files_size(&files).unwrap();
    let spec = multimodal_spec(&adapter, files);

    let cost = estimate_lens_cost(&spec).unwrap();

    assert_eq!(cost.vram_bytes, 0);
    assert_eq!(cost.ram_bytes, expected_bytes);
    assert_eq!(cost.batch_ceiling, u32::MAX);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cuda_multimodal_adapter_cost_counts_files_as_gpu_vram() {
    let root = temp_root("multimodal-cuda-cost");
    let (_model, adapter, files) = multimodal_fixture(&root, "cuda_fail_loud");
    let expected_bytes = files_size(&files).unwrap();
    let spec = multimodal_spec(&adapter, files);

    let cost = estimate_lens_cost(&spec).unwrap();
    let placement = placement_from_spec(
        &spec,
        cost,
        PlacementBudget {
            vram_soft_cap_bytes: expected_bytes + 1,
            tei_reserved_bytes: 0,
            vram_allocated_bytes: 0,
            ram_soft_cap_bytes: 64,
            ram_used_bytes: 0,
            cpu_resident_limit: 1,
            cpu_resident_count: 0,
        },
    )
    .unwrap();

    assert_eq!(cost.vram_bytes, expected_bytes);
    assert_eq!(cost.ram_bytes, expected_bytes);
    assert_eq!(placement, Placement::Gpu);
    let _ = fs::remove_dir_all(root);
}

fn multimodal_fixture(root: &Path, provider: &str) -> (PathBuf, PathBuf, Vec<PathBuf>) {
    let helper = root.join("helper.py");
    fs::write(&helper, b"print('not used')").unwrap();
    let model = root.join("model.onnx");
    fs::write(&model, [1_u8; 11]).unwrap();
    let adapter = root.join("adapter.json");
    fs::write(
        &adapter,
        format!(
            r#"{{
  "schema": "calyx-multimodal-adapter-v2",
  "engine": "onnx-external",
  "axis": "image",
  "model_id": "fixture/model",
  "processor_model_id": "fixture/model",
  "dim": 16,
  "python": "python3",
  "helper": "helper.py",
  "model_file": "model.onnx",
  "provider": "{provider}"
}}"#
        ),
    )
    .unwrap();
    let files = vec![model.clone(), adapter.clone()];
    (model, adapter, files)
}

fn multimodal_spec(adapter: &Path, files: Vec<PathBuf>) -> LensSpec {
    LensSpec {
        name: "fixture-image-adapter".to_string(),
        runtime: LensRuntime::MultimodalAdapter {
            axis: "image".to_string(),
            model_id: "fixture/model".to_string(),
            adapter_config: Some(adapter.to_path_buf()),
            files,
        },
        output: calyx_core::SlotShape::Dense(16),
        modality: calyx_core::Modality::Image,
        weights_sha256: [1_u8; 32],
        corpus_hash: [2_u8; 32],
        norm_policy: calyx_registry::NormPolicy::unit(),
        max_batch: None,
        axis: Some("image:fixture/model".to_string()),
        asymmetry: calyx_core::Asymmetry::None,
        quant_default: calyx_core::QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: 0.02,
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn entry(lens_id: &str, name: &str, manifest: &str) -> LensCatalogEntry {
    LensCatalogEntry {
        lens_id: lens_id.to_string(),
        name: name.to_string(),
        modality: "text".to_string(),
        runtime: "onnx_colbert".to_string(),
        dim: 384,
        retrieval_only: false,
        excluded_from_dedup: false,
        weights_sha256: "00".repeat(32),
        manifest: PathBuf::from(manifest),
        cost: LensCost::zero(),
        placement: Placement::Cpu,
    }
}

#[test]
fn list_health_uses_metadata_without_reading_missing_artifact() {
    let root = temp_root("list-health-metadata");
    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        r#"{
  "name": "missing-artifact",
  "modality": "text",
  "runtime": "onnx-int8",
  "dim": 384,
  "shape": {"kind": "dense", "dim": 384},
  "dtype": "int8",
  "weights_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
  "artifact_set_sha256": null,
  "files": [
    {"role": "model", "path": "missing.onnx", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 123}
  ],
  "pooling": "mean",
  "norm": "unit",
  "source_hf_id": "fixture/missing",
  "license": "apache-2.0",
  "non_commercial": false,
  "quant_default": {"turbo_quant": {"bits_per_channel_x2": 7}},
  "truncate_dim": null,
  "recall_delta": 0.02
}"#,
    )
    .unwrap();

    assert_eq!(health_from_manifest(&manifest), LensHealth::Cold);
    let _ = fs::remove_dir_all(root);
}

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn vram_budget_env_overrides_win_without_probe() {
    // Both overrides set: returned verbatim, probe (deliberately bogus) ignored.
    let (cap, tei) =
        compute_vram_budget(Some(30 * GIB), Some(3 * GIB), Some((1, 1)), 4 * GIB).unwrap();
    assert_eq!(cap, 30 * GIB);
    assert_eq!(tei, 3 * GIB);
}

#[test]
fn vram_budget_derives_from_live_reading() {
    // No overrides: cap = board_total - headroom, reservation = live used. This
    // is the fresh-checkout path that the old fixed 20 GiB default broke.
    let total = 32607 * 1024 * 1024; // a 5090 board, NVML MiB -> bytes
    let used = 3182 * 1024 * 1024; // the resident TEI footprint
    let (cap, tei) = compute_vram_budget(None, None, Some((total, used)), 4 * GIB).unwrap();
    assert_eq!(cap, total - 4 * GIB);
    assert_eq!(tei, used);
    // The lens budget left for new GPU lenses is healthy, not starved to near-zero.
    assert!(cap - tei > 24 * GIB);
}

#[test]
fn vram_budget_partial_override_blends_with_probe() {
    // Cap pinned by operator, reservation derived from live device usage.
    let used = 5 * GIB;
    let (cap, tei) =
        compute_vram_budget(Some(28 * GIB), None, Some((40 * GIB, used)), 4 * GIB).unwrap();
    assert_eq!(cap, 28 * GIB);
    assert_eq!(tei, used);
}

#[test]
fn vram_budget_fails_closed_without_probe_or_override() {
    // No overrides AND no probe reading: must error, never guess a budget.
    let err = compute_vram_budget(None, None, None, 4 * GIB).unwrap_err();
    assert!(err.to_string().contains("VRAM probe reading required"));
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-catalog-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
