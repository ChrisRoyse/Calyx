//! FSV for issue #215 — the vault-backed calyx-oracle forward predictor wired as a first-class
//! forecast component and blended alongside kNN (#81) and bits-vote (#84).
//!
//! Full-state verification: a real `AsterVault` is seeded with real recurrence rows and a real
//! self-consistency calibration series, the real `calyx_oracle::oracle_predict` runs against it, the
//! emitted component is persisted and read back byte-identically, and the recurrence observation
//! count is cross-checked against the **vault ledger CF** (the source of truth) — not the return
//! value. Synthetic known-truth inputs (X of "YES", Y of "NO") give known expected outputs. No mock
//! oracle, no fallbacks, fail loud.

use std::collections::BTreeMap;

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, PowerCalibration, TrustTag,
};
use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorKind, AnchorValue, Asymmetry, Constellation, CxFlags, CxId, FixedClock, InputRef,
    LedgerRef, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
    VaultId, VaultStore,
};
use calyx_oracle::{Action, DomainId, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY};
use calyx_poly::forecast::ComponentKind;
use calyx_poly::forecast_blend::blend_components;
use calyx_poly::oracle_forecast::{
    ERR_ORACLE_PREDICT, OracleForecast, produce_oracle_forecast_component, read_oracle_forecast,
    write_oracle_forecast,
};

#[path = "fsv_support.rs"]
mod support;
use serde_json::json;
use support::{named_fsv_root, reset_dir, write_json};

const DOMAIN: &str = "crypto";
const ACTION: &str = "btc_up_action";

fn yes() -> AnchorValue {
    AnchorValue::Text("YES".to_string())
}

#[test]
fn issue215_oracle_forecast_component_fsv() {
    let (root, keep) = named_fsv_root("POLY_ISSUE215_FSV_ROOT", "poly-issue215-oracle");
    reset_dir(&root);

    // ── Happy path: 20 unanimous YES observations → predict YES, p_yes well above 0.5. ──────────
    let vault = seeded_vault(&[Outcome::Yes; 20]);
    let forecast = produce_oracle_forecast_component(
        &vault,
        &action(ACTION),
        DomainId::from(DOMAIN),
        &yes(),
        &FixedClock::new(1_000),
    )
    .expect("happy oracle forecast");

    // Return-value expectations (known truth: unanimous YES).
    assert!(forecast.predicted_is_yes, "20 YES must predict YES");
    assert!(
        forecast.p_yes > 0.5,
        "unanimous YES must push p_yes above 0.5, got {}",
        forecast.p_yes
    );
    assert_eq!(forecast.component.kind, ComponentKind::Oracle);
    assert_eq!(
        forecast.recurrence_observations, 20,
        "recurrence observation count read from ledger CF must equal the 20 seeded rows"
    );
    assert!(
        forecast.self_consistency > 0.0,
        "reliability must be a positive measured weight"
    );
    assert_eq!(forecast.component.n_support, 20);

    // Full-state verification #1: persist and read back byte-identically.
    let path = write_oracle_forecast(&root, &forecast).expect("write oracle forecast");
    let readback = read_oracle_forecast(&path).expect("read oracle forecast");
    assert_eq!(
        readback, forecast,
        "persisted oracle forecast must round-trip exactly"
    );

    // Full-state verification #2: independently read the vault ledger CF (source of truth) and
    // confirm the recurrence observation count the component claims actually resides there.
    let ledger_obs = ledger_recurrence_observations(&vault, forecast.ledger_seq);
    assert_eq!(
        ledger_obs, 20,
        "vault ledger CF must record 20 recurrence observations at seq {}",
        forecast.ledger_seq
    );

    // ── Blend: oracle contributes alongside kNN + bits-vote (#215 integration). ────────────────
    let knn = calyx_poly::forecast::ForecastComponent::new(
        ComponentKind::KnnBaseRate,
        0.60,
        0.80,
        100,
        TrustTag::Trusted,
        "synthetic knn",
    )
    .unwrap();
    let bits = calyx_poly::forecast::ForecastComponent::new(
        ComponentKind::BitsVote,
        0.65,
        0.90,
        100,
        TrustTag::Trusted,
        "synthetic bits",
    )
    .unwrap();
    let blend_without =
        blend_components(&[knn.clone(), bits.clone()]).expect("blend without oracle");
    let blend_with =
        blend_components(&[knn.clone(), bits.clone(), forecast.component.clone()]).expect("blend");
    assert_eq!(
        blend_with.contributing, 3,
        "oracle must be a contributing blend component"
    );
    assert!(
        (blend_with.p_model - blend_without.p_model).abs() > 1e-9,
        "a reliable oracle component must move the blended p_model (with={} without={})",
        blend_with.p_model,
        blend_without.p_model
    );
    // Oracle (provisional guard by default) makes the whole blend provisional.
    assert_eq!(blend_with.trust, TrustTag::Provisional);

    // ── Edge 1: predominantly NO (15 NO / 5 YES) → predict NO, p_yes < 0.5. ────────────────────
    let vault_no = seeded_vault(&outcomes(5, 15));
    let no_forecast = produce_oracle_forecast_component(
        &vault_no,
        &action(ACTION),
        DomainId::from(DOMAIN),
        &yes(),
        &FixedClock::new(2_000),
    )
    .expect("no-leaning oracle forecast");
    assert!(!no_forecast.predicted_is_yes, "15 NO must predict NO");
    assert!(
        no_forecast.p_yes < 0.5,
        "NO-leaning oracle must put p_yes below 0.5, got {}",
        no_forecast.p_yes
    );
    assert_eq!(no_forecast.recurrence_observations, 20);

    // ── Edge 2: even 10/10 split → near-zero separation → p_yes ≈ 0.5. ─────────────────────────
    let vault_split = seeded_vault(&outcomes(10, 10));
    let split = produce_oracle_forecast_component(
        &vault_split,
        &action(ACTION),
        DomainId::from(DOMAIN),
        &yes(),
        &FixedClock::new(3_000),
    )
    .expect("split oracle forecast");
    assert!(
        (split.p_yes - 0.5).abs() < 0.05,
        "a 10/10 split must give ~coin-flip p_yes, got {}",
        split.p_yes
    );

    // ── Edge 3: no recurrence for the queried action → hard error, fail loud. ──────────────────
    let vault_happy = seeded_vault(&[Outcome::Yes; 5]);
    let err = produce_oracle_forecast_component(
        &vault_happy,
        &action("missing_action"),
        DomainId::from(DOMAIN),
        &yes(),
        &FixedClock::new(4_000),
    )
    .expect_err("missing action must fail closed");
    assert_eq!(
        err.code(),
        ERR_ORACLE_PREDICT,
        "no-recurrence must fail loud"
    );

    // ── Evidence log. ─────────────────────────────────────────────────────────────────────────
    let summary = json!({
        "issue": 215,
        "source_of_truth": [
            "oracle_forecast_*.json readback",
            "vault ledger CF recurrence_observations cross-check",
        ],
        "happy": evidence(&forecast, ledger_obs),
        "no_leaning": {"p_yes": no_forecast.p_yes, "predicted_is_yes": no_forecast.predicted_is_yes},
        "split": {"p_yes": split.p_yes},
        "blend": {
            "p_model_without_oracle": blend_without.p_model,
            "p_model_with_oracle": blend_with.p_model,
            "contributing": blend_with.contributing,
            "trust": format!("{:?}", blend_with.trust),
        },
        "no_recurrence_error": err.code(),
    });
    write_json(&root.join("summary.json"), &summary);
    println!(
        "issue215_fsv_summary={}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    if keep {
        println!("poly_issue215_fsv_root={}", root.display());
    }
}

