use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::manifest::{ImmutableRef, ManifestStore};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Input, Lens, LensId, Modality, Panel, QuantPolicy, SlotId, SlotShape, SlotState,
    SlotVector, VaultId,
};
use calyx_registry::{
    AlgorithmicLens, LensRuntime, LensSpec, Registry, SlotSpec, SwapController, list_panel,
    load_vault_panel_state, persist_vault_panel_state,
};
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue752_vault_bound_panel_registry_roundtrip_and_edges() {
    let (root, keep_root) = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create issue752 root");

    let happy = happy_path(&root.join("happy"));
    let no_registry = no_registry_edge(&root.join("edge-no-registry"));
    let corrupt = corrupt_registry_edge(&root.join("edge-corrupt-registry"));
    let unsupported = unsupported_runtime_edge(&root.join("edge-unsupported-runtime"));

    let readback = json!({
        "source_of_truth": "vault CURRENT, immutable MANIFEST, panel JSON asset, registry JSON asset",
        "happy": happy,
        "edge_no_registry_ref": no_registry,
        "edge_corrupt_registry_ref": corrupt,
        "edge_unsupported_runtime": unsupported,
    });
    let readback_path = root.join("issue752-vault-panel-registry-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .expect("write issue752 readback");
    println!("ISSUE752_FSV_ROOT={}", root.display());
    println!("ISSUE752_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["happy"]["registry_contains_after_load"], true);
    assert_eq!(readback["happy"]["measure_after_load"]["dim"], 16);
    assert_eq!(readback["happy"]["parked_after_reload"], true);
    assert_eq!(
        readback["edge_no_registry_ref"]["registry_snapshot_present"],
        false
    );
    assert_eq!(readback["edge_no_registry_ref"]["registry_lens_count"], 0);
    assert_eq!(
        readback["edge_corrupt_registry_ref"]["load_error"],
        "CALYX_ASTER_CORRUPT_SHARD"
    );
    assert_eq!(
        readback["edge_unsupported_runtime"]["measure_error"],
        "CALYX_LENS_UNREACHABLE"
    );

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup issue752 registry fsv root");
    }
}

fn happy_path(vault_dir: &Path) -> serde_json::Value {
    let fixture = persist_lens_vault(
        vault_dir,
        AlgorithmicLens::byte_features("issue752-byte", Modality::Text),
        LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        },
        "issue752-byte",
        100,
    );
    let manifest = ManifestStore::open(vault_dir)
        .load_current()
        .expect("load happy manifest");
    let panel_bytes = read_ref(vault_dir, &fixture.write.panel_ref);
    let registry_bytes = read_ref(vault_dir, &fixture.write.registry_ref);
    assert!(String::from_utf8_lossy(&registry_bytes).contains(&fixture.lens_id.to_string()));

    let loaded = load_vault_panel_state(vault_dir).expect("load persisted panel state");
    let measured = loaded
        .registry
        .measure(
            fixture.lens_id,
            &Input::new(Modality::Text, b"issue752 cold load probe".to_vec()),
        )
        .expect("measure loaded runtime");
    let mut controller = SwapController::new(loaded.panel.clone());
    controller
        .park_lens(fixture.slot_id, 101)
        .expect("park loaded slot");
    let second = persist_vault_panel_state(vault_dir, controller.panel(), &loaded.registry)
        .expect("persist parked panel");
    let reloaded = load_vault_panel_state(vault_dir).expect("reload parked panel");
    let listing = list_panel(&reloaded.panel, &reloaded.registry);
    let parked = listing
        .iter()
        .any(|slot| slot.slot_id == fixture.slot_id && slot.state == SlotState::Parked);

    json!({
        "manifest_seq_after_first_persist": fixture.write.manifest_seq,
        "manifest_seq_after_park": second.manifest_seq,
        "durable_seq_after_persist": manifest.durable_seq,
        "panel_ref": fixture.write.panel_ref.logical_path,
        "registry_ref": fixture.write.registry_ref.logical_path,
        "panel_hash": blake3::hash(&panel_bytes).to_hex().to_string(),
        "registry_hash": blake3::hash(&registry_bytes).to_hex().to_string(),
        "registry_json_contains_lens_id": String::from_utf8_lossy(&registry_bytes)
            .contains(&fixture.lens_id.to_string()),
        "registry_contains_after_load": loaded.registry.contains(fixture.lens_id),
        "lens_spec_roundtrip": loaded.registry.lens_spec(fixture.lens_id) == Some(&fixture.spec),
        "snapshot_lens_count": loaded
            .registry_snapshot
            .as_ref()
            .map(|snapshot| snapshot.lenses.len()),
        "measure_after_load": vector_summary(&measured),
        "parked_after_reload": parked,
        "list_panel": listing,
    })
}

fn no_registry_edge(vault_dir: &Path) -> serde_json::Value {
    create_panel_vault(vault_dir);
    let manifest = ManifestStore::open(vault_dir)
        .load_current()
        .expect("load no-registry manifest");
    let loaded = load_vault_panel_state(vault_dir).expect("load no-registry state");
    json!({
        "registry_ref_present": manifest.registry_ref.is_some(),
        "registry_snapshot_present": loaded.registry_snapshot.is_some(),
        "registry_lens_count": loaded.registry.lens_snapshots().len(),
        "panel_version": loaded.panel.version,
    })
}

