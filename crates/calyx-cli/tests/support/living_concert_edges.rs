use std::collections::BTreeMap;

use calyx_anneal::{
    AsterAnnealLedgerStore, BudgetConfig, BudgetEnforcer, BudgetProbe, BudgetProbeSample,
};
use calyx_aster::cf::{ColumnFamily, OnlineKeyKind, base_key, online_key};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Clock, CxFlags, CxId, FixedClock, Lens, LensId, Modality,
    VaultId,
};
use calyx_ledger::{EntryKind, LedgerAppender, SubjectId};
use calyx_loom::recurrence::SeriesStore;
use calyx_oracle::predict_next_occurrence;
use calyx_registry::{AlgorithmicLens, Input, Registry};
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction,
    ProducedSlots, SlotCalibrationMeta, guard_with_ledger,
};
use serde_json::{Value, json};

use super::living_concert::{BASE_EVENT_SECS, GROWN_PANEL, START_TS, WEEK_SECS};
use super::living_concert_store::{
    append_event, constellation, ctx, online_row, slot, write_constellation, write_event_with_rows,
};

pub fn lens_unreachable_edge(vault: &AsterVault, registry: &Registry, input: &Input) -> Value {
    let missing = LensId::from_bytes([0xee; 16]);
    let error = registry.measure(missing, input).expect_err("missing lens");
    let row = online_row(
        50,
        json!({"tag":"lens_endpoint_killed_v1","error_code":error.code}),
    );
    write_event_with_rows(
        vault,
        EntryKind::Measure,
        SubjectId::Lens(missing),
        json!({"tag":"lens_endpoint_killed_v1","error_code":error.code}),
        vec![row],
        START_TS + 50,
    );
    json!({"error_code":error.code,"graceful_degradation":"slot_03 explicit absent"})
}

pub fn anneal_heal_edge(vault: &AsterVault) -> Value {
    let before = online_row(
        200,
        json!({"tag":"derived_index_state_v1","state":"degraded"}),
    );
    let after = online_row(
        201,
        json!({"tag":"derived_index_state_v1","state":"rebuilt","healed_by":"anneal"}),
    );
    write_event_with_rows(
        vault,
        EntryKind::Anneal,
        SubjectId::Query(b"issue641:anneal".to_vec()),
        json!({"tag":"living_anneal_heal_v1","transition":"degraded_to_rebuilt"}),
        vec![before, after],
        START_TS + 80,
    );
    json!({"before":"degraded","after":"rebuilt","ledger_kind":"anneal"})
}

pub fn oracle_step(vault: &AsterVault, cx_id: CxId) -> Value {
    let prediction = predict_next_occurrence(vault, cx_id, 0.91).expect("oracle prediction");
    append_event(
        vault,
        EntryKind::Answer,
        SubjectId::Cx(cx_id),
        json!({"tag":"living_oracle_prediction_v1","prediction":prediction}),
        START_TS + 90,
    );
    json!(prediction)
}

pub fn ward_step(vault: &AsterVault, cx_id: CxId) -> Value {
    let clock = FixedClock::new(START_TS + 100);
    let profile = guard_profile(&clock);
    let mut produced = ProducedSlots::new();
    produced.insert(slot(0), vec![1.0, 0.0]);
    let mut matched = MatchedSlots::new();
    matched.insert(slot(0), vec![0.0, 1.0]);
    let store = AsterAnnealLedgerStore::new(vault);
    let mut appender = LedgerAppender::open(store, clock).expect("open ward ledger");
    let (verdict, ledger_ref) =
        guard_with_ledger(&mut appender, cx_id, &profile, &produced, &matched, true)
            .expect("ward guard ledger");
    vault
        .write_cf(
            ColumnFamily::Online,
            online_key(OnlineKeyKind::HeadState, 300),
            serde_json::to_vec(&json!({"tag":"ward_injection_v1","verdict":verdict})).unwrap(),
        )
        .expect("write ward online");
    json!({"verdict": verdict, "ledger_ref": ledger_ref})
}

pub fn conflicting_anchor_edge(
    vault: &AsterVault,
    registry: &Registry,
    vault_id: VaultId,
) -> Value {
    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Base)
        .unwrap()
        .len();
    let a = conflict_constellation(vault, registry, vault_id, "support", true);
    let b = conflict_constellation(vault, registry, vault_id, "refute", false);
    let store = SeriesStore::new(vault);
    store
        .append_occurrence(a, EpochSecs(BASE_EVENT_SECS), ctx("support"))
        .unwrap();
    store
        .append_occurrence(b, EpochSecs(BASE_EVENT_SECS + WEEK_SECS), ctx("refute"))
        .unwrap();
    let after = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Base)
        .unwrap()
        .len();
    let row = online_row(
        400,
        json!({
            "tag":"conflicting_anchor_recurrence_v1",
            "same_content_signature": true,
            "anchor_values": ["support", "refute"],
            "cx_ids": [a.to_string(), b.to_string()],
            "merged": false,
            "base_rows_before": before,
            "base_rows_after": after,
        }),
    );
    write_event_with_rows(
        vault,
        EntryKind::Measure,
        SubjectId::Query(b"issue641:conflict".to_vec()),
        json!({"tag":"conflicting_anchor_recurrence_v1","merged":false}),
        vec![row],
        START_TS + 110,
    );
    json!({"cx_ids":[a.to_string(), b.to_string()],"merged":false,"base_rows_before":before,"base_rows_after":after})
}

