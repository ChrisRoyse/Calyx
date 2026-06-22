use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Input, Lens, LensId, Modality, Panel, QuantPolicy, Result as CalyxResult, SlotShape,
    SlotVector, VaultId,
};
use calyx_registry::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use calyx_registry::{
    CALYX_PANEL_LENS_MISSING, LensRuntime, LensSpec, PanelLensRuntime, PanelTemplate, Registry,
    apply_panel_template, bio_default, legal_default, list_panel, load_vault_panel_state,
    media_default, medical_default, persist_vault_panel_state,
};
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue789_templates_are_deterministic_and_domain_complete() {
    for template in templates() {
        let first = instantiate_ids(&template, 1);
        let second = instantiate_ids(&template, 999);
        assert_eq!(first, second);
        assert!(slot_names(&template).ends_with(&[
            "E2_recency".to_string(),
            "E3_periodic".to_string(),
            "E4_positional".to_string(),
        ]));
    }

    assert!(slot_names(&legal_default()).contains(&"legal_bert_small".to_string()));
    assert!(slot_names(&legal_default()).contains(&"causal_dual".to_string()));
    assert!(slot_names(&medical_default()).contains(&"biomedbert_small_embeddings".to_string()));
    assert!(slot_names(&bio_default()).contains(&"protein_esm2".to_string()));
    assert!(slot_names(&bio_default()).contains(&"dna_dnabert2".to_string()));
    assert!(slot_names(&bio_default()).contains(&"molecule_chemberta".to_string()));
    assert!(slot_names(&media_default()).contains(&"image_siglip2".to_string()));
    assert!(slot_names(&media_default()).contains(&"audio_clap".to_string()));
    assert!(slot_names(&media_default()).contains(&"speaker_wavlm".to_string()));
    assert!(slot_names(&media_default()).contains(&"style_register".to_string()));
}

#[test]
fn issue789_apply_template_resolves_lenses_and_missing_is_atomic() {
    let templates = templates();
    let registry = registry_for(&templates);
    let template = legal_default();
    let mut panel = empty_panel();

    let applied = apply_panel_template(&mut panel, &registry, &template, 789).unwrap();

    assert_eq!(applied.diff.added.len(), template.slots.len());
    assert_eq!(panel.slots.len(), template.slots.len());
    assert_eq!(
        applied.resolved_lenses.len(),
        registry_slot_count(&template)
    );
    assert_eq!(list_panel(&panel, &registry).len(), template.slots.len());

    let mut missing_panel = empty_panel();
    let before = missing_panel.clone();
    let missing = apply_panel_template(&mut missing_panel, &Registry::new(), &media_default(), 790)
        .unwrap_err();

    assert_eq!(missing.code, CALYX_PANEL_LENS_MISSING);
    assert_eq!(missing_panel, before);
}

#[test]
fn issue789_default_domain_panels_fsv_readbacks() {
    let (root, keep_root) = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create issue789 fsv root");

    let templates = templates();
    let registry = registry_for(&templates);
    let mut status_files = Vec::new();
    for template in &templates {
        let vault_dir = root.join(format!("vault-{}", template.name));
        create_panel_vault(&vault_dir);
        let mut panel = empty_panel();
        let applied = apply_panel_template(&mut panel, &registry, template, 789).unwrap();
        let write = persist_vault_panel_state(&vault_dir, &panel, &registry).unwrap();
        let loaded = load_vault_panel_state(&vault_dir).unwrap();
        let listing = list_panel(&loaded.panel, &loaded.registry);
        let names = loaded
            .panel
            .slots
            .iter()
            .map(|slot| slot.slot_key.key().to_string())
            .collect::<Vec<_>>();

        let status = json!({
            "template": template.name,
            "vault": vault_dir,
            "panel_version": loaded.panel.version,
            "slot_count": loaded.panel.slots.len(),
            "registry_lens_count": loaded.registry_snapshot.as_ref().map(|s| s.lenses.len()).unwrap_or(0),
            "resolved_lenses": applied.resolved_lenses,
            "diff": applied.diff,
            "panel_ref": write.panel_ref.logical_path,
            "registry_ref": write.registry_ref.logical_path,
            "slot_names": names,
            "listing": listing,
        });
        let path = root.join(format!("panel-status-{}.json", template.name));
        fs::write(&path, serde_json::to_vec_pretty(&status).unwrap()).unwrap();
        status_files.push(path);

        assert_eq!(loaded.panel.slots.len(), template.slots.len());
        assert!(names.ends_with(&[
            "E2_recency".to_string(),
            "E3_periodic".to_string(),
            "E4_positional".to_string(),
        ]));
    }

    let edge_path = missing_lens_edge(&root);
    let summary = json!({
        "source_of_truth": "fresh manual verification vault CURRENT plus immutable panel/registry assets",
        "templates": templates.iter().map(|template| template.name.clone()).collect::<Vec<_>>(),
        "status_files": status_files,
        "edge_file": edge_path,
    });
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    println!("ISSUE789_FSV_ROOT={}", root.display());
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());

    if !keep_root {
        fs::remove_dir_all(&root).expect("cleanup issue789 fsv root");
    }
}

