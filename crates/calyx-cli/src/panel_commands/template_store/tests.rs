use super::*;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{LensCost, LensId, Modality, Placement, SlotShape};
use calyx_registry::{
    LensForgeManifest, derive_runtime_contract_from_spec, lens_spec_from_manifest_path,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::lens_commands::support::runtime_name;
use crate::panel_commands::template_model::{LENS_SNAPSHOT_VERSION, TemplateLensSnapshot};

#[test]
fn template_registration_canonicalizes_runtime_contracts() {
    let root = temp_root("canonical-runtime");
    fs::create_dir_all(&root).unwrap();
    let (endpoint, server) = tei_registration_server(MIN_CONTENT_LENSES * 2);
    let mut template = saved_template(
        "tei-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, None, Some(&endpoint)))
            .collect(),
    );
    let mut registry = Registry::new();

    let added = register_template_lenses(&mut registry, &mut template).unwrap();
    server.join().unwrap();

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
fn resident_registration_canonicalizes_verified_specs_to_frozen_contracts() {
    let root = temp_root("resident-canonical-runtime");
    fs::create_dir_all(&root).unwrap();
    let template = saved_template(
        "resident-tei-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, None, None))
            .collect(),
    );
    let template_id = id_for_loaded(&template).unwrap();

    let expected = resident_registration::expected_slots(&template, &template_id).unwrap();

    assert_eq!(expected.len(), MIN_CONTENT_LENSES);
    for (lens, slot) in template.lenses.iter().zip(expected) {
        let raw_spec = lens.verified_materialization_spec(&template_id).unwrap();
        assert_ne!(raw_spec.declared_contract(), slot.contract);
        assert_eq!(slot.spec.declared_contract(), slot.contract);
        assert_ne!(slot.spec.lens_id(), raw_spec.lens_id());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_runtime_lens_id_fails_before_registry_mutation() {
    let root = temp_root("stale-runtime");
    fs::create_dir_all(&root).unwrap();
    let stale = LensId::from_bytes([0x55; 16]);
    let (endpoint, server) = tei_registration_server((MIN_CONTENT_LENSES - 1) * 2);
    let mut template = saved_template(
        "tei-template-stale",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| {
                tei_lens_ref(
                    &root,
                    idx,
                    (idx + 1 == MIN_CONTENT_LENSES).then_some(stale),
                    Some(&endpoint),
                )
            })
            .collect(),
    );
    let mut registry = Registry::new();

    let error = register_template_lenses(&mut registry, &mut template).unwrap_err();
    server.join().unwrap();

    assert_eq!(error.code(), TEMPLATE_INVALID);
    assert!(error.message().contains("expected"));
    assert_eq!(registry.lens_snapshots().len(), 0);
    assert!(!registry.contains(stale));
    assert!(
        template
            .lenses
            .iter()
            .take(MIN_CONTENT_LENSES - 1)
            .all(|lens| lens.runtime_lens_id.is_none())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_object_is_rejected_normally_but_readable_only_for_explicit_refresh() {
    let home = temp_root("legacy-refresh");
    let manifests = home.join("commissioned");
    fs::create_dir_all(&manifests).unwrap();
    let mut legacy = saved_template(
        "legacy-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&manifests, idx, None, None))
            .collect(),
    );
    let mut current = legacy.clone();
    current.name = "current-template".to_string();
    current.version = 2;
    legacy.schema_version = 1;
    for lens in &mut legacy.lenses {
        lens.immutable_snapshot = None;
    }
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    let template_id = blake3::hash(&bytes).to_hex().to_string();
    let store_root = home.join("panels").join("templates");
    let object_rel = format!("objects/{template_id}.json");
    let object_path = store_root.join(&object_rel);
    fs::create_dir_all(object_path.parent().unwrap()).unwrap();
    fs::write(&object_path, &bytes).unwrap();
    let current_bytes = serde_json::to_vec_pretty(&current).unwrap();
    let current_id = blake3::hash(&current_bytes).to_hex().to_string();
    let current_rel = format!("objects/{current_id}.json");
    fs::write(store_root.join(&current_rel), &current_bytes).unwrap();
    let catalog = PanelTemplateCatalog {
        schema_version: CATALOG_VERSION,
        templates: vec![
            PanelTemplateIndexEntry {
                name: current.name.clone(),
                active_template_id: current_id.clone(),
                versions: vec![PanelTemplateVersionRef {
                    version: current.version,
                    template_id: current_id.clone(),
                    object_path: current_rel,
                    blake3_hex: current_id.clone(),
                    size_bytes: current_bytes.len() as u64,
                    saved_at_ms: 2,
                }],
            },
            PanelTemplateIndexEntry {
                name: legacy.name.clone(),
                active_template_id: template_id.clone(),
                versions: vec![PanelTemplateVersionRef {
                    version: 1,
                    template_id: template_id.clone(),
                    object_path: object_rel,
                    blake3_hex: template_id.clone(),
                    size_bytes: bytes.len() as u64,
                    saved_at_ms: 1,
                }],
            },
        ],
    };
    fs::write(
        store_root.join("index.json"),
        serde_json::to_vec_pretty(&catalog).unwrap(),
    )
    .unwrap();
    let store = TemplateStore::open(&home);

    let ordinary_error = store.load("legacy-template").unwrap_err();
    let refresh_source = store.load_for_refresh("legacy-template").unwrap();
    let current_readback = store.load("current-template").unwrap();
    let summaries = store.list().unwrap();

    assert_eq!(ordinary_error.code(), TEMPLATE_INVALID);
    assert!(ordinary_error.message().contains(&template_id));
    assert!(ordinary_error.remediation().contains("template refresh"));
    assert_eq!(refresh_source.schema_version, 1);
    assert_eq!(refresh_source.name, "legacy-template");
    assert_eq!(current_readback.schema_version, OBJECT_VERSION);
    assert_eq!(summaries.len(), 2);
    let legacy_summary = summaries
        .iter()
        .find(|summary| summary.name == "legacy-template")
        .unwrap();
    assert_eq!(legacy_summary.object_schema_version, 1);
    assert!(legacy_summary.migration_required);
    assert!(
        legacy_summary
            .refresh_command
            .as_deref()
            .unwrap()
            .contains(&template_id)
    );
    let current_summary = summaries
        .iter()
        .find(|summary| summary.name == "current-template")
        .unwrap();
    assert_eq!(current_summary.object_schema_version, OBJECT_VERSION);
    assert!(!current_summary.migration_required);
    assert!(current_summary.refresh_command.is_none());
    assert!(
        refresh_source
            .lenses
            .iter()
            .all(|lens| lens.immutable_snapshot.is_none())
    );
    let current_path = store_root.join(format!("objects/{current_id}.json"));
    let mut corrupt = current_bytes;
    corrupt.push(b'\n');
    fs::write(current_path, corrupt).unwrap();
    let corrupt_error = store.list().unwrap_err();
    assert_eq!(corrupt_error.code(), TEMPLATE_INVALID);
    assert!(corrupt_error.message().contains("hash mismatch"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn concurrent_template_saves_preserve_both_catalog_updates() {
    let home = temp_root("concurrent-saves");
    let manifests = home.join("commissioned");
    fs::create_dir_all(&manifests).unwrap();
    let lenses = (0..MIN_CONTENT_LENSES)
        .map(|idx| {
            let mut lens = tei_lens_ref(&manifests, idx, None, None);
            lens.placement = Placement::Gpu;
            lens
        })
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(3));
    let writers = ["concurrent-a", "concurrent-b"]
        .into_iter()
        .map(|name| {
            let store = TemplateStore::open(&home);
            let barrier = barrier.clone();
            let lenses = lenses.clone();
            thread::spawn(move || {
                barrier.wait();
                store.save(
                    TemplateDraft {
                        name: name.to_string(),
                        notes: "concurrent durable publication test".to_string(),
                        lenses,
                        ensemble_card: None,
                    },
                    42,
                )
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for writer in writers {
        writer.join().unwrap().unwrap();
    }

    let catalog = TemplateStore::open(&home).read_catalog().unwrap();
    let names = catalog
        .templates
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["concurrent-a", "concurrent-b"]);
    for entry in &catalog.templates {
        let version = &entry.versions[0];
        let bytes = fs::read(home.join("panels/templates").join(&version.object_path)).unwrap();
        assert_eq!(blake3::hash(&bytes).to_hex().as_str(), version.blake3_hex);
    }
    let leaked = fs::read_dir(home.join("panels/templates"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!leaked);
    fs::remove_dir_all(home).unwrap();
}

/// #1947: the target panel keeps temporal time controls Active and marks them as
/// compute-on-recall sidecars, which is what exempts them from the "active slot
/// => frozen registry entry" rule at swap and at ingest preflight.
#[test]
fn target_panel_materializes_temporal_sidecars_as_active_compute_on_recall() {
    let root = temp_root("target-panel-temporal-sidecar");
    fs::create_dir_all(&root).unwrap();
    let template = saved_template(
        "temporal-sidecar-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, None, None))
            .collect(),
    );

    let panel = template.to_target_panel(42);

    assert_eq!(panel.slots.len(), MIN_CONTENT_LENSES + 3);
    let (content, temporal) = panel.slots.split_at(MIN_CONTENT_LENSES);
    for slot in content {
        assert_eq!(slot.state, calyx_core::SlotState::Active);
        assert!(
            !slot.is_compute_on_recall_sidecar(),
            "content slots must stay subject to the registry requirement"
        );
    }
    assert_eq!(
        temporal
            .iter()
            .map(|slot| slot.slot_key.key().to_string())
            .collect::<Vec<_>>(),
        vec!["E2_recency", "E3_periodic", "E4_positional"],
    );
    for slot in temporal {
        assert_eq!(slot.state, calyx_core::SlotState::Active);
        assert!(slot.is_compute_on_recall_sidecar());
        assert_eq!(slot.modality, Modality::Structured);
    }
    fs::remove_dir_all(root).unwrap();
}

/// #1947: swap must refuse to publish an active *content* slot with no frozen
/// registry entry, rather than deferring the failure to ingest preflight — while
/// never naming the exempt compute-on-recall sidecars.
#[test]
fn swap_guard_refuses_active_slot_without_frozen_registry_entry() {
    let root = temp_root("swap-guard-unbacked-active");
    fs::create_dir_all(&root).unwrap();
    let template = saved_template(
        "unbacked-active-template",
        (0..MIN_CONTENT_LENSES)
            .map(|idx| tei_lens_ref(&root, idx, None, None))
            .collect(),
    );
    let panel = template.to_target_panel(42);
    let empty = Registry::new();

    let error = super::store::ensure_active_slots_are_registered(&template.name, &panel, &empty)
        .unwrap_err();

    let rendered = format!("{error:?}");
    assert!(
        rendered.contains(calyx_registry::CALYX_PANEL_LENS_MISSING),
        "expected CALYX_PANEL_LENS_MISSING, got {rendered}"
    );
    // The parked temporal sidecars must not be named by the refusal.
    assert!(!rendered.contains("E2_recency"), "got {rendered}");
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

fn tei_lens_ref(
    root: &Path,
    idx: usize,
    runtime_lens_id: Option<LensId>,
    endpoint: Option<&str>,
) -> TemplateLensRef {
    let name = format!("fixture-tei-{idx}");
    let endpoint = endpoint
        .map(str::to_string)
        .unwrap_or_else(|| format!("http://127.0.0.1:{}/embed", 18_000 + idx));
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
    let manifest: LensForgeManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let runtime_contract = derive_runtime_contract_from_spec(&spec).unwrap();
    let immutable_snapshot = TemplateLensSnapshot {
        derivation_version: Some(calyx_registry::derivation::LENS_DERIVATION_VERSION),
        schema_version: LENS_SNAPSHOT_VERSION,
        manifest_blake3: fixture_blake3(&manifest),
        spec_blake3: fixture_blake3(&spec),
        runtime_contract_blake3: fixture_blake3(&runtime_contract),
        manifest,
        manifest_base_dir: root.to_path_buf(),
        spec: spec.clone(),
        runtime_contract,
    };
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
        immutable_snapshot: Some(immutable_snapshot),
        counts_toward_a35: true,
    }
}

fn tei_registration_server(request_count: usize) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/embed", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let body = serde_json::to_vec(&vec![vec![0.353_553_38_f32; 8]]).unwrap();
        for _ in 0..request_count {
            let (mut stream, _) = listener.accept().unwrap();
            read_http_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (endpoint, server)
}

fn read_http_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "TEI fixture request ended before its body");
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_len = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap();
        if request.len() >= header_end + 4 + content_len {
            return;
        }
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

fn fixture_blake3<T: serde::Serialize>(value: &T) -> String {
    blake3::hash(&serde_json::to_vec(value).unwrap())
        .to_hex()
        .to_string()
}