pub fn budget_edge(vault: &AsterVault, cx_id: CxId) -> Value {
    let before = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))
        .unwrap()
        .is_some();
    let clock = FixedClock::new(START_TS + 120);
    let enforcer = BudgetEnforcer::with_probe(
        BudgetConfig {
            cpu_fraction: 0.0,
            vram_bytes: 1,
            tick_interval_ms: 1,
        },
        &clock,
        Probe,
    )
    .expect("budget enforcer");
    let error = match enforcer.acquire(0.01, 0) {
        Ok(_) => panic!("budget unexpectedly admitted background work"),
        Err(error) => error,
    };
    let after = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))
        .unwrap()
        .is_some();
    let row = online_row(
        500,
        json!({
            "tag":"over_budget_background_v1","error_code":error.code,
            "serving_read_before":before,"serving_read_after":after
        }),
    );
    write_event_with_rows(
        vault,
        EntryKind::Anneal,
        SubjectId::Query(b"issue641:budget".to_vec()),
        json!({"tag":"over_budget_background_v1","error_code":error.code}),
        vec![row],
        START_TS + 121,
    );
    json!({"error_code":error.code,"serving_read_before":before,"serving_read_after":after})
}

pub fn objective_rows(vault: &AsterVault, admission: &Value, prediction: &Value) -> Value {
    let start = 0.42_f64;
    let bits = admission["decision"]["signal_bits"].as_f64().unwrap_or(0.0);
    let confidence = prediction["confidence"].as_f64().unwrap_or(0.0);
    let end = start + bits + confidence * 0.01;
    let rows = vec![
        online_row(
            600,
            json!({"tag":"objective_j_v1","position":"start","j":start}),
        ),
        online_row(
            601,
            json!({"tag":"objective_j_v1","position":"end","j":end,"tripwires":"green"}),
        ),
    ];
    write_event_with_rows(
        vault,
        EntryKind::Admin,
        SubjectId::Query(b"issue641:j".to_vec()),
        json!({"tag":"living_objective_j_v1","start":start,"end":end}),
        rows,
        START_TS + 130,
    );
    json!({"start":start,"end":end,"non_decreasing":end >= start})
}

fn conflict_constellation(
    vault: &AsterVault,
    registry: &Registry,
    vault_id: VaultId,
    label: &str,
    value: bool,
) -> CxId {
    let input = Input::new(
        Modality::Text,
        format!("conflict shared signature {label}").into_bytes(),
    );
    let mut slots = BTreeMap::new();
    let byte_id = AlgorithmicLens::byte_features("issue641-byte-features", Modality::Text).id();
    slots.insert(
        slot(0),
        registry.measure(byte_id, &input).expect("byte lens"),
    );
    let cx_id = vault.cx_id_for_input(&input.bytes, GROWN_PANEL);
    let anchor = Anchor {
        kind: AnchorKind::Recurrence,
        value: AnchorValue::Bool(value),
        source: format!("issue641-conflict:{label}"),
        observed_at: START_TS,
        confidence: 1.0,
    };
    let cx = constellation(
        vault_id,
        cx_id,
        GROWN_PANEL,
        &input,
        slots,
        vec![anchor],
        CxFlags::default(),
    );
    write_constellation(
        vault,
        cx,
        json!({"tag":"conflict_ingest_v1","label":label}),
        START_TS + 105,
    );
    cx_id
}

fn guard_profile(clock: &dyn Clock) -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(0), 0.7);
    let mut calibration = CalibrationMeta::new([0x41; 32], "issue641-fixed", 0.0, 0.0, 0.99, clock);
    calibration
        .per_slot
        .insert(slot(0), SlotCalibrationMeta::from_calibration(&calibration));
    GuardProfile {
        guard_id: "018f48a4-9a79-74d2-8a5c-9ad7f6b8c641"
            .parse::<GuardId>()
            .unwrap(),
        panel_version: u64::from(GROWN_PANEL),
        domain: "issue641-living".to_string(),
        tau,
        required_slots: vec![slot(0)],
        policy: GuardPolicy::AllRequired,
        calibration: Some(calibration),
        novelty_action: NoveltyAction::NewRegion,
    }
}

struct Probe;

impl BudgetProbe for Probe {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        }
    }
}
