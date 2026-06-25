use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::read_series;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorValue, Asymmetry, FixedClock, LensId, Modality, Panel, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, VaultId, VaultStore,
};
use calyx_oracle::{DomainId, reverse_query};
use calyx_registry::{Registry, persist_vault_panel_state};
use serde_json::json;
use ulid::Ulid;

use super::super::lens::build_lens;
use super::super::vault::{ResolvedVault, now_ms, vault_salt};
use super::batch::parse_batch_line;
use super::command::ingest_batch_streaming;
use super::store::open_vault;

#[test]
fn batch_ingest_structures_oracle_recurrence_for_reverse_query() {
    let (root, resolved) = test_vault("oracle-event");
    let jsonl = resolved.path.join("oracle.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"What treats type 2 diabetes? A metformin B aspirin","#,
            r#""anchors":[{"kind":"label:answer","value":"A"},{"kind":"test-pass","value":"true"}],"#,
            r#""oracle":{"domain":"endocrinology","action":"What treats type 2 diabetes?","#,
            r#""outcome":"A","t_secs":1700000000}}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let cx_id = vault.cx_id_for_input(
        "What treats type 2 diabetes? A metformin B aspirin".as_bytes(),
        1,
    );
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();
    assert_eq!(
        cx.metadata_value(calyx_oracle::ORACLE_DOMAIN_METADATA_KEY),
        Some("endocrinology")
    );
    assert_eq!(
        cx.metadata_value(calyx_oracle::ORACLE_ACTION_METADATA_KEY),
        Some("What treats type 2 diabetes?")
    );

    let series = read_series(&vault, cx_id).unwrap();
    assert_eq!(series.occurrences.len(), 1);
    assert_eq!(series.occurrences[0].t_k.0, 1700000000);
    assert_eq!(
        vault
            .scan_cf_at(snapshot, ColumnFamily::Recurrence)
            .unwrap()
            .len(),
        1
    );

    let causes = reverse_query(
        &vault,
        &AnchorValue::Enum("A".to_string()),
        DomainId::from("endocrinology"),
        &FixedClock::new(1700000001),
    )
    .unwrap();

    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0].action_or_event, "What treats type 2 diabetes?");
    assert!(!causes[0].provisional);
    if let Ok(fsv_root) = std::env::var("CALYX_FSV_ROOT") {
        let recurrence_rows = vault
            .scan_cf_at(snapshot, ColumnFamily::Recurrence)
            .unwrap()
            .len();
        let report = json!({
            "issue": 885,
            "scenario": "oracle_event_batch_ingest_reverse_query",
            "vault_path": resolved.path.display().to_string(),
            "cx_id": cx_id.to_string(),
            "base_metadata": {
                "oracle.domain": cx.metadata_value(calyx_oracle::ORACLE_DOMAIN_METADATA_KEY),
                "oracle.action": cx.metadata_value(calyx_oracle::ORACLE_ACTION_METADATA_KEY),
                "oracle.effect": cx.metadata_value(calyx_oracle::ORACLE_EFFECT_METADATA_KEY),
                "oracle.structured": cx.metadata_value("oracle.structured"),
            },
            "recurrence": {
                "cf_rows": recurrence_rows,
                "occurrences": series.occurrences.len(),
                "frequency": series.frequency,
                "first_t_secs": series.occurrences[0].t_k.0,
                "first_context_bytes": series.occurrences[0].context.bytes.len(),
            },
            "reverse_query": {
                "cause_count": causes.len(),
                "first_action_or_event": causes[0].action_or_event,
                "first_domain": causes[0].domain.as_str(),
                "first_provisional": causes[0].provisional,
                "first_confidence": causes[0].confidence,
                "ledger_seq": causes[0].provenance.seq,
            }
        });
        let fsv_path = Path::new(&fsv_root);
        fs::create_dir_all(fsv_path).unwrap();
        fs::write(
            fsv_path.join("issue885_oracle_event_readback.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    cleanup_root(root);
}

#[test]
fn oracle_reingest_does_not_duplicate_recurrence_occurrence() {
    let (root, resolved) = test_vault("oracle-idempotent");
    let jsonl = resolved.path.join("oracle.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"Question one","oracle":{"domain":"cardiology","#,
            r#""action":"Question one","outcome":"yes","outcome_kind":"label:answer"}}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();
    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let cx_id = vault.cx_id_for_input("Question one".as_bytes(), 1);
    let series = read_series(&vault, cx_id).unwrap();
    assert_eq!(series.occurrences.len(), 1);
    assert_eq!(series.frequency, 1);
    cleanup_root(root);
}

#[test]
fn malformed_oracle_event_is_loud_usage_error() {
    let err = parse_batch_line(
        0,
        r#"{"text":"x","oracle":{"domain":"","action":"q","outcome":"A"}}"#,
    )
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("oracle.domain"));

    let err = parse_batch_line(
        1,
        r#"{"text":"x","oracle":{"domain":"d","action":"q","outcome":"A","t_secs":-1}}"#,
    )
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("oracle.t_secs"));
}

fn test_vault(name: &str) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let mut registry = Registry::new();
    let built = build_lens(
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
    let panel = panel_with_text_slot(lens_id, SlotShape::Dense(16));
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

fn cleanup_root(root: PathBuf) {
    if std::env::var_os("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}

fn temp_root(name: &str) -> PathBuf {
    let parent = std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    parent.join(format!(
        "calyx-cli-ingest-oracle-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}
