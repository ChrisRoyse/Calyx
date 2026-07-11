use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, VaultStore};
use calyx_ledger::LedgerAppender;
use serde_json::{Value, json};

use super::*;

#[test]
fn mcp_propose_lens_rejects_algorithmic_signal_before_hot_add() {
    let env = TestEnv::new("propose-driver");
    let server = server();
    call_ok(&server, 100, "calyx.create_vault", json!({"name": "v"}));
    call_ok(
        &server,
        101,
        "calyx.add_lens",
        json!({"vault": "v", "name": "byte_axis", "runtime": "algorithmic", "shape": "Dense(16)"}),
    );
    let vault_dir = vault_dir(&env, "v");
    write_full_budget(&vault_dir);
    let before_panel = calyx_registry::load_vault_panel_state(&vault_dir).unwrap();
    let before_slots = before_panel.panel.slots.len();

    let cx_ids = ingest_frequency_corpus(&server);
    anchor_frequency_outcomes(&server, &cx_ids);

    let proposal = call_ok(
        &server,
        400,
        "calyx.propose_lens",
        json!({"vault": "v", "anchor": "label:frequency"}),
    );

    assert_eq!(
        proposal["admitted"],
        false,
        "proposal={}",
        serde_json::to_string_pretty(&proposal).unwrap()
    );
    assert_eq!(proposal["terminal_state"], "gate_rejected");
    assert_eq!(
        proposal["gate_outcome"]["reason"]["reason"],
        "non_learned_signal"
    );
    assert_eq!(
        proposal["gate_outcome"]["reason"]["signal_kind"],
        "algorithmic"
    );
    assert!(proposal["backfill"].is_null());
    assert!(proposal["ledger_ref"]["seq"].as_u64().unwrap() > 0);

    let after_panel = calyx_registry::load_vault_panel_state(&vault_dir).unwrap();
    assert_eq!(after_panel.panel.slots.len(), before_slots);

    let vault = open_test_vault(&env, "v");
    let docs = super::super::core::load_docs(&vault).unwrap();
    assert_eq!(docs.len(), 60);
    let sample = docs.values().next().expect("stored docs");
    let candidate_slots = after_panel
        .panel
        .slots
        .iter()
        .filter(|slot| slot.slot_id != SlotId::new(0))
        .collect::<Vec<_>>();
    assert_eq!(candidate_slots.len(), before_slots.saturating_sub(1));
    assert!(
        sample
            .slots
            .keys()
            .all(|slot| slot.get() < before_slots as u16)
    );
    assert!(
        vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::Assay,
                &model::assay_key("label:frequency"),
            )
            .unwrap()
            .is_some()
    );
    let operator_row = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::AnnealOperators,
            &model::proposal_key("label:frequency"),
        )
        .unwrap()
        .expect("proposal row");
    let operator_json: Value = serde_json::from_slice(&operator_row).unwrap();
    assert_eq!(operator_json["terminal_state"], "gate_rejected");
    assert_eq!(
        operator_json["gate_outcome"]["reason"]["reason"],
        "non_learned_signal"
    );

    let history = proposal_history(&vault);
    assert!(
        history
            .iter()
            .any(|record| matches!(record, calyx_anneal::AdmissionRecord::LensRejected(_)))
    );
}