fn evidence(f: &OracleForecast, ledger_obs: u64) -> serde_json::Value {
    json!({
        "predicted_is_yes": f.predicted_is_yes,
        "oracle_confidence": f.oracle_confidence,
        "p_yes": f.p_yes,
        "self_consistency": f.self_consistency,
        "recurrence_observations": f.recurrence_observations,
        "ledger_seq": f.ledger_seq,
        "ledger_cf_recurrence_observations": ledger_obs,
        "trust": format!("{:?}", f.trust),
        "n_support": f.component.n_support,
        "reliability": f.component.reliability,
    })
}

// ── Synthetic vault seeding (real engine, known-truth inputs) ──────────────────────────────────

#[derive(Clone, Copy)]
enum Outcome {
    Yes,
    No,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Yes => "YES",
            Outcome::No => "NO",
        }
    }
}

fn outcomes(yes_count: usize, no_count: usize) -> Vec<Outcome> {
    let mut v = vec![Outcome::Yes; yes_count];
    v.extend(std::iter::repeat(Outcome::No).take(no_count));
    v
}

fn seeded_vault(rows: &[Outcome]) -> AsterVault<FixedClock> {
    let vault = AsterVault::with_clock(vault_id(), b"issue215-salt", FixedClock::new(1));
    let panel = panel(&[1, 2]);
    put_sufficiency(&vault, &panel, 1.0, 0.8);
    seed_self_consistency(&vault, DOMAIN);
    // One constellation carrying all observations for the action.
    let cx_id = CxId::from_bytes([200u8; 16]);
    write_base(&vault, cx_id, DOMAIN, ACTION);
    for (idx, outcome) in rows.iter().enumerate() {
        write_occurrence(&vault, cx_id, idx, ACTION, outcome.label());
    }
    vault
}

fn write_base(vault: &AsterVault<FixedClock>, cx_id: CxId, domain: &str, action_id: &str) {
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&constellation(cx_id, domain, action_id))
                .expect("encode base"),
        )
        .expect("write base");
}

