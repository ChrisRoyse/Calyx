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
};
pub(super) use calyx_registry::{load_vault_panel_state, persist_vault_panel_state};
use ulid::Ulid;

use crate::cmd::search::rebuild_persistent_indexes;
use crate::cmd::vault::vault_salt;

pub(super) fn seed_home(name: &str) -> (PathBuf, PathBuf) {
    seed_home_with_anchors(name, true)
}

pub(super) fn seed_home_without_anchors(name: &str) -> (PathBuf, PathBuf) {
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
    let failing = register_failing_external_lens(&mut registry, &vault_dir);
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

fn register_failing_external_lens(registry: &mut Registry, root: &Path) -> calyx_core::LensId {
    let name = "issue879-unrelated-failing-external".to_string();
    let script = root.join("issue879-failing-external.py");
    fs::write(
        &script,
        r#"import json, struct, sys
golden = list(b"calyx frozen process runtime identity probe v1")
while True:
    header = sys.stdin.buffer.read(4)
    if not header:
        break
    size = struct.unpack(">I", header)[0]
    payload = json.loads(sys.stdin.buffer.read(size))
    if any(item != golden for item in payload["inputs"]):
        sys.stderr.write("intentional non-golden measurement failure\n")
        sys.exit(17)
    body = json.dumps({"vectors": [[0.25, 0.5, 0.75, 1.0] for _ in payload["inputs"]]}).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(body)))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()
"#,
    )
    .unwrap();
    let cmd = "python3".to_string();
    let args = vec![script.display().to_string()];
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
        .with_timeout(Duration::from_secs(5));
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

pub(super) fn only_matrix(vault_dir: &Path) -> PathBuf {
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

pub(super) fn only_progress(vault_dir: &Path) -> PathBuf {
    let runs = vault_dir.join("idx").join("probe_matrix").join("runs");
    let dirs = fs::read_dir(&runs)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].path().join("progress.json")
}
