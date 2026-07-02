use super::*;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{LensCost, LensId, Modality, Placement, SlotShape};
use calyx_registry::lens_spec_from_manifest_path;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::lens_commands::support::runtime_name;

#[test]
fn template_registration_canonicalizes_runtime_contracts() {
    let root = temp_root("canonical-runtime");
    fs::create_dir_all(&root).unwrap();
    let mut template = saved_template(
        "tei-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, None))
            .collect(),
    );
    let mut registry = Registry::new();

    let added = register_template_lenses(&mut registry, &mut template).unwrap();

    assert_eq!(added, MIN_CONTENT_LENSES);
    for lens in &template.lenses {
        let runtime_lens_id = lens.runtime_lens_id.unwrap();
        assert_ne!(runtime_lens_id, lens.lens_id);
        assert!(registry.contains(runtime_lens_id));
        assert_eq!(
            registry
                .lens_spec(runtime_lens_id)
                .unwrap()
                .declared_contract()
                .lens_id(),
            runtime_lens_id
        );
    }
    let panel = template.to_target_panel(42);
    for (slot, lens) in panel.slots.iter().zip(template.lenses.iter()) {
        assert_eq!(slot.lens_id, lens.runtime_lens_id.unwrap());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_runtime_lens_id_fails_before_registry_mutation() {
    let root = temp_root("stale-runtime");
    fs::create_dir_all(&root).unwrap();
    let stale = LensId::from_bytes([0x55; 16]);
    let mut template = saved_template(
        "tei-template-stale",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, (idx == 0).then_some(stale)))
            .collect(),
    );
    let mut registry = Registry::new();

    let error = register_template_lenses(&mut registry, &mut template).unwrap_err();

    assert_eq!(error.code(), TEMPLATE_INVALID);
    assert!(error.message().contains("expected"));
    assert_eq!(registry.lens_snapshots().len(), 0);
    assert!(!registry.contains(stale));
    fs::remove_dir_all(root).unwrap();
}

fn saved_template(name: &str, lenses: Vec<TemplateLensRef>) -> SavedPanelTemplate {
    SavedPanelTemplate {
        schema_version: OBJECT_VERSION,
        name: name.to_string(),
        version: 1,
        notes: String::new(),
        min_content_lenses: MIN_CONTENT_LENSES,
        lenses,
        time_controls: default_time_controls(),
        ensemble_card: None,
    }
}

fn tei_lens_ref(root: &Path, idx: usize, runtime_lens_id: Option<LensId>) -> TemplateLensRef {
    let name = format!("fixture-tei-{idx}");
    let endpoint = format!("http://127.0.0.1:{}/embed", 18_000 + idx);
    let descriptor_name = format!("tei-descriptor-{idx}.json");
    let descriptor_bytes =
        format!(r#"{{"source_hf_id":"fixture/tei-{idx}","endpoint":"{endpoint}","dim":8}}"#)
            .into_bytes();
    let descriptor_digest = sha256_hex(&descriptor_bytes);
    fs::write(root.join(&descriptor_name), &descriptor_bytes).unwrap();
    let manifest_path = root.join(format!("manifest-{idx}.json"));
    fs::write(
        &manifest_path,
        json!({
            "name": name,
            "modality": "text",
            "runtime": "tei",
            "dim": 8,
            "dtype": "f32",
            "weights_sha256": descriptor_digest,
            "files": [{
                "role": "model",
                "path": descriptor_name,
                "sha256": descriptor_digest,
                "bytes": descriptor_bytes.len()
            }],
            "pooling": "mean",
            "norm": "unit",
            "source_hf_id": format!("fixture/tei-{idx}"),
            "endpoint": endpoint,
            "license": "apache-2.0"
        })
        .to_string(),
    )
    .unwrap();
    let spec = lens_spec_from_manifest_path(&manifest_path).unwrap();
    TemplateLensRef {
        slot_key: format!("fixture_tei_{idx}"),
        lens_name: spec.name.clone(),
        lens_id: spec.lens_id(),
        runtime_lens_id,
        weights_sha256: hex32(&spec.weights_sha256),
        runtime: runtime_name(&spec.runtime).to_string(),
        modality: Modality::Text,
        shape: SlotShape::Dense(8),
        placement: Placement::Cpu,
        cost: LensCost::default(),
        manifest: manifest_path.display().to_string(),
        counts_toward_a35: true,
    }
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-template-store-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex32(&Sha256::digest(bytes).into())
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
