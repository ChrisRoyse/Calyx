use std::collections::BTreeMap;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorKind, AnchorValue, Asymmetry, CxFlags, CxId, FixedClock, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, VaultId, VaultStore,
};
use proptest::prelude::*;
use serde_json::json;

use super::*;
use crate::{
    CALYX_ORACLE_INSUFFICIENT, CALYX_ORACLE_LEDGER_WRITE_FAILURE, CALYX_ORACLE_NO_RECURRENCE,
};

const DOMAIN: &str = "issue432";
const ACTION: &str = "action_A";

#[test]
fn action_a_twenty_pass_observations_predict_pass_under_ceiling() {
    let vault = vault();
    let panel = panel(&[1, 2]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_ceiling_point_95(&vault, DOMAIN);
    for seed in 100..120 {
        add_series(
            &vault,
            seed,
            DOMAIN,
            ACTION,
            &[Row::prediction("Pass", Some("ci_passed"))],
        );
    }

    let prediction = oracle_predict(
        &vault,
        &action(ACTION, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(900),
    )
    .expect("prediction");

    assert_eq!(prediction.outcome, AnchorValue::Text("Pass".to_string()));
    assert!(prediction.confidence > 0.5);
    assert!(prediction.confidence <= 0.95 + f32::EPSILON);
    assert!(prediction.confidence <= prediction.bound.dpi_ceiling);
    assert!(!prediction.consequences.is_empty());
    assert!(prediction.guard.provisional);
    let payload = ledger_payload(&vault, prediction.provenance);
    assert_eq!(payload["tag"], LEDGER_TAG);
    assert_eq!(payload["recurrence_observations"], 20);
}

#[test]
fn raw_confidence_is_capped_by_self_consistency_exactly() {
    assert_close(apply_confidence_ceiling(0.9, 0.7, 1.0), 0.7);
}

#[test]
fn insufficient_panel_returns_before_recurrence_query() {
    let vault = vault();
    let panel = panel(&[1]);
    put_sufficiency(&vault, &panel, 0.25, 0.75);

    let error = oracle_predict(
        &vault,
        &action(ACTION, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(901),
    )
    .expect_err("insufficient");

    assert_eq!(error.code(), CALYX_ORACLE_INSUFFICIENT);
    assert!(matches!(error, OracleError::Insufficient { .. }));
}

#[test]
fn no_matching_action_recurrence_fails_closed() {
    let vault = vault();
    let panel = panel(&[1]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_ceiling_point_95(&vault, DOMAIN);

    let error = oracle_predict(
        &vault,
        &action("missing_action", panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(902),
    )
    .expect_err("no action recurrence");

    assert_eq!(error.code(), CALYX_ORACLE_NO_RECURRENCE);
}

#[test]
fn single_observation_returns_low_confidence_prediction() {
    let vault = vault();
    let panel = panel(&[1]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_ceiling_point_95(&vault, DOMAIN);
    add_series(
        &vault,
        130,
        DOMAIN,
        ACTION,
        &[Row::prediction("Pass", Some("ci_passed"))],
    );

    let prediction = oracle_predict(
        &vault,
        &action(ACTION, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(903),
    )
    .expect("single observation predicts");

    assert_eq!(prediction.outcome, AnchorValue::Text("Pass".to_string()));
    assert!(prediction.confidence > 0.0);
    assert!(prediction.confidence < 0.5);
}

#[test]
fn uniform_disagreement_has_near_zero_confidence() {
    let vault = vault();
    let panel = panel(&[1]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_ceiling_point_95(&vault, DOMAIN);
    for seed in 140..150 {
        add_series(
            &vault,
            seed,
            DOMAIN,
            ACTION,
            &[Row::prediction("Pass", None)],
        );
    }
    for seed in 150..160 {
        add_series(
            &vault,
            seed,
            DOMAIN,
            ACTION,
            &[Row::prediction("Fail", None)],
        );
    }

    let prediction = oracle_predict(
        &vault,
        &action(ACTION, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(904),
    )
    .expect("uniform prediction");

    assert!(prediction.confidence <= 0.001);
}

#[test]
fn ledger_write_failure_fails_closed_without_prediction() {
    let vault = vault();
    let panel = panel(&[1]);
    let bad_action = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa+";
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_ceiling_point_95(&vault, DOMAIN);
    add_series(
        &vault,
        170,
        DOMAIN,
        bad_action,
        &[Row::prediction("Pass", Some("ci_passed"))],
    );

    let error = oracle_predict(
        &vault,
        &action(bad_action, panel),
        DomainId::from(DOMAIN),
        &FixedClock::new(905),
    )
    .expect_err("ledger redaction rejects token-like action id");

    assert_eq!(error.code(), CALYX_ORACLE_LEDGER_WRITE_FAILURE);
}

proptest! {
    #[test]
    fn confidence_cap_never_exceeds_dpi(raw in -1.0f32..2.0, ceiling in -1.0f32..2.0, dpi in 0.0f32..2.0) {
        prop_assert!(apply_confidence_ceiling(raw, ceiling, dpi) <= dpi.clamp(0.0, 1.0) + f32::EPSILON);
    }
}

#[derive(Clone, Copy)]
struct Row {
    outcome: &'static str,
    truth: Option<&'static str>,
    consequence: Option<&'static str>,
}

impl Row {
    fn truth(outcome: &'static str) -> Self {
        Self {
            outcome,
            truth: Some(outcome),
            consequence: None,
        }
    }

    fn prediction(outcome: &'static str, consequence: Option<&'static str>) -> Self {
        Self {
            outcome,
            truth: None,
            consequence,
        }
    }
}

fn seed_ceiling_point_95(vault: &AsterVault<FixedClock>, domain: &str) {
    add_series(vault, 1, domain, "calibration", &[Row::truth("pass"); 6]);
    add_series(vault, 2, domain, "calibration", &[Row::truth("pass"); 3]);
    add_series(vault, 3, domain, "calibration", &[Row::truth("pass"); 2]);
    add_series(
        vault,
        4,
        domain,
        "calibration",
        &[Row::truth("pass"), Row::truth("fail")],
    );
    for idx in 0..37 {
        let outcome = if idx % 2 == 0 { "pass" } else { "fail" };
        add_series(
            vault,
            5 + idx,
            domain,
            "calibration",
            &[Row::truth(outcome)],
        );
    }
}

fn add_series(
    vault: &AsterVault<FixedClock>,
    seed: u8,
    domain: &str,
    action_id: &str,
    rows: &[Row],
) {
    let cx_id = CxId::from_bytes([seed; 16]);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&constellation(cx_id, domain, action_id))
                .expect("encode base"),
        )
        .expect("write base");
    for (occ_idx, row) in rows.iter().enumerate() {
        let occurrence = Occurrence {
            id: OccurrenceId(occ_idx as u64),
            t_k: EpochSecs(1_000 + occ_idx as i64),
            context: OccurrenceContext::new(context(domain, action_id, row)).expect("context"),
        };
        vault
            .write_cf(
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, occ_idx as u64),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                    .expect("encode recurrence"),
            )
            .expect("write recurrence");
    }
}

fn context(domain: &str, action_id: &str, row: &Row) -> Vec<u8> {
    let mut value = json!({
        "action": action_id,
        "oracle_verdict": { "value": { "text": row.outcome } },
        "outcome_anchor": { "value": { "text": row.outcome } }
    });
    if let Some(truth) = row.truth {
        value["ground_truth_anchor"] = json!({ "value": { "text": truth } });
    }
    if let Some(action_or_event) = row.consequence {
        value["consequences"] = json!([{
            "action_or_event": action_or_event,
            "domain": domain,
            "outcome": { "value": { "text": "Deployable" } }
        }]);
    }
    serde_json::to_vec(&value).expect("context json")
}

fn put_sufficiency(
    vault: &AsterVault<FixedClock>,
    panel: &Panel,
    panel_bits: f32,
    entropy_bits: f32,
) {
    let key = AssayCacheKey::scoped(panel.version, DOMAIN, vault_id(), AnchorKind::Reward);
    let mut store = AssayStore::default();
    store.put(
        key.clone(),
        AssaySubject::Panel,
        estimate(panel_bits, EstimatorKind::PanelSufficiency),
        "oracle predict panel bits",
        1,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        estimate(entropy_bits, EstimatorKind::OutcomeEntropy),
        "oracle predict entropy",
        1,
    );
    for slot in &panel.slots {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: slot.slot_id },
            estimate(
                panel_bits / panel.slots.len().max(1) as f32,
                EstimatorKind::Ksg,
            ),
            "oracle predict lens bits",
            1,
        );
    }
    store.persist_to_vault(vault).expect("persist assay");
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 120, estimator, TrustTag::Trusted)
}

fn action(action_id: &str, panel: Panel) -> Action {
    Action {
        action_id: action_id.to_string(),
        panel,
        guard: None,
    }
}

fn panel(slots: &[u16]) -> Panel {
    Panel {
        version: 432,
        slots: slots.iter().copied().map(slot).collect(),
        created_at: 1_785_600_000,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Code,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("oracle-predict-fixture".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 432,
    }
}

fn constellation(cx_id: CxId, domain: &str, action_id: &str) -> calyx_core::Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(
        ORACLE_ACTION_METADATA_KEY.to_string(),
        action_id.to_string(),
    );
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 432,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn ledger_payload(vault: &AsterVault<FixedClock>, ledger_ref: LedgerRef) -> serde_json::Value {
    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(ledger_ref.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    let entry = calyx_ledger::decode(&bytes).expect("decode ledger");
    serde_json::from_slice(&entry.payload).expect("payload json")
}

fn vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"issue432-salt", FixedClock::new(1))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-6);
}