fn corrupt_registry_edge(vault_dir: &Path) -> serde_json::Value {
    let fixture = persist_lens_vault(
        vault_dir,
        AlgorithmicLens::byte_features("issue752-corrupt", Modality::Text),
        LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        },
        "issue752-corrupt",
        200,
    );
    let registry_path = vault_dir.join(&fixture.write.registry_ref.logical_path);
    let before = fs::read(&registry_path).expect("read registry before corruption");
    fs::write(&registry_path, b"{\"corrupted\":\"issue752\"}").expect("corrupt registry asset");
    let after = fs::read(&registry_path).expect("read registry after corruption");
    let error = match load_vault_panel_state(vault_dir) {
        Ok(_) => panic!("corrupt registry hash accepted"),
        Err(error) => error,
    };
    json!({
        "registry_ref": fixture.write.registry_ref.logical_path,
        "before_hash": blake3::hash(&before).to_hex().to_string(),
        "after_hash": blake3::hash(&after).to_hex().to_string(),
        "hash_changed": before != after,
        "load_error": error.code,
        "message": error.message,
    })
}

fn unsupported_runtime_edge(vault_dir: &Path) -> serde_json::Value {
    let fixture = persist_lens_vault(
        vault_dir,
        AlgorithmicLens::scalar("issue752-cold-candle", Modality::Text),
        LensRuntime::CandleLocal {
            model_id: "issue752-missing-model".to_string(),
            files: Vec::new(),
            dtype: "f32".to_string(),
            pooling: "mean".to_string(),
        },
        "issue752-cold-candle",
        300,
    );
    let loaded = load_vault_panel_state(vault_dir).expect("load unsupported runtime state");
    let error = loaded
        .registry
        .measure(
            fixture.lens_id,
            &Input::new(Modality::Text, b"unsupported runtime probe".to_vec()),
        )
        .expect_err("unsupported runtime placeholder fails closed");
    let listing = list_panel(&loaded.panel, &loaded.registry);
    json!({
        "registry_contains_after_load": loaded.registry.contains(fixture.lens_id),
        "measure_error": error.code,
        "health_after_load": listing[0].health,
        "snapshot_runtime": fixture.spec.runtime,
    })
}

struct PersistedFixture {
    lens_id: LensId,
    slot_id: SlotId,
    spec: LensSpec,
    write: calyx_registry::VaultPanelWrite,
}

fn persist_lens_vault(
    vault_dir: &Path,
    lens: AlgorithmicLens,
    runtime: LensRuntime,
    slot_key: &str,
    now: u64,
) -> PersistedFixture {
    create_panel_vault(vault_dir);
    let mut registry = Registry::new();
    let spec = lens_spec(&lens, runtime);
    let lens_id = registry
        .register_frozen_with_spec(lens.clone(), lens.contract().clone(), spec.clone())
        .expect("register frozen lens with spec");
    let mut controller = SwapController::new(empty_panel());
    let add = controller
        .add_lens(
            &registry,
            SlotSpec::dense_text(slot_key, lens_id, dense_dim(lens.shape())),
            [],
            now,
        )
        .expect("add lens to panel");
    let write = persist_vault_panel_state(vault_dir, controller.panel(), &registry)
        .expect("persist panel and registry");
    PersistedFixture {
        lens_id,
        slot_id: add.slot.slot_id,
        spec,
        write,
    }
}

fn create_panel_vault(vault_dir: &Path) {
    let _ = fs::remove_dir_all(vault_dir);
    let options = VaultOptions {
        panel: Some(empty_panel()),
        ..VaultOptions::default()
    };
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue752-vault-panel-registry".to_vec(),
        options,
    )
    .expect("create panel vault");
}

fn lens_spec(lens: &AlgorithmicLens, runtime: LensRuntime) -> LensSpec {
    LensSpec {
        name: lens.contract().name().to_string(),
        runtime,
        output: lens.shape(),
        modality: lens.modality(),
        weights_sha256: lens.contract().weights_sha256(),
        corpus_hash: lens.contract().corpus_hash(),
        norm_policy: lens.contract().norm_policy(),
        max_batch: None,
        axis: Some("issue752".to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
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

fn dense_dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) => dim,
        _ => panic!("issue752 test lenses must be dense"),
    }
}

fn vector_summary(vector: &SlotVector) -> serde_json::Value {
    match vector {
        SlotVector::Dense { dim, data } => json!({
            "kind": "dense",
            "dim": dim,
            "len": data.len(),
            "first": data.first().copied(),
        }),
        other => json!({"kind": format!("{other:?}")}),
    }
}

fn read_ref(vault_dir: &Path, reference: &ImmutableRef) -> Vec<u8> {
    fs::read(vault_dir.join(&reference.logical_path)).expect("read manifest ref")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn fsv_root() -> (PathBuf, bool) {
    if let Some(root) = std::env::var_os("CALYX_ISSUE752_PANEL_REGISTRY_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    (
        std::env::temp_dir().join(format!(
            "calyx-registry-issue752-panel-registry-{}-{id}",
            std::process::id()
        )),
        false,
    )
}