fn write_occurrence(
    vault: &AsterVault<FixedClock>,
    cx_id: CxId,
    occ_idx: usize,
    action_id: &str,
    outcome: &str,
) {
    let context = json!({
        "action": action_id,
        "oracle_verdict": { "value": { "text": outcome } },
        "outcome_anchor": { "value": { "text": outcome } }
    });
    let occurrence = Occurrence {
        id: OccurrenceId(occ_idx as u64),
        t_k: EpochSecs(1_000 + occ_idx as i64),
        context: OccurrenceContext::new(serde_json::to_vec(&context).unwrap()).expect("context"),
    };
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, occ_idx as u64),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence)).expect("encode"),
        )
        .expect("write recurrence");
}

/// Seeds a self-consistency calibration series so `oracle_self_consistency` yields a positive,
/// bounded reliability (the pattern mirrors calyx-oracle's own predict fixtures).
fn seed_self_consistency(vault: &AsterVault<FixedClock>, domain: &str) {
    let mut seed = 1u8;
    let add = |vault: &AsterVault<FixedClock>, seed: u8, outcomes: &[&str]| {
        let cx_id = CxId::from_bytes([seed; 16]);
        write_base(vault, cx_id, domain, "calibration");
        for (idx, outcome) in outcomes.iter().enumerate() {
            let context = json!({
                "action": "calibration",
                "oracle_verdict": { "value": { "text": outcome } },
                "outcome_anchor": { "value": { "text": outcome } },
                "ground_truth_anchor": { "value": { "text": outcome } }
            });
            let occurrence = Occurrence {
                id: OccurrenceId(idx as u64),
                t_k: EpochSecs(1_000 + idx as i64),
                context: OccurrenceContext::new(serde_json::to_vec(&context).unwrap())
                    .expect("context"),
            };
            vault
                .write_cf(
                    ColumnFamily::Recurrence,
                    recurrence_key(cx_id, idx as u64),
                    encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                        .expect("encode"),
                )
                .expect("write recurrence");
        }
    };
    // ≥ MIN_ASSAY_SAMPLES (50) ground-truth samples with verdict == ground_truth (so validity uses
    // the exact all-agree path = 1.0) and mixed pass/fail across series (so flakiness > 0): the
    // proven calyx-oracle "point-0.95" calibration shape. 6+3+2+2+37 = 50 samples.
    add(vault, seed, &["pass"; 6]);
    seed += 1;
    add(vault, seed, &["pass"; 3]);
    seed += 1;
    add(vault, seed, &["pass"; 2]);
    seed += 1;
    add(vault, seed, &["pass", "fail"]);
    seed += 1;
    for idx in 0..37u8 {
        let outcome = if idx % 2 == 0 { "pass" } else { "fail" };
        add(vault, seed + idx, &[outcome]);
    }
}

fn put_sufficiency(
    vault: &AsterVault<FixedClock>,
    panel: &Panel,
    panel_bits: f32,
    entropy_bits: f32,
) {
    let key = AssayCacheKey::scoped(panel.version, DOMAIN, vault_id(), AnchorKind::Reward);
    let mut store = AssayStore::default();
    // The panel sufficiency estimate must carry a *passing* planted-signal power calibration
    // (recovery_ratio ≥ min) or the assay gate fails closed with CALYX_ASSAY_ESTIMATOR_UNDERPOWERED.
    let passing = PowerCalibration::new(1.0, 1.0, 0.5, 200, 4, 0).expect("passing calibration");
    store.put(
        key.clone(),
        AssaySubject::Panel,
        MiEstimate::point(
            panel_bits,
            120,
            EstimatorKind::PanelSufficiency,
            TrustTag::Trusted,
        )
        .with_power_calibration(passing),
        "oracle predict panel bits",
        1,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        MiEstimate::point(
            entropy_bits,
            120,
            EstimatorKind::OutcomeEntropy,
            TrustTag::Trusted,
        ),
        "oracle predict entropy",
        1,
    );
    for slot in &panel.slots {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: slot.slot_id },
            MiEstimate::point(
                panel_bits / panel.slots.len().max(1) as f32,
                120,
                EstimatorKind::Ksg,
                TrustTag::Trusted,
            ),
            "oracle predict lens bits",
            1,
        );
    }
    store.persist_to_vault(vault).expect("persist assay");
}

fn action(action_id: &str) -> Action {
    Action {
        action_id: action_id.to_string(),
        panel: panel(&[1, 2]),
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
        axis: Some("issue215-oracle-fixture".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 432,
    }
}

fn constellation(cx_id: CxId, domain: &str, action_id: &str) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(
        ORACLE_ACTION_METADATA_KEY.to_string(),
        action_id.to_string(),
    );
    Constellation {
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

fn ledger_recurrence_observations(vault: &AsterVault<FixedClock>, seq: u64) -> u64 {
    let bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger")
        .expect("ledger row present");
    let entry = calyx_ledger::decode(&bytes).expect("decode ledger");
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).expect("payload json");
    payload["recurrence_observations"]
        .as_u64()
        .expect("recurrence_observations")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
