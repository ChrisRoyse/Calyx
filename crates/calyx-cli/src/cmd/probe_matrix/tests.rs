use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, VaultId, VaultStore,
};
use calyx_registry::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use calyx_registry::measure::measure_constellation;
use calyx_registry::spec::default_recall_delta;
use calyx_registry::{
    AlgorithmicLens, ExternalCmdLens, LensRuntime, LensSpec, Registry, VaultPanelState,
    load_vault_panel_state, persist_vault_panel_state,
};
use calyx_sextant::RrfProfile;
use ulid::Ulid;

use super::*;
use crate::cmd::search::rebuild_persistent_indexes;
use crate::cmd::vault::vault_salt;

mod bounded;
mod refused;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<ProbeMatrixArgs> {
    match super::parse_probe_matrix(&toks(parts))? {
        Subcommand::ProbeMatrix(args) => Ok(args),
        _ => unreachable!("parse_probe_matrix must return ProbeMatrix"),
    }
}

#[test]
fn parses_probe_matrix_axes() {
    let args = parse(&[
        "corpus",
        "--frontier",
        "type 2 diabetes",
        "--slot",
        "8",
        "--slot",
        "14",
        "--weighted-profile",
        "bridge",
        "--phrasing",
        "clinical",
        "--length",
        "paragraph",
        "--top-k",
        "7",
        "--guard",
        "off",
        "--resident-addr",
        "127.0.0.1:8787",
        "--max-variants",
        "3",
        "--time-budget-ms",
        "5000",
    ])
    .unwrap();

    assert_eq!(args.vault, "corpus");
    assert_eq!(args.frontier, "type 2 diabetes");
    assert_eq!(args.slots, vec![SlotId::new(8), SlotId::new(14)]);
    assert_eq!(args.weighted_profiles, vec![RrfProfile::Bridge]);
    assert_eq!(args.phrasings, vec![ProbePhrasing::Clinical]);
    assert_eq!(args.lengths, vec![ProbeLength::Paragraph]);
    assert_eq!(args.top_k, 7);
    assert_eq!(args.resident_addr, Some("127.0.0.1:8787".parse().unwrap()));
    assert_eq!(args.max_variants, Some(3));
    assert_eq!(args.time_budget_ms, Some(5000));
}

#[test]
fn missing_frontier_fails_closed() {
    let err = parse(&["corpus", "--top-k", "3"]).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--frontier"));
}

#[test]
fn bad_profile_fails_closed() {
    let err = parse(&["corpus", "--frontier", "x", "--weighted-profile", "unknown"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("unknown --weighted-profile"));
}

#[test]
fn probe_matrix_open_options_use_latest_only_router_readback() {
    let (_home, vault_dir) = seed_home("off-open-options");
    let state = load_vault_panel_state(&vault_dir).unwrap();
    let options = super::probe_read_vault_options(&state.panel, GuardChoice::Off);

    assert!(
        !options.restore_mvcc_rows,
        "probe-matrix is read-only latest-state search and must not restore every MVCC row"
    );
    assert!(
        !options.restore_ledger_hook,
        "probe-matrix must not materialize the full ledger hook before latest-state search"
    );
    assert!(
        options.read_only,
        "probe-matrix opens must fail closed before any vault mutation"
    );
    assert_eq!(
        options.selected_cfs,
        Some(vec![
            calyx_aster::cf::ColumnFamily::Base,
            calyx_aster::cf::ColumnFamily::Anchors,
        ]),
        "probe-matrix provenance readback must enumerate only Base plus grounding Anchors CF"
    );
}

#[test]
fn in_region_probe_matrix_opens_panel_slot_cfs_for_guard_hydration() {
    let (_home, vault_dir) = seed_home("in-region-open-options");
    let state = load_vault_panel_state(&vault_dir).unwrap();
    let options = super::probe_read_vault_options(&state.panel, GuardChoice::InRegion);
    let selected = options.selected_cfs.expect("in-region must select CFs");

    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::Base));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::Anchors));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::slot(SlotId::new(8))));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::slot(SlotId::new(14))));
    assert!(
        options.read_only,
        "probe-matrix in-region guard must still use read-only handles"
    );
}

