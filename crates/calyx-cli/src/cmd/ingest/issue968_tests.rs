use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::encode::encode_slot_vector;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
    SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::{Registry, persist_vault_panel_state};
use serde_json::{Value, json};
use ulid::Ulid;

use super::command::ingest_batch_streaming;
use super::store::open_vault;
use crate::cmd::vault::{ResolvedVault, now_ms, vault_salt};

#[test]
fn issue968_exact_replay_with_missing_base_anchors_preserves_stored_slots() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue968");
    let plain = resolved.path.join("plain.jsonl");
    let anchored = resolved.path.join("anchored.jsonl");
    fs::write(
        &plain,
        r#"{"text":"alpha issue968 replay","metadata":{"source_dataset":"issue968"}}"#,
    )
    .unwrap();
    fs::write(&anchored, anchored_row()).unwrap();

    ingest_batch_streaming(&resolved, &plain).expect("plain ingest");
    let vault = open_vault(&resolved).unwrap();
    let cx_id = vault.cx_id_for_input(b"alpha issue968 replay", 1);
    let replacement_slot = SlotVector::Dense {
        dim: 16,
        data: (0..16).map(|index| 100.0 + index as f32).collect(),
    };
    let replacement_slot_bytes = encode_slot_vector(&replacement_slot).unwrap();
    vault
        .write_cf(
            ColumnFamily::slot(SlotId::new(0)),
            slot_key(cx_id),
            replacement_slot_bytes.clone(),
        )
        .unwrap();
    vault.flush().unwrap();

    let before_anchor_replay = physical_state(&vault, cx_id);
    drop(vault);

    ingest_batch_streaming(&resolved, &anchored).expect("anchored replay repairs anchors");
    let vault = open_vault(&resolved).unwrap();
    let after_anchor_replay = physical_state(&vault, cx_id);
    assert_eq!(
        after_anchor_replay["slot_00"]["blake3"], before_anchor_replay["slot_00"]["blake3"],
        "replay must preserve existing slot bytes as the source of truth"
    );
    assert_eq!(after_anchor_replay["base_anchor_count"], 3);
    assert_eq!(after_anchor_replay["anchors_cf_rows"], 3);
    assert_eq!(
        after_anchor_replay["ledger_rows"].as_u64().unwrap(),
        before_anchor_replay["ledger_rows"].as_u64().unwrap() + 4,
        "one idempotent ingest ledger row plus three anchor marker rows"
    );
    drop(vault);

    ingest_batch_streaming(&resolved, &anchored).expect("third exact replay is stable");
    let vault = open_vault(&resolved).unwrap();
    let after_third_replay = physical_state(&vault, cx_id);
    assert_eq!(
        after_third_replay["slot_00"]["blake3"],
        before_anchor_replay["slot_00"]["blake3"]
    );
    assert_eq!(after_third_replay["base_anchor_count"], 3);
    assert_eq!(after_third_replay["anchors_cf_rows"], 3);
    assert_eq!(
        after_third_replay["ledger_rows"].as_u64().unwrap(),
        after_anchor_replay["ledger_rows"].as_u64().unwrap() + 1,
        "stable exact replay records only the idempotent ingest ledger row"
    );

    write_fsv(
        &resolved,
        &before_anchor_replay,
        &after_anchor_replay,
        &after_third_replay,
    );
    fs::remove_dir_all(root).ok();
}

fn anchored_row() -> &'static str {
    concat!(
        r#"{"text":"alpha issue968 replay","metadata":{"source_dataset":"issue968"},"#,
        r#""anchors":["#,
        r#"{"kind":"label:campaign","value":"calyx15000-2m","source":"issue968","confidence":1.0},"#,
        r#"{"kind":"label:source_type","value":"ops_script","source":"issue968","confidence":1.0},"#,
        r#"{"kind":"label:source_path","value":"scripts\\build-calyx-ingest-batch.ps1","source":"issue968","confidence":1.0}"#,
        r#"]}"#,
        "\n",
    )
}

fn physical_state(vault: &AsterVault, cx_id: calyx_core::CxId) -> Value {
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();
    let slot_bytes = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::slot(SlotId::new(0)),
            &slot_key(cx_id),
        )
        .unwrap()
        .expect("slot row");
    json!({
        "snapshot": snapshot,
        "cx_id": cx_id,
        "base_anchor_count": cx.anchors.len(),
        "base_anchor_kinds": cx.anchors.iter().map(|anchor| anchor_kind(anchor.kind.clone())).collect::<Vec<_>>(),
        "anchors_cf_rows": vault.scan_cf_at(snapshot, ColumnFamily::Anchors).unwrap().len(),
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).unwrap().len(),
        "base_rows": vault.scan_cf_at(snapshot, ColumnFamily::Base).unwrap().len(),
        "slot_00": {
            "bytes": slot_bytes.len(),
            "blake3": blake3::hash(&slot_bytes).to_hex().to_string(),
        },
    })
}

fn write_fsv(
    resolved: &ResolvedVault,
    before_anchor_replay: &Value,
    after_anchor_replay: &Value,
    after_third_replay: &Value,
) {
    let Some(root) = std::env::var_os("CALYX_FSV_ROOT") else {
        return;
    };
    let root = PathBuf::from(root).join("issue968-batch-duplicate-replay");
    fs::create_dir_all(&root).unwrap();
    let artifact = json!({
        "issue": 968,
        "source_of_truth": {
            "base_cf": "cf/base",
            "anchors_cf": "cf/anchors",
            "ledger_cf": "cf/ledger",
            "slot_cf": "cf/slot_00",
        },
        "trigger": "anchored JSONL replay after an existing Base row already owns the CxId",
        "expected": {
            "anchor_replay": "adds missing anchors without rewriting slot bytes",
            "third_replay": "adds one idempotent ingest ledger row and no anchor rows",
        },
        "vault": {
            "name": resolved.name,
            "vault_id": resolved.vault_id.to_string(),
        },
        "before_anchor_replay": before_anchor_replay,
        "after_anchor_replay": after_anchor_replay,
        "after_third_replay": after_third_replay,
    });
    fs::write(
        root.join("issue968-batch-duplicate-replay-readback.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}

fn test_vault_with_registered_dense_lens(name: &str) -> (PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let mut registry = Registry::new();
    let built = crate::cmd::lens::build_lens(
        "algo16",
        "algorithmic",
        None,
        None,
        Some("Dense(16)"),
        Some("text"),
    )
    .unwrap();
    let lens_id = built.lens_id;
    built.register(&mut registry).unwrap();
    let panel = panel_with_text_slot(lens_id);
    AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&path, &panel, &registry).unwrap();
    (
        root,
        ResolvedVault {
            path,
            name: name.to_string(),
            vault_id,
        },
    )
}

fn panel_with_text_slot(lens_id: LensId) -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "synthetic"),
            lens_id,
            shape: SlotShape::Dense(16),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("synthetic".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-ingest-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn anchor_kind(kind: AnchorKind) -> String {
    match kind {
        AnchorKind::Label(axis) => format!("label:{axis}"),
        other => format!("{other:?}"),
    }
}
