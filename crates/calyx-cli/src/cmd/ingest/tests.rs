use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Asymmetry, CxId, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::load_vault_panel_state;
use proptest::prelude::*;
use ulid::Ulid;

use super::super::vault::{ResolvedVault, now_ms, vault_salt};
use super::anchor::parse_anchor_kind;
use super::batch::read_batch_texts;
use super::command::ingest_texts;
use super::constellation::{measure_constellation, text_input};
use super::parse::{parse_anchor, validate_text};
use super::store::{ensure_base_exists, open_vault};

#[test]
fn ingest_same_text_twice_returns_same_cx_and_second_is_not_new() {
    let (root, resolved) = test_vault("idem", panel_with_unregistered_text_slot());

    let first = ingest_texts(&resolved, &[String::from("hello")]).unwrap();
    let second = ingest_texts(&resolved, &[String::from("hello")]).unwrap();

    assert_eq!(first[0].cx_id, second[0].cx_id);
    assert!(first[0].new);
    assert!(!second[0].new);
    fs::remove_dir_all(root).ok();
}

#[test]
fn anchor_label_kind_round_trips() {
    let kind = parse_anchor_kind("label:positive").unwrap();
    assert_eq!(kind, AnchorKind::Label("positive".to_string()));
    let anchor = Anchor {
        kind,
        value: AnchorValue::Enum("positive".to_string()),
        source: "unit".to_string(),
        observed_at: 7,
        confidence: 0.75,
    };
    let decoded: Anchor = serde_json::from_str(&serde_json::to_string(&anchor).unwrap()).unwrap();
    assert_eq!(decoded, anchor);
}

#[test]
fn measure_outputs_absent_not_zero_filled_and_does_not_store() {
    let (root, resolved) = test_vault("measure", panel_with_unregistered_text_slot());
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();

    let cx = measure_constellation(&vault, &state, text_input("hello".to_string()), 1).unwrap();

    assert!(matches!(
        cx.slots.get(&SlotId::new(0)),
        Some(SlotVector::Absent {
            reason: AbsentReason::LensUnavailable
        })
    ));
    assert_eq!(
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len(),
        0
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_jsonl_empty_and_invalid_edges() {
    let root = temp_root("jsonl");
    fs::create_dir_all(&root).unwrap();
    let empty = root.join("empty.jsonl");
    fs::write(&empty, "").unwrap();
    assert!(read_batch_texts(&empty).unwrap().is_empty());

    let invalid = root.join("bad.jsonl");
    fs::write(&invalid, "{\"text\":\"ok\"}\nnot-json\n").unwrap();
    let err = read_batch_texts(&invalid).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_IO_ERROR");
    assert!(err.message().contains("line 2"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn empty_text_and_bad_confidence_are_usage_errors() {
    assert_eq!(
        validate_text("").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse_anchor(&tokens([
            "v",
            "00000000000000000000000000000000",
            "--kind",
            "label:x",
            "--value",
            "x",
            "--confidence",
            "1.5",
        ]))
        .unwrap_err()
        .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn anchor_unknown_cx_fails_as_vault_access_denied() {
    let (root, resolved) = test_vault("anchor-miss", panel_with_unregistered_text_slot());
    let vault = open_vault(&resolved).unwrap();
    let err = ensure_base_exists(&vault, CxId::from_bytes([9; 16])).unwrap_err();
    assert_eq!(err.code(), "CALYX_VAULT_ACCESS_DENIED");
    fs::remove_dir_all(root).ok();
}

proptest! {
    #[test]
    fn cx_id_derivation_is_deterministic(input in ".*") {
        let salt = b"cli-ingest-salt";
        let left = CxId::from_input(input.as_bytes(), 17, salt);
        let right = CxId::from_input(input.as_bytes(), 17, salt);
        prop_assert_eq!(left, right);
    }
}

fn test_vault(name: &str, panel: Panel) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    (
        root,
        ResolvedVault {
            path,
            name: name.to_string(),
            vault_id,
        },
    )
}

fn panel_with_unregistered_text_slot() -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "synthetic"),
            lens_id: LensId::from_bytes([7; 16]),
            shape: SlotShape::Dense(3),
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

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-ingest-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