#[test]
fn run_persists_matrix_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy");

    run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "happy".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
        },
    )
    .unwrap();

    let matrix_path = only_matrix(&vault_dir);
    let readback_bytes = fs::read(&matrix_path).unwrap();
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(artifact.schema_version, 5);
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Ok);
    assert!(artifact.run.complete);
    assert_eq!(artifact.run.completed_variant_count, 6);
    assert_eq!(artifact.run.next_variant_index, None);
    assert!(PathBuf::from(&artifact.run.progress_artifact).exists());
    assert!(PathBuf::from(&artifact.run.partial_matrix_artifact).exists());
    assert_eq!(artifact.vault, "happy");
    assert_eq!(artifact.active_slots, vec![SlotId::new(8), SlotId::new(14)]);
    assert_eq!(artifact.diagnostics.query_measurements.len(), 1);
    assert_eq!(
        artifact.diagnostics.query_measurements[0].measure_call_count,
        1
    );
    assert_eq!(
        artifact.diagnostics.query_measurements[0].variant_use_count,
        6
    );
    assert_eq!(
        artifact.diagnostics.query_measurements[0].measured_slot_count,
        2
    );
    assert_eq!(artifact.diagnostics.variant_guard_counts.len(), 6);
    assert!(artifact.diagnostics.variant_guard_counts.iter().all(|row| {
        row.pre_guard_hit_count.is_none()
            && row.post_guard_hit_count.is_none()
            && row.guard_filtered_hit_count.is_none()
    }));
    assert_eq!(artifact.log.schema_version, 1);
    assert_eq!(artifact.log.records.len(), 6);
    assert!(!artifact.log.productive.is_empty());
    assert!(
        artifact
            .log
            .records
            .iter()
            .all(|record| record.accepted_hit_count == 1)
    );
    assert!(artifact.log.records.iter().any(|record| {
        record.hits.iter().any(|hit| {
            hit.provenance
                .iter()
                .any(|p| p == "metadata:source_id=clinical")
        })
    }));
}

#[test]
fn requested_missing_slot_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("bad-slot");

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "bad-slot".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(123)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Clinical],
            lengths: vec![ProbeLength::Phrase],
            top_k: 1,
            guard: GuardChoice::Off,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    let progress_path = only_progress(&vault_dir);
    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["status"], "failed");
    assert_eq!(progress["phase"], "slot_validation_error");
    assert!(!progress_path.with_file_name("matrix.json").exists());
}

fn seed_home(name: &str) -> (PathBuf, PathBuf) {
    seed_home_with_anchors(name, true)
}

fn seed_home_without_anchors(name: &str) -> (PathBuf, PathBuf) {
    seed_home_with_anchors(name, false)
}

