use super::*;
use crate::lens_commands::support::hex_from_bytes;

#[test]
fn rejects_sub_ten_content_templates() {
    let template = SavedPanelTemplate {
        schema_version: OBJECT_VERSION,
        name: "too-small".to_string(),
        version: 1,
        notes: String::new(),
        min_content_lenses: MIN_CONTENT_LENSES,
        lenses: Vec::new(),
        time_controls: default_time_controls(),
        ensemble_card: None,
    };

    let error = template.validate().unwrap_err();
    assert_eq!(error.code(), TEMPLATE_INVALID);
}

#[test]
fn temporal_controls_never_count_toward_a35() {
    for control in default_time_controls() {
        assert!(!control.counts_toward_a35);
    }
}

#[test]
fn templates_without_a37_card_are_not_gate_eligible() {
    let template = SavedPanelTemplate {
        schema_version: OBJECT_VERSION,
        name: "diagnostic".to_string(),
        version: 1,
        notes: String::new(),
        min_content_lenses: MIN_CONTENT_LENSES,
        lenses: Vec::new(),
        time_controls: default_time_controls(),
        ensemble_card: None,
    };

    assert!(!template.a37_gate_eligible());
    assert_eq!(
        template.require_a37_gate().unwrap_err().code(),
        TEMPLATE_A37_GATE_REFUSED
    );
}

#[test]
fn hash_from_bytes_stays_lower_hex() {
    assert_eq!(
        hex_from_bytes(&[0xabu8; 32]),
        "abababababababababababababababababababababababababababababababab"
    );
}

#[test]
fn catalog_lens_ref_uses_manifest_metadata_without_artifact_read() {
    let root = std::env::temp_dir().join(format!("calyx-template-metadata-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let manifest = root.join("manifest.json");
    std::fs::write(
        &manifest,
        serde_json::json!({
            "name": "metadata-only",
            "modality": "text",
            "runtime": "onnx-int8",
            "dim": 384,
            "dtype": "int8",
            "weights_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
            "files": [{
                "role": "model",
                "path": "missing-model.onnx",
                "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                "bytes": 123456789
            }],
            "pooling": "mean",
            "norm": "l2",
            "source_hf_id": "fixture/missing",
            "license": "apache-2.0"
        })
        .to_string(),
    )
    .unwrap();
    let entry = super::super::LensCatalogEntry {
        lens_id: LensId::from_bytes([7_u8; 16]).to_string(),
        name: "metadata-only".to_string(),
        modality: "text".to_string(),
        runtime: "onnx".to_string(),
        dim: 384,
        weights_sha256: "11".repeat(32),
        manifest,
        cost: LensCost::default(),
        placement: Placement::Cpu,
    };

    let reference = lens_ref_from_catalog(&entry).unwrap();

    assert_eq!(reference.runtime, "onnx");
    assert_eq!(reference.shape, SlotShape::Dense(384));
    assert_eq!(reference.modality, Modality::Text);
    assert_eq!(reference.lens_name, "metadata-only");
    std::fs::remove_dir_all(root).unwrap();
}
