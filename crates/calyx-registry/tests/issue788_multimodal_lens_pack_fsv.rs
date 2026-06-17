use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Input, Lens, Modality};
use calyx_registry::{
    CALYX_LICENSE_DENIED, MultimodalAdapterLens, MultimodalAdapterSpec, MultimodalAxis, Registry,
    register_multimodal_lens_pack,
};
use serde_json::json;

#[test]
fn issue788_multimodal_lens_pack_fsv_readback() {
    let (root, keep_root) = fsv_root();
    fs::create_dir_all(&root).unwrap();

    let mut registry = Registry::new();
    let specs = vec![
        adapter_spec(&root, MultimodalAxis::Image, 768),
        adapter_spec(&root, MultimodalAxis::Audio, 512),
    ];
    let entries = register_multimodal_lens_pack(&mut registry, &specs).unwrap();
    let registrations = entries
        .iter()
        .map(|entry| {
            json!({
                "lens_id": entry.lens_id.to_string(),
                "name": entry.spec.name,
                "modality": format!("{:?}", entry.spec.modality).to_ascii_lowercase(),
                "shape": format!("{:?}", entry.spec.output),
                "health": entry.spec.health(),
                "runtime": entry.spec.runtime,
            })
        })
        .collect::<Vec<_>>();

    let edges = malformed_edges(&root);
    let license = license_gate_readback();
    fs::write(
        root.join("registry-snapshot.json"),
        serde_json::to_vec_pretty(&registry.lens_snapshots()).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("registrations.json"),
        serde_json::to_vec_pretty(&registrations).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("edges.json"),
        serde_json::to_vec_pretty(&edges).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("license.json"),
        serde_json::to_vec_pretty(&license).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&json!({
            "issue": 788,
            "registered_lenses": entries.len(),
            "edge_rows": edges.len(),
            "license_denied_code": license["denied"]["error_code"],
            "hash_fallback_present": false,
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(entries.len(), 2);
    assert!(
        registrations
            .iter()
            .all(|row| row["health"] == json!("loaded"))
    );
    assert!(
        edges
            .iter()
            .all(|row| row["after"]["error_code"] == "CALYX_LENS_DIM_MISMATCH")
    );
    assert_eq!(license["denied"]["error_code"], CALYX_LICENSE_DENIED);

    if !keep_root {
        let _ = fs::remove_dir_all(root);
    }
}

fn fsv_root() -> (PathBuf, bool) {
    if let Some(root) = std::env::var_os("CALYX_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    (
        std::env::temp_dir().join(format!("calyx-issue788-fsv-{}-{nanos}", std::process::id())),
        false,
    )
}

fn malformed_edges(root: &std::path::Path) -> Vec<serde_json::Value> {
    [
        (
            MultimodalAxis::Image,
            Input::new(Modality::Image, b"not-an-image".to_vec()),
            768,
        ),
        (
            MultimodalAxis::Audio,
            Input::new(Modality::Audio, b"RIFFbad".to_vec()),
            512,
        ),
    ]
    .into_iter()
    .map(|(axis, input, dim)| {
        let lens = MultimodalAdapterLens::from_adapter_spec(adapter_spec(root, axis, dim)).unwrap();
        let error = lens.measure(&input).unwrap_err();
        json!({
            "axis": axis.as_str(),
            "before": {
                "input_len": input.bytes.len(),
                "attempted": false,
            },
            "after": {
                "attempted": true,
                "error_code": error.code,
                "error_message": error.message,
            }
        })
    })
    .collect()
}

fn license_gate_readback() -> serde_json::Value {
    let denied = MultimodalAdapterLens::from_adapter_spec(MultimodalAdapterSpec {
        name: "issue788-nc-dna".to_string(),
        axis: MultimodalAxis::Dna,
        model_id: "fixture/dna".to_string(),
        dim: 16,
        license: Some("CC-BY-NC-SA-4.0".to_string()),
        allow_non_commercial: false,
        adapter_config: None,
        files: Vec::new(),
    })
    .unwrap_err();
    json!({
        "denied": {
            "license": "CC-BY-NC-SA-4.0",
            "allow_flag": false,
            "error_code": denied.code,
            "error_message": denied.message,
        }
    })
}

fn adapter_spec(root: &std::path::Path, axis: MultimodalAxis, dim: u32) -> MultimodalAdapterSpec {
    let dir = root.join(format!("fixture-{}", axis.as_str()));
    fs::create_dir_all(&dir).unwrap();
    let helper = dir.join("helper.py");
    fs::write(
        &helper,
        b"raise SystemExit('helper should not run in this test')\n",
    )
    .unwrap();
    let model = dir.join("model.onnx");
    fs::write(&model, b"fixture-model").unwrap();
    let config = dir.join("adapter.json");
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
  "provider": "cpu_explicit"
}}"#,
            axis.as_str(),
            axis.as_str(),
            axis.as_str(),
            dim
        ),
    )
    .unwrap();
    MultimodalAdapterSpec {
        name: format!("issue788-{}", axis.as_str()),
        axis,
        model_id: format!("fixture/{}", axis.as_str()),
        dim,
        license: Some("apache-2.0".to_string()),
        allow_non_commercial: false,
        adapter_config: Some(config),
        files: Vec::new(),
    }
}
