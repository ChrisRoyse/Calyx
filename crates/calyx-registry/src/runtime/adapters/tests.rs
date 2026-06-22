use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Input, Lens, Modality};

use super::{
    CALYX_LICENSE_DENIED, MultimodalAdapterLens, MultimodalAdapterSpec, MultimodalAxis,
    default_multimodal_lens_specs,
};
use crate::LensHealth;

#[test]
fn adapter_requires_real_config_and_registers_loaded_contract() {
    let fixture = adapter_fixture("loaded", MultimodalAxis::Image, 768);
    let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "fixture-image",
        MultimodalAxis::Image,
        768,
        Some(fixture.config.clone()),
        None,
        false,
    ))
    .unwrap();
    let spec = lens.lens_spec();
    let contract = lens.contract();

    assert_eq!(spec.health(), LensHealth::Loaded);
    contract.verify_registration(&lens).unwrap();

    let reloaded = MultimodalAdapterLens::from_lens_spec(&spec).unwrap();
    contract.verify_registration(&reloaded).unwrap();
}

#[test]
fn missing_adapter_config_fails_closed() {
    let error = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "missing",
        MultimodalAxis::Image,
        768,
        None,
        None,
        false,
    ))
    .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
    assert!(error.message.contains("config is required"));
}

#[test]
fn cuda_fail_loud_provider_loads_from_real_config() {
    let fixture = adapter_fixture_with_provider(
        "cuda-provider",
        MultimodalAxis::Image,
        128,
        "cuda_fail_loud",
    );
    let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "fixture-cuda-image",
        MultimodalAxis::Image,
        128,
        Some(fixture.config),
        None,
        false,
    ))
    .unwrap();

    assert!(lens.provider().is_gpu());
    assert_eq!(
        lens.provider_detail(),
        "cuda:0,error_on_failure,no_cpu_fallback"
    );
}

#[test]
fn cuda_preferred_provider_loads_from_real_config() {
    let fixture = adapter_fixture_with_provider(
        "cuda-preferred-provider",
        MultimodalAxis::Image,
        128,
        "cuda_preferred",
    );
    let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "fixture-cuda-preferred-image",
        MultimodalAxis::Image,
        128,
        Some(fixture.config),
        None,
        false,
    ))
    .unwrap();

    assert!(lens.provider().is_gpu());
    assert_eq!(lens.provider_detail(), "cuda:0,allow_cpu_fallback");
}

#[test]
fn unsupported_adapter_provider_fails_closed() {
    let fixture = adapter_fixture_with_provider("bad-provider", MultimodalAxis::Image, 128, "auto");
    let error = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "fixture-bad-provider",
        MultimodalAxis::Image,
        128,
        Some(fixture.config),
        None,
        false,
    ))
    .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
    assert!(
        error
            .message
            .contains("unsupported multimodal adapter provider auto")
    );
}

#[test]
fn malformed_inputs_return_typed_errors_before_helper_spawn() {
    let cases = [
        (
            MultimodalAxis::Image,
            Input::new(Modality::Image, b"not-an-image".to_vec()),
        ),
        (
            MultimodalAxis::Audio,
            Input::new(Modality::Audio, b"RIFFbad".to_vec()),
        ),
        (
            MultimodalAxis::Protein,
            Input::new(Modality::Protein, b"ACDZ".to_vec()),
        ),
        (
            MultimodalAxis::Dna,
            Input::new(Modality::Dna, b"ACGTX".to_vec()),
        ),
        (
            MultimodalAxis::Molecule,
            Input::new(Modality::Molecule, b"C?O".to_vec()),
        ),
    ];

    for (axis, input) in cases {
        let fixture = adapter_fixture(axis.as_str(), axis, 16);
        let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
            "bad",
            axis,
            16,
            Some(fixture.config),
            None,
            false,
        ))
        .unwrap();
        let error = lens.measure(&input).unwrap_err();
        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
        assert!(error.message.contains(axis.as_str()));
        assert!(!fixture.marker.exists());
    }
}

#[test]
fn license_gate_denies_noncommercial_before_config_load() {
    let denied = MultimodalAdapterLens::from_adapter_spec(adapter_spec(
        "nc-dna",
        MultimodalAxis::Dna,
        16,
        None,
        Some("CC-BY-NC-SA-4.0"),
        false,
    ))
    .unwrap_err();

    assert_eq!(denied.code, CALYX_LICENSE_DENIED);
}

#[test]
fn default_pack_advertises_only_real_priority_axes() {
    let specs = default_multimodal_lens_specs();

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].axis, MultimodalAxis::Image);
    assert_eq!(specs[0].dim, 768);
    assert_eq!(specs[1].axis, MultimodalAxis::Audio);
    assert_eq!(specs[1].dim, 512);
}

struct AdapterFixture {
    config: PathBuf,
    marker: PathBuf,
}

fn adapter_fixture(label: &str, axis: MultimodalAxis, dim: u32) -> AdapterFixture {
    adapter_fixture_with_provider(label, axis, dim, "cpu_explicit")
}

fn adapter_fixture_with_provider(
    label: &str,
    axis: MultimodalAxis,
    dim: u32,
    provider: &str,
) -> AdapterFixture {
    let root = temp_root(label);
    let helper = root.join("helper.py");
    let marker = root.join("helper-ran.marker");
    fs::write(
        &helper,
        format!(
            "from pathlib import Path\nPath({:?}).write_text('ran')\n",
            marker
        ),
    )
    .unwrap();
    let model = root.join("model.onnx");
    fs::write(&model, b"not-used-by-invalid-input-tests").unwrap();
    let config = root.join("adapter.json");
    fs::write(
        &config,
        format!(
            r#"{{
  "schema": "calyx-multimodal-adapter-v2",
  "engine": "onnx-external",
  "axis": "{}",
  "model_id": "fixture/{}",
  "processor_model_id": "fixture/{}",
  "dim": {},
  "python": "python3",
  "helper": "helper.py",
  "model_file": "model.onnx",
  "provider": "{}"
}}"#,
            axis.as_str(),
            axis.as_str(),
            axis.as_str(),
            dim,
            provider
        ),
    )
    .unwrap();
    AdapterFixture { config, marker }
}

fn adapter_spec(
    name: &str,
    axis: MultimodalAxis,
    dim: u32,
    adapter_config: Option<PathBuf>,
    license: Option<&str>,
    allow_non_commercial: bool,
) -> MultimodalAdapterSpec {
    MultimodalAdapterSpec {
        name: name.to_string(),
        axis,
        model_id: format!("fixture/{}", axis.as_str()),
        dim,
        license: license.map(str::to_string),
        allow_non_commercial,
        adapter_config,
        files: Vec::new(),
    }
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-multimodal-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
