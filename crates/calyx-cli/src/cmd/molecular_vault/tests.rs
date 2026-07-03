use std::fs;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
    VaultId,
};
use calyx_registry::runtime::algorithmic::AlgorithmicLens;
use calyx_registry::{
    LensRuntime, LensSpec, Registry, persist_vault_panel_state, spec::default_recall_delta,
};
use serde_json::json;

use super::*;

#[test]
fn parser_requires_rows() {
    let err = parse_materialize_molecular_vault(&["demo".to_string()]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn parser_accepts_home() {
    let parsed = parse_materialize_molecular_vault(&[
        "vault".to_string(),
        "--rows".to_string(),
        "rows.jsonl".to_string(),
        "--home".to_string(),
        "target/home".to_string(),
    ])
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::MaterializeMolecularVault(MaterializeMolecularVaultArgs {
            vault: "vault".to_string(),
            rows: "rows.jsonl".into(),
            home: Some("target/home".into()),
        })
    );
}

#[test]
fn rejects_missing_required_modalities() {
    let root = temp_root("missing-modalities");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    fs::write(
        &rows,
        r#"{"id":"clinical","domain":"clinical","modality":"text","text":"metformin clinical","bridge_terms":["metformin"],"metadata":{"source_dataset":"pubmedqa","source_path":"/s","source_sha256":"aaa"}}"#,
    )
    .unwrap();
    let err = read_rows(&rows)
        .and_then(|rows| validate_row_set(&rows))
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("protein row"));
}

#[test]
fn materializes_rows_slots_anchors_and_graph_readback() {
    let home = temp_root("happy");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id: VaultId = "01KWM8840TEST0000000000000".parse().unwrap();
    let name = "issue884-molecular-test";
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    let (panel, registry) = panel_and_registry();
    AsterVault::new_durable(
        &vault_dir,
        vault_id,
        super::vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault_dir, &panel, &registry).unwrap();
    fs::write(
        home.join("vaults").join("index.json"),
        serde_json::to_vec_pretty(&json!({
            "vaults": [{
                "name": name,
                "vault_id": vault_id,
                "path": format!("vaults/{vault_id}"),
                "panel_template": "issue884-molecular-test"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let rows = home.join("rows.jsonl");
    fs::write(
        &rows,
        concat!(
            r#"{"id":"clinical-1","domain":"clinical","modality":"text","text":"metformin clinical diabetes bridge","bridge_terms":["metformin"],"metadata":{"source_dataset":"pubmedqa","source_path":"/source/pubmedqa.jsonl","source_sha256":"aaa"}}"#,
            "\n",
            r#"{"id":"molecule-1","domain":"molecular","modality":"molecule","input":"CN(C)C(=N)N=C(N)N","text":"metformin molecule binding affinity diabetes bridge","bridge_terms":["metformin"],"binding_affinity_nm":8.0,"metadata":{"source_dataset":"bindingdb","source_path":"/source/bindingdb.tsv","source_sha256":"bbb"}}"#,
            "\n",
            r#"{"id":"protein-1","domain":"molecular","modality":"protein","input":"MTEYKLVVVG","text":"metformin protein target bridge","bridge_terms":["metformin"],"metadata":{"source_dataset":"uniprot","source_path":"/source/uniprot.fasta","source_sha256":"ccc"}}"#,
            "\n",
            r#"{"id":"dna-1","domain":"molecular","modality":"dna","input":"ACGTACGTNN","text":"metformin dna regulatory bridge","bridge_terms":["metformin"],"metadata":{"source_dataset":"opentargets","source_path":"/source/target.parquet","source_sha256":"ddd"}}"#,
            "\n",
        ),
    )
    .unwrap();
    let report = super::materialize(
        &home,
        MaterializeMolecularVaultArgs {
            vault: name.to_string(),
            rows,
            home: None,
        },
    )
    .unwrap();
    assert_eq!(report.row_count, 4);
    assert_eq!(report.affinity_row_count, 1);
    assert_eq!(report.bridge_term_count, 1);
    assert_eq!(report.readback.base_rows, 4);
    assert_eq!(report.readback.graph_nodes, 5);
    assert_eq!(report.readback.graph_edges, 8);
    assert!(report.readback.anchor_rows >= 13);
    for slot in ["text", "protein", "dna", "molecule"] {
        assert_eq!(report.readback.measured_slot_rows.get(slot), Some(&1));
    }
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-molecular-vault-{name}-{}",
        std::process::id()
    ))
}

fn panel_and_registry() -> (Panel, Registry) {
    let mut registry = Registry::new();
    let mut slots = Vec::new();
    for (index, (name, modality)) in [
        ("text", Modality::Text),
        ("protein", Modality::Protein),
        ("dna", Modality::Dna),
        ("molecule", Modality::Molecule),
    ]
    .into_iter()
    .enumerate()
    {
        let lens_id = register_dummy_lens(&mut registry, name, modality);
        let slot_id = SlotId::new(index as u16);
        slots.push(Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, name.to_string()),
            lens_id,
            shape: SlotShape::Dense(16),
            modality,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some(name.to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: Default::default(),
            state: SlotState::Active,
            added_at_panel_version: (index + 1) as u32,
        });
    }
    (
        Panel {
            version: slots.len() as u32,
            slots,
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        },
        registry,
    )
}

fn register_dummy_lens(registry: &mut Registry, name: &str, modality: Modality) -> LensId {
    let lens = AlgorithmicLens::byte_features(name, modality);
    let contract = lens.contract().clone();
    let id = contract.lens_id();
    let output = contract.shape();
    let spec = LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output,
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(name.to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::None,
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    id
}