fn seed_home_with_anchors(name: &str, anchored: bool) -> (PathBuf, PathBuf) {
    let home =
        std::env::temp_dir().join(format!("calyx-probe-matrix-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([9; 16]));
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    fs::write(
        home.join("vaults").join("index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "vaults": [{
                "name": name,
                "vault_id": vault_id.to_string(),
                "path": format!("vaults/{vault_id}"),
                "panel_template": "text-default"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let mut registry = Registry::new();
    let byte = register_lens(
        &mut registry,
        AlgorithmicLens::byte_features("issue879-byte", Modality::Text),
        "issue879-byte",
        "byte-features",
    );
    let sparse = register_lens(
        &mut registry,
        AlgorithmicLens::sparse_keywords("issue879-sparse", Modality::Text, 64),
        "issue879-sparse",
        "sparse-keywords:64",
    );
    let panel = Panel {
        version: 1,
        slots: vec![
            slot(SlotId::new(8), "issue879-byte", byte, SlotShape::Dense(16)),
            slot(
                SlotId::new(14),
                "issue879-sparse",
                sparse,
                SlotShape::Sparse(64),
            ),
        ],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    };
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault_dir, &panel, &registry).unwrap();
    let state = VaultPanelState {
        panel,
        registry,
        registry_snapshot: None,
    };
    let alpha = measure_constellation(&vault, &state, Input::new(Modality::Text, "alpha"), 1)
        .unwrap()
        .slots;
    let omega = measure_constellation(&vault, &state, Input::new(Modality::Text, "omega"), 1)
        .unwrap()
        .slots;
    let alpha_slot8 = alpha.get(&SlotId::new(8)).unwrap().clone();
    let alpha_slot14 = alpha.get(&SlotId::new(14)).unwrap().clone();
    let omega_slot8 = omega.get(&SlotId::new(8)).unwrap().clone();
    let omega_slot14 = omega.get(&SlotId::new(14)).unwrap().clone();
    for (text, source_id, slot8, slot14) in [
        (
            "clinical dense-only marker",
            "clinical",
            alpha_slot8.clone(),
            omega_slot14.clone(),
        ),
        (
            "mechanistic sparse-only marker",
            "mechanistic",
            omega_slot8.clone(),
            alpha_slot14.clone(),
        ),
        (
            "omega unrelated control",
            "control",
            omega_slot8,
            omega_slot14,
        ),
    ] {
        let mut cx =
            measure_constellation(&vault, &state, Input::new(Modality::Text, text), 1).unwrap();
        cx.slots.insert(SlotId::new(8), slot8);
        cx.slots.insert(SlotId::new(14), slot14);
        cx.metadata = BTreeMap::from([
            ("source_dataset".to_string(), "issue879-fixture".to_string()),
            ("source_id".to_string(), source_id.to_string()),
        ]);
        if anchored {
            cx.anchors.push(Anchor {
                kind: AnchorKind::Label("answer".to_string()),
                value: AnchorValue::Text(source_id.to_string()),
                source: "issue879-test".to_string(),
                observed_at: 1,
                confidence: 1.0,
            });
        }
        vault.put(cx).unwrap();
    }
    vault.flush().unwrap();
    rebuild_persistent_indexes(&vault_dir, &vault).unwrap();
    let mut panel = state.panel.clone();
    let mut registry = state.registry.clone();
    let failing = register_failing_external_lens(&mut registry);
    panel.slots.push(slot(
        SlotId::new(99),
        "issue879-unrelated-failing-external",
        failing,
        SlotShape::Dense(4),
    ));
    persist_vault_panel_state(&vault_dir, &panel, &registry).unwrap();
    (home, vault_dir)
}

fn register_lens(
    registry: &mut Registry,
    lens: AlgorithmicLens,
    name: &str,
    runtime_kind: &str,
) -> calyx_core::LensId {
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: runtime_kind.to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(runtime_kind.to_string()),
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
    lens_id
}

fn register_failing_external_lens(registry: &mut Registry) -> calyx_core::LensId {
    let name = "issue879-unrelated-failing-external".to_string();
    let cmd = "calyx-definitely-missing-external-lens".to_string();
    let args = Vec::<String>::new();
    let args_text = args.join("\0");
    let weights = sha256_digest(&[cmd.as_bytes(), args_text.as_bytes()]);
    let corpus = sha256_digest(&[b"external-cmd-runtime-v1"]);
    let contract = FrozenLensContract::new(
        name.clone(),
        weights,
        corpus,
        SlotShape::Dense(4),
        Modality::Text,
        LensDType::F32,
        NormPolicy::None,
    );
    let lens = ExternalCmdLens::new(&name, &cmd, args.clone(), Modality::Text, 4)
        .with_timeout(Duration::from_millis(100));
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name,
        runtime: LensRuntime::ExternalCmd { cmd, args },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some("issue879-unrelated-failing-external".to_string()),
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
    lens_id
}

fn slot(id: SlotId, key: &str, lens_id: calyx_core::LensId, shape: SlotShape) -> Slot {
    Slot {
        slot_id: id,
        slot_key: SlotKey::new(id, key),
        lens_id,
        shape,
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn only_matrix(vault_dir: &Path) -> PathBuf {
    let root = vault_dir.join("idx").join("probe_matrix");
    let dirs = fs::read_dir(&root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let matrices: Vec<_> = dirs
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("runs"))
        .collect();
    assert_eq!(matrices.len(), 1);
    matrices[0].join("matrix.json")
}

fn only_progress(vault_dir: &Path) -> PathBuf {
    let runs = vault_dir.join("idx").join("probe_matrix").join("runs");
    let dirs = fs::read_dir(&runs)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].path().join("progress.json")
}
