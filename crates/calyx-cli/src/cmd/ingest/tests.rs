use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Asymmetry, CxId, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::{Registry, load_vault_panel_state, persist_vault_panel_state};
use proptest::prelude::*;
use ulid::Ulid;

use super::super::vault::{ResolvedVault, now_ms, vault_salt};
use super::anchor::parse_anchor_kind;
use super::batch::{parse_batch_line, read_batch_texts};
use super::command::{ingest_batch_streaming, ingest_texts};
use super::constellation::{measure_constellation, text_input};
use super::parse::{parse_anchor, validate_text};
use super::store::{ensure_base_exists, open_vault};

#[test]
fn ingest_same_text_twice_returns_same_cx_and_second_is_not_new() {
    let (root, resolved) = test_vault_with_registered_dense_lens("idem");

    let first = ingest_texts(&resolved, &[String::from("hello")]).unwrap();
    let second = ingest_texts(&resolved, &[String::from("hello")]).unwrap();

    assert_eq!(first[0].cx_id, second[0].cx_id);
    assert!(first[0].new);
    assert!(!second[0].new);
    fs::remove_dir_all(root).ok();
}

#[test]
fn ingest_into_fully_unregistered_panel_fails_loud_not_silently_empty() {
    // Doctrine #1273 rule 3: a vault whose every content lens is unavailable must
    // refuse ingest (loud, named), never silently persist an unsearchable cx.
    let (root, resolved) = test_vault("unbound", panel_with_unregistered_text_slot());
    let err = match ingest_texts(&resolved, &[String::from("hello")]) {
        Ok(_) => panic!("ingest into a fully-unregistered panel must fail loud, not Ok"),
        Err(e) => e,
    };
    assert_eq!(
        err.code(),
        "CALYX_LENS_UNREACHABLE",
        "got: {}",
        err.to_json()
    );
    assert!(
        err.message().contains("0/") && err.message().contains("content lenses"),
        "message must name the unavailable lenses: {}",
        err.message()
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn ingest_registered_dense_lens_persists_search_index_files() {
    let (root, resolved) = test_vault_with_registered_dense_lens("persist-index");
    let reports = ingest_texts(
        &resolved,
        &[
            String::from("alpha north signal"),
            String::from("beta south signal"),
            String::from("gamma east signal"),
        ],
    )
    .unwrap();

    let manifest_path = resolved.path.join("idx/search/manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let slot = &manifest["slots"].as_array().unwrap()[0];
    let graph_path = resolved.path.join(slot["graph_rel"].as_str().unwrap());
    let ids_path = resolved.path.join(slot["id_map_rel"].as_str().unwrap());
    let ids: serde_json::Value = serde_json::from_slice(&fs::read(&ids_path).unwrap()).unwrap();

    assert!(reports.iter().all(|report| report.new));
    assert_eq!(manifest["format"], "calyx-search-index-manifest-v1");
    assert_eq!(slot["slot"], 0);
    assert_eq!(slot["dim"], 16);
    assert_eq!(slot["len"], 3);
    assert!(graph_path.is_file());
    assert_eq!(ids["format"], "calyx-search-index-idmap-v1");
    assert_eq!(ids["ids"].as_array().unwrap().len(), 3);
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
    assert!(
        cx.flags.degraded,
        "missing applicable content lens degrades"
    );
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
fn retrieval_only_temporal_absence_does_not_degrade_content_ingest() {
    let (root, resolved) =
        test_vault_with_registered_dense_lens_and_temporal_sidecar("temporal-sidecar-degraded");
    let jsonl = resolved.path.join("plain.jsonl");
    fs::write(&jsonl, "{\"text\":\"alpha temporal sidecar signal\"}\n").unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input(
        "alpha temporal sidecar signal".as_bytes(),
        state.panel.version,
    );
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert!(
        !cx.flags.degraded,
        "expected temporal sidecar absence must not mark content degraded"
    );
    assert!(matches!(
        cx.slots.get(&SlotId::new(0)),
        Some(SlotVector::Dense { dim: 16, .. })
    ));
    assert!(matches!(
        cx.slots.get(&SlotId::new(1)),
        Some(SlotVector::Absent {
            reason: AbsentReason::NotApplicable
        })
    ));

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
fn batch_ingest_threads_anchors_into_base_cf_and_anchors_cf() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-at-ingest");
    let jsonl = resolved.path.join("anchored.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"},{"kind":"test-pass","value":"true"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    // FSV: read the anchors back from the stored constellation (base-CF), not from
    // the ingest return value. cx_id is derived from the input bytes + panel version.
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input("alpha north signal".as_bytes(), state.panel.version);
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert_eq!(
        cx.anchors.len(),
        2,
        "both anchors persisted on the constellation"
    );
    assert!(cx.anchors.iter().any(|anchor| {
        anchor.kind == AnchorKind::Label("answer".to_string())
            && anchor.value == AnchorValue::Enum("B".to_string())
            && anchor.source == "calyx-ingest"
            && anchor.confidence == 1.0
    }));
    assert!(cx.anchors.iter().any(|anchor| {
        anchor.kind == AnchorKind::TestPass && anchor.value == AnchorValue::Bool(true)
    }));
    // A constellation carrying its own anchor is grounded at distance 0.
    assert!(
        !cx.flags.ungrounded,
        "anchored constellation is not ungrounded"
    );

    // FSV: anchors are physically present in the Anchors CF — the index the kernel's
    // `domain_anchors(kind)` reads to find grounded nodes. One row per (cx, kind).
    let anchor_rows = vault.scan_cf_at(snapshot, ColumnFamily::Anchors).unwrap();
    assert_eq!(anchor_rows.len(), 2, "two anchor rows in the Anchors CF");

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_merges_anchors_for_existing_cx() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-backfill");
    let plain = resolved.path.join("plain-backfill.jsonl");
    let anchored = resolved.path.join("anchored-backfill.jsonl");
    fs::write(&plain, "{\"text\":\"alpha north signal\"}\n").unwrap();
    fs::write(
        &anchored,
        concat!(
            r#"{"text":"alpha north signal","#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &plain).unwrap();
    let vault_before = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault_before.cx_id_for_input("alpha north signal".as_bytes(), state.panel.version);
    let before = vault_before.get(cx_id, vault_before.snapshot()).unwrap();
    assert!(before.anchors.is_empty());
    assert!(before.flags.ungrounded);
    drop(vault_before);

    ingest_batch_streaming(&resolved, &anchored).unwrap();

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot = vault_after.snapshot();
    let after = vault_after.get(cx_id, snapshot).unwrap();
    let anchor_rows = vault_after
        .scan_cf_at(snapshot, ColumnFamily::Anchors)
        .unwrap();
    let ledger_rows = vault_after
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .unwrap();

    assert_eq!(after.anchors.len(), 1);
    assert_eq!(
        after.anchors[0].kind,
        AnchorKind::Label("answer".to_string())
    );
    assert_eq!(after.anchors[0].value, AnchorValue::Enum("B".to_string()));
    assert!(!after.flags.ungrounded);
    assert_eq!(anchor_rows.len(), 1);
    assert!(
        ledger_rows.len() >= 3,
        "ingest, idempotent, and anchor ledger rows"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_without_anchors_stays_ungrounded() {
    let (root, resolved) = test_vault_with_registered_dense_lens("no-anchors-at-ingest");
    let jsonl = resolved.path.join("plain.jsonl");
    fs::write(&jsonl, "{\"text\":\"beta south signal\"}\n").unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input("beta south signal".as_bytes(), state.panel.version);
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert!(cx.anchors.is_empty());
    assert!(cx.flags.ungrounded, "no anchors => ungrounded stays true");
    assert!(
        vault
            .scan_cf_at(snapshot, ColumnFamily::Anchors)
            .unwrap()
            .is_empty()
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_jsonl_malformed_anchor_is_loud_usage_error() {
    // Unknown anchor kind must fail loudly (no silent drop of a grounding truth).
    let bad_kind = parse_batch_line(
        0,
        "{\"text\":\"x\",\"anchors\":[{\"kind\":\"bogus\",\"value\":\"y\"}]}",
    )
    .unwrap_err();
    assert_eq!(bad_kind.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(bad_kind.message().contains("line 1"));

    // Out-of-range confidence is rejected at parse time.
    let bad_conf = parse_batch_line(
        4,
        "{\"text\":\"x\",\"anchors\":[{\"kind\":\"label:a\",\"value\":\"v\",\"confidence\":1.5}]}",
    )
    .unwrap_err();
    assert_eq!(bad_conf.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(bad_conf.message().contains("line 5"));
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

fn test_vault_with_registered_dense_lens(name: &str) -> (std::path::PathBuf, ResolvedVault) {
    test_vault_with_registered_dense_lens_and_panel(name, false)
}

fn test_vault_with_registered_dense_lens_and_temporal_sidecar(
    name: &str,
) -> (std::path::PathBuf, ResolvedVault) {
    test_vault_with_registered_dense_lens_and_panel(name, true)
}

fn test_vault_with_registered_dense_lens_and_panel(
    name: &str,
    temporal_sidecar: bool,
) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let mut registry = Registry::new();
    let built = super::super::lens::build_lens(
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
    let panel = if temporal_sidecar {
        panel_with_text_slot_and_temporal_sidecar(lens_id, SlotShape::Dense(16))
    } else {
        panel_with_text_slot(lens_id, SlotShape::Dense(16))
    };
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

fn panel_with_unregistered_text_slot() -> Panel {
    panel_with_text_slot(LensId::from_bytes([7; 16]), SlotShape::Dense(3))
}

fn panel_with_text_slot_and_temporal_sidecar(lens_id: LensId, shape: SlotShape) -> Panel {
    let mut panel = panel_with_text_slot(lens_id, shape);
    let slot = SlotId::new(1);
    panel.version = 2;
    panel.slots.push(Slot {
        slot_id: slot,
        slot_key: SlotKey::new(slot, "E2_recency"),
        lens_id: LensId::from_bytes([8; 16]),
        shape: SlotShape::Dense(1),
        modality: Modality::Structured,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("E2_recency".to_string()),
        retrieval_only: true,
        excluded_from_dedup: true,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 2,
    });
    panel
}

fn panel_with_text_slot(lens_id: LensId, shape: SlotShape) -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "synthetic"),
            lens_id,
            shape,
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