fn missing_lens_edge(root: &Path) -> PathBuf {
    let mut panel = empty_panel();
    let before = json!({
        "panel_version": panel.version,
        "slot_count": panel.slots.len(),
    });
    let error =
        apply_panel_template(&mut panel, &Registry::new(), &medical_default(), 790).unwrap_err();
    let after = json!({
        "panel_version": panel.version,
        "slot_count": panel.slots.len(),
    });
    let edge = json!({
        "error_code": error.code,
        "before": before,
        "after": after,
        "panel_unchanged": before == after,
    });
    let path = root.join("edge-missing-lens.json");
    fs::write(&path, serde_json::to_vec_pretty(&edge).unwrap()).unwrap();
    assert_eq!(error.code, CALYX_PANEL_LENS_MISSING);
    assert_eq!(before, after);
    path
}

fn templates() -> Vec<PanelTemplate> {
    vec![
        legal_default(),
        medical_default(),
        bio_default(),
        media_default(),
    ]
}

fn instantiate_ids(template: &PanelTemplate, created_at: u64) -> Vec<LensId> {
    calyx_registry::instantiate_panel(template, created_at)
        .panel
        .slots
        .into_iter()
        .map(|slot| slot.lens_id)
        .collect()
}

fn slot_names(template: &PanelTemplate) -> Vec<String> {
    template
        .slots
        .iter()
        .map(|slot| slot.name.clone())
        .collect()
}

fn registry_slot_count(template: &PanelTemplate) -> usize {
    template
        .slots
        .iter()
        .filter(|slot| matches!(slot.runtime, PanelLensRuntime::Registry { .. }))
        .count()
}

fn registry_for(templates: &[PanelTemplate]) -> Registry {
    let mut registry = Registry::new();
    let mut seen = BTreeMap::new();
    for template in templates {
        for slot in &template.slots {
            let PanelLensRuntime::Registry { name } = &slot.runtime else {
                continue;
            };
            let prior = seen.insert(name.clone(), (slot.output, slot.modality));
            if let Some((shape, modality)) = prior {
                assert_eq!((shape, modality), (slot.output, slot.modality));
                continue;
            }
            register_dummy_lens(&mut registry, name, slot.output, slot.modality);
        }
    }
    registry
}

fn register_dummy_lens(registry: &mut Registry, name: &str, output: SlotShape, modality: Modality) {
    let contract = FrozenLensContract::new(
        name,
        sha256_digest(&[b"issue789-panel-lens", name.as_bytes(), b"weights"]),
        sha256_digest(&[b"issue789-panel-lens", name.as_bytes(), b"corpus"]),
        output,
        modality,
        LensDType::F32,
        NormPolicy::Finite,
    );
    let lens = DummyLens {
        id: contract.lens_id(),
        output,
        modality,
    };
    let spec = LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "issue789-panel-template".to_string(),
        },
        output,
        modality,
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some("issue789".to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
}

#[derive(Clone)]
struct DummyLens {
    id: LensId,
    output: SlotShape,
    modality: Modality,
}

impl Lens for DummyLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        self.output
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, _input: &Input) -> CalyxResult<SlotVector> {
        Ok(match self.output {
            SlotShape::Dense(dim) => SlotVector::Dense {
                dim,
                data: vec![0.0; dim as usize],
            },
            SlotShape::Sparse(dim) => SlotVector::Sparse {
                dim,
                entries: Vec::new(),
            },
            SlotShape::Multi { token_dim } => SlotVector::Multi {
                token_dim,
                tokens: Vec::new(),
            },
        })
    }
}

fn create_panel_vault(vault_dir: &Path) {
    let _ = fs::remove_dir_all(vault_dir);
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue789-default-panels".to_vec(),
        VaultOptions {
            panel: Some(empty_panel()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
}

fn empty_panel() -> Panel {
    Panel {
        version: 1,
        slots: Vec::new(),
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    (
        std::env::temp_dir().join(format!("calyx-registry-issue789-{id}")),
        false,
    )
}