#[test]
fn commissioned_placeholder_card_uses_measured_profile_metrics() {
    let env = TestEnv::new("commissioned-profile");
    let vault_dir = env.path("profile-vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let corpus = vec![
        commissioned_doc(&vault_dir, 70, true, "identical profile input"),
        commissioned_doc(&vault_dir, 71, false, "identical profile input"),
    ];
    let candidate = calyx_anneal::CandidateLens::Commission {
        spec: calyx_anneal::CommissionSpec {
            target_modality: calyx_core::Modality::Text,
            endpoint: None,
            model_id: Some("fixture/model".to_string()),
            axis: "unit".to_string(),
            suggested_targets: vec![calyx_anneal::ConversionTarget {
                hf_id: "fixture/model".to_string(),
                modality: calyx_core::Modality::Text,
                axis: "unit".to_string(),
                formats: vec!["adapter".to_string()],
                expected_bits: 0.99,
                expected_cost: calyx_anneal::ExpectedTargetCost {
                    placement: calyx_core::Placement::Cpu,
                    vram_mb: 0.0,
                    ram_mb: 1.0,
                    ms_per_input: 1.0,
                },
                expected_bits_per_vram_mb: None,
                expected_bits_per_ms: 0.99,
            }],
            description: "fixture commission".to_string(),
        },
    };
    let measured = super::super::propose_profile::measure_candidate(
        &vault_dir,
        &AnchorKind::TestPass,
        &candidate,
        &corpus,
    )
    .unwrap();
    let card = super::super::propose_profile::capability_card(
        &measured,
        &corpus,
        &AnchorKind::TestPass,
        measured.cost.unwrap(),
    )
    .unwrap();

    assert_eq!(
        card.signal_kind,
        calyx_registry::CapabilitySignalKind::Placeholder
    );
    assert_eq!(card.health, calyx_registry::LensHealth::Cold);
    assert_eq!(card.coverage.requested, corpus.len());
    assert_eq!(card.coverage.measured, corpus.len());
    assert!(card.signal.unwrap() <= f32::EPSILON);
    assert!(card.low_spread);
    assert!(card.separation.used_labels);
}

fn ingest_frequency_corpus(server: &McpServer) -> Vec<String> {
    (0..60)
        .map(|idx| {
            let positive = idx < 30;
            let input = if positive {
                format!("zzzzzzzzzzzzzzzzzzzzzzzz {idx:02}")
            } else {
                format!("!!!!!!!!!!!!!!!!!!!!!!!! {idx:02}")
            };
            call_ok(
                server,
                110 + idx as u64,
                "calyx.ingest",
                json!({"vault": "v", "input": input}),
            )["cx_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn commissioned_doc(
    vault_dir: &Path,
    seed: u8,
    positive: bool,
    input: &str,
) -> calyx_core::Constellation {
    let mut cx = constellation(
        seed,
        Some(AnchorValue::Bool(positive)),
        vec![1.0, 0.0],
        vec![1.0, 0.0],
    );
    let retained = calyx_aster::retained_input::retain_text_input(vault_dir, input).unwrap();
    cx.input_ref = calyx_core::InputRef {
        hash: *blake3::hash(input.as_bytes()).as_bytes(),
        pointer: retained.pointer,
        redacted: false,
    };
    cx
}

fn anchor_frequency_outcomes(server: &McpServer, cx_ids: &[String]) {
    for (idx, cx_id) in cx_ids.iter().enumerate() {
        call_ok(
            server,
            220 + idx as u64,
            "calyx.anchor",
            json!({
                "vault": "v",
                "cx_id": cx_id,
                "kind": "label",
                "label": "frequency",
                "value": idx < 30,
            }),
        );
    }
}

fn vault_dir(env: &TestEnv, name: &str) -> PathBuf {
    let index: Value =
        serde_json::from_slice(&fs::read(env.path("vaults/index.json")).unwrap()).unwrap();
    let relative = index["vaults"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .and_then(|entry| entry["path"].as_str())
        .unwrap();
    env.home.join(relative)
}

fn write_full_budget(vault_dir: &Path) {
    let dir = vault_dir.join(".anneal");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("budget.toml"),
        "cpu_fraction = 1.0\nvram_bytes = 536870912\ntick_interval_ms = 100\n",
    )
    .unwrap();
}

fn open_test_vault(env: &TestEnv, name: &str) -> AsterVault {
    let resolved =
        crate::tools::vault::store::resolve_vault_info(&env.home, name).expect("resolve vault");
    AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        crate::tools::vault::store::vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )
    .unwrap()
}

fn proposal_history(vault: &AsterVault) -> Vec<calyx_anneal::AdmissionRecord> {
    let appender = LedgerAppender::open(
        calyx_anneal::AsterAnnealLedgerStore::new(vault),
        calyx_core::SystemClock,
    )
    .unwrap();
    let ledger = calyx_anneal::AnnealLedger::new(
        appender,
        calyx_ledger::ActorId::Service("test-readback".to_string()),
    )
    .unwrap();
    calyx_anneal::proposal_history(&ledger, 8).unwrap()
}
