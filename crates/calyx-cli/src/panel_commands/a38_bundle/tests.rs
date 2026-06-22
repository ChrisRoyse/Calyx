use std::path::{Path, PathBuf};

use calyx_core::{LensCost, LensId, Modality, Placement, SlotShape};

use super::model::A38_COVERAGE_STATUS;
use super::store::A38BundleStore;
use super::*;
use crate::panel_commands::template_model::{
    A37_ADMISSION_VERSION, CARD_VERSION, CATALOG_VERSION, CapabilityCardRef, PanelTemplateCatalog,
    PanelTemplateIndexEntry, PanelTemplateVersionRef, SavedPanelTemplate, TemplateA37Admission,
    TemplateEnsembleCard, TemplateLensRef, default_time_controls,
};
use crate::panel_commands::{LensCatalog, LensCatalogEntry};

#[test]
fn save_persists_bundle_and_readback_lists_it() {
    let home = temp_home("save-readback");
    write_registry(&home, fixture_entries(0));
    let template_id = write_template(&home, true);
    let evidence = write_evidence(&home);

    save(&save_args(
        &home,
        &evidence,
        &["text", "image", "audio"],
        &[
            "text-0", "text-1", "text-2", "text-3", "text-4", "text-5", "text-6", "text-7",
            "text-8", "text-9", "image-0", "audio-0",
        ],
        None,
    ))
    .unwrap();

    let store = A38BundleStore::open(&home);
    let catalog = store.read_catalog().unwrap();
    let bundle = store.load("constellation-24-general").unwrap();

    assert_eq!(catalog.bundles.len(), 1);
    assert_eq!(bundle.base_template.template_id, template_id);
    assert_eq!(bundle.coverage_status, A38_COVERAGE_STATUS);
    assert!(bundle.under_budget);
    assert_eq!(bundle.content_lens_count, 12);
    assert_eq!(bundle.modality_counts.get("text"), Some(&10));
    assert_eq!(bundle.modality_counts.get("image"), Some(&1));
    assert_eq!(bundle.modality_counts.get("audio"), Some(&1));
    assert_eq!(bundle.evidence_refs.len(), 1);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn missing_required_modality_fails_closed_without_index() {
    let home = temp_home("missing-modality");
    write_registry(&home, fixture_entries(0));
    write_template(&home, true);
    let evidence = write_evidence(&home);

    let error = save(&save_args(
        &home,
        &evidence,
        &["text", "image"],
        &[
            "text-0", "text-1", "text-2", "text-3", "text-4", "text-5", "text-6", "text-7",
            "text-8", "text-9",
        ],
        None,
    ))
    .unwrap_err();

    assert_eq!(error.code(), A38_BUNDLE_INCOMPLETE);
    assert!(
        !home
            .join("panels")
            .join("a38-bundles")
            .join("index.json")
            .exists()
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn over_budget_bundle_fails_closed_without_index() {
    let home = temp_home("over-budget");
    write_registry(&home, fixture_entries(2 * 1024 * 1024));
    write_template(&home, true);
    let evidence = write_evidence(&home);

    let error = save(&save_args(
        &home,
        &evidence,
        &["text", "image"],
        &[
            "text-0", "text-1", "text-2", "text-3", "text-4", "text-5", "text-6", "text-7",
            "text-8", "text-9", "image-0",
        ],
        Some(1),
    ))
    .unwrap_err();

    assert_eq!(error.code(), A38_BUNDLE_BUDGET_EXCEEDED);
    assert!(
        !home
            .join("panels")
            .join("a38-bundles")
            .join("index.json")
            .exists()
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn base_template_without_a37_gate_fails_closed_without_index() {
    let home = temp_home("base-refused");
    write_registry(&home, fixture_entries(0));
    write_template(&home, false);
    let evidence = write_evidence(&home);

    let error = save(&save_args(
        &home,
        &evidence,
        &["text"],
        &[
            "text-0", "text-1", "text-2", "text-3", "text-4", "text-5", "text-6", "text-7",
            "text-8", "text-9",
        ],
        None,
    ))
    .unwrap_err();

    assert_eq!(error.code(), A38_BUNDLE_BASE_A37_REFUSED);
    assert!(
        !home
            .join("panels")
            .join("a38-bundles")
            .join("index.json")
            .exists()
    );
    std::fs::remove_dir_all(home).unwrap();
}

fn temp_home(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-a38-bundle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write_registry(home: &Path, mut entries: Vec<LensCatalogEntry>) {
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let path = home.join("lenses").join("registry.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&LensCatalog { lenses: entries }).unwrap(),
    )
    .unwrap();
}

fn write_evidence(home: &Path) -> PathBuf {
    let path = home.join("fsv").join("evidence.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::json!({"status":"gate_passed","bytes":"readback"}).to_string(),
    )
    .unwrap();
    path
}

fn fixture_entries(image_vram_bytes: u64) -> Vec<LensCatalogEntry> {
    let mut entries = (0_u8..10)
        .map(|idx| lens_entry(idx, &format!("text-{idx}"), "text", 0))
        .collect::<Vec<_>>();
    entries.push(lens_entry(20, "image-0", "image", image_vram_bytes));
    entries.push(lens_entry(21, "audio-0", "audio", 0));
    entries
}

fn lens_entry(idx: u8, name: &str, modality: &str, vram_bytes: u64) -> LensCatalogEntry {
    LensCatalogEntry {
        lens_id: LensId::from_bytes([idx; 16]).to_string(),
        name: name.to_string(),
        modality: modality.to_string(),
        runtime: "fixture".to_string(),
        dim: 8,
        weights_sha256: "11".repeat(32),
        manifest: PathBuf::from(format!("/tmp/{name}.json")),
        cost: LensCost {
            vram_bytes,
            batch_ceiling: 1,
            ..LensCost::default()
        },
        placement: Placement::Gpu,
    }
}

fn write_template(home: &Path, gate_passed: bool) -> String {
    let template = SavedPanelTemplate {
        schema_version: crate::panel_commands::template_model::OBJECT_VERSION,
        name: "constellation-24".to_string(),
        version: 1,
        notes: "fixture base".to_string(),
        min_content_lenses: 10,
        lenses: (0_u8..10).map(template_lens).collect(),
        time_controls: default_time_controls(),
        ensemble_card: Some(template_card(gate_passed)),
    };
    template.validate().unwrap();
    let bytes = serde_json::to_vec_pretty(&template).unwrap();
    let id = blake3::hash(&bytes).to_hex().to_string();
    let object_path = home
        .join("panels")
        .join("templates")
        .join("objects")
        .join(format!("{id}.json"));
    std::fs::create_dir_all(object_path.parent().unwrap()).unwrap();
    std::fs::write(&object_path, &bytes).unwrap();
    let index = PanelTemplateCatalog {
        schema_version: CATALOG_VERSION,
        templates: vec![PanelTemplateIndexEntry {
            name: "constellation-24".to_string(),
            active_template_id: id.clone(),
            versions: vec![PanelTemplateVersionRef {
                version: 1,
                template_id: id.clone(),
                object_path: format!("objects/{id}.json"),
                blake3_hex: id.clone(),
                size_bytes: bytes.len() as u64,
                saved_at_ms: 1,
            }],
        }],
    };
    let index_path = home.join("panels").join("templates").join("index.json");
    std::fs::write(index_path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();
    id
}

fn template_lens(idx: u8) -> TemplateLensRef {
    TemplateLensRef {
        slot_key: format!("text_{idx}"),
        lens_name: format!("text-{idx}"),
        lens_id: LensId::from_bytes([idx; 16]),
        runtime_lens_id: None,
        weights_sha256: "11".repeat(32),
        runtime: "fixture".to_string(),
        modality: Modality::Text,
        shape: SlotShape::Dense(8),
        placement: Placement::Gpu,
        cost: LensCost::default(),
        manifest: format!("/tmp/text-{idx}.json"),
        counts_toward_a35: true,
    }
}

fn template_card(gate_passed: bool) -> TemplateEnsembleCard {
    TemplateEnsembleCard {
        schema_version: CARD_VERSION,
        source: "fixture".to_string(),
        content_lens_count: 10,
        measured_lens_count: 10,
        all_loaded: true,
        min_coverage_rate: 1.0,
        total_vram_bytes: 0,
        total_ram_bytes: 0,
        mean_ms_per_input: 0.0,
        card_refs: Vec::<CapabilityCardRef>::new(),
        a37_admission: TemplateA37Admission {
            schema_version: A37_ADMISSION_VERSION,
            source: "fixture".to_string(),
            gate_eligible: gate_passed,
            status: if gate_passed {
                "gate_passed".to_string()
            } else {
                "gate_failed".to_string()
            },
            verdict: "fixture".to_string(),
            content_lens_count: 10,
            temporal_sidecar_count: 3,
            temporal_counts_toward_content_floor: false,
            association_family_count: 4,
            n_eff: Some(10.0),
            mean_pairwise_corr: Some(0.1),
            mean_pairwise_nmi: Some(0.1),
            sum_unique_pid_bits: Some(1.0),
        },
        a37_ensemble_card_ref: None,
        a37_admission_card_ref: None,
    }
}

fn save_args(
    home: &Path,
    evidence: &Path,
    required: &[&str],
    lenses: &[&str],
    budget_vram_mib: Option<u64>,
) -> Vec<String> {
    let mut args = vec![
        "--home".to_string(),
        home.display().to_string(),
        "--name".to_string(),
        "constellation-24-general".to_string(),
        "--base-template".to_string(),
        "constellation-24".to_string(),
        "--evidence".to_string(),
        evidence.display().to_string(),
    ];
    for modality in required {
        args.push("--required-modality".to_string());
        args.push((*modality).to_string());
    }
    for lens in lenses {
        args.push("--include-lens".to_string());
        args.push((*lens).to_string());
    }
    if let Some(budget) = budget_vram_mib {
        args.push("--budget-vram-mib".to_string());
        args.push(budget.to_string());
    }
    args
}
