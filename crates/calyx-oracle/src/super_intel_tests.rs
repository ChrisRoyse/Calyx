use std::collections::BTreeMap;

use calyx_assay::{
    DeficitRoutingContext, PanelSufficiency, TrustTag, panel_sufficiency_with_context,
    per_sensor_attribution,
};
use calyx_core::{
    AnchorKind, Asymmetry, CalyxError, Clock, CxId, FixedClock, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use calyx_lodestar::{
    GroundednessReport, InMemoryAnnIndex, InMemoryCorpus, Kernel, RecallQuery, RecallReport,
    RecallTestParams, build_kernel_index,
};
use proptest::prelude::*;

use super::*;
use crate::CALYX_ORACLE_NO_RECURRENCE;

#[test]
fn super_intel_tier_oracle_clean_passes_at_point_eight() {
    let tier = measure_tier_oracle_clean_with_source(
        &OracleSource::ok(0.0, 0.8, false),
        DomainId::from("ph50"),
        &FixedClock::new(1),
    );

    assert_eq!(tier.tier, Tier::OracleClean);
    assert!(tier.passed);
    assert_eq!(tier.measured_value, 0.8);
    assert_eq!(tier.threshold, ORACLE_CLEAN_THRESHOLD);
    assert!(tier.cheapest_fix.is_none());
}

#[test]
fn super_intel_tier_oracle_clean_fails_at_point_five() {
    let tier = measure_tier_oracle_clean_with_source(
        &OracleSource::ok(0.0, 0.5, false),
        DomainId::from("ph50"),
        &FixedClock::new(2),
    );

    assert!(!tier.passed);
    assert_eq!(tier.measured_value, 0.5);
    assert_eq!(
        tier.cheapest_fix.as_deref(),
        Some(ORACLE_FIX_VALIDITY_ANCHOR)
    );
}

#[test]
fn super_intel_tier_panel_sufficient_passes_at_known_bits() {
    let panel = panel(&[1, 2]);
    let assay = StaticAssay(report(1.05, 1.0, &[(SlotId::new(1), 0.5)]));
    let tier = measure_tier_panel_sufficient_with_assay(
        &assay,
        &panel,
        DomainId::from("ph50"),
        &FixedClock::new(3),
    );

    assert_eq!(tier.tier, Tier::PanelSufficient);
    assert!(tier.passed);
    assert_eq!(tier.measured_value, 1.05);
    assert_eq!(tier.threshold, 1.0);
}

#[test]
fn super_intel_tier_panel_sufficient_names_max_deficit_lens() {
    let panel = panel(&[1, 2]);
    let assay = StaticAssay(report(
        0.46,
        1.0,
        &[(SlotId::new(1), 0.30), (SlotId::new(2), 0.01)],
    ));
    let tier = measure_tier_panel_sufficient_with_assay(
        &assay,
        &panel,
        DomainId::from("ph50"),
        &FixedClock::new(4),
    );

    assert!(!tier.passed);
    assert_eq!(tier.measured_value, 0.46);
    assert_eq!(tier.threshold, 1.0);
    let fix = tier.cheapest_fix.expect("panel fix");
    assert!(fix.contains("outcome/execution-derived lens"));
    assert!(fix.contains(&LensId::from_bytes([2; 16]).to_string()));
}

#[test]
fn super_intel_tier_kernel_exists_passes_at_point_nine_six() {
    let tier = measure_tier_kernel_exists(
        &KernelSource::ratio(0.96, 10),
        DomainId::from("ph50"),
        &held_out(),
        &FixedClock::new(5),
    );

    assert_eq!(tier.tier, Tier::KernelExists);
    assert!(tier.passed);
    assert_eq!(tier.measured_value, 0.96);
    assert_eq!(tier.threshold, KERNEL_RECALL_RATIO);
}

#[test]
fn super_intel_tier_kernel_exists_fails_at_point_nine_three() {
    let tier = measure_tier_kernel_exists(
        &KernelSource::ratio(0.93, 10),
        DomainId::from("ph50"),
        &held_out(),
        &FixedClock::new(6),
    );

    assert!(!tier.passed);
    assert_eq!(tier.measured_value, 0.93);
    assert_eq!(tier.cheapest_fix.as_deref(), Some(KERNEL_FIX_ANCHORS));
}

#[test]
fn super_intel_tier_kernel_gate_uses_ph33_recall_api() {
    let ids = [cx(1), cx(2)];
    let kernel = kernel(&ids);
    let mut embeddings = BTreeMap::new();
    embeddings.insert(ids[0], vec![1.0, 0.0]);
    embeddings.insert(ids[1], vec![0.0, 1.0]);
    let kernel_index = build_kernel_index(&kernel, &embeddings).expect("kernel index");
    let rows = vec![
        RecallQuery {
            cx_id: ids[0],
            vector: vec![1.0, 0.0],
        },
        RecallQuery {
            cx_id: ids[1],
            vector: vec![0.0, 1.0],
        },
    ];
    let full = InMemoryAnnIndex::new(rows.clone()).expect("full index");
    let corpus = InMemoryCorpus::new("ph50-held-out", rows);
    let gate = KernelRecallGate::new(
        &kernel_index,
        &full,
        &corpus,
        RecallTestParams {
            held_out_fraction: 1.0,
            top_k: 1,
            rng_seed: 7,
            min_recall_ratio: 0.0,
        },
    );

    let tier = measure_tier_kernel_exists(
        &gate,
        DomainId::from("ph50"),
        &HeldOutSplit::new("held-out", vec![cx(9)], ids.to_vec()),
        &FixedClock::new(7),
    );

    assert!(tier.passed);
    assert_eq!(tier.measured_value, 1.0);
}

#[test]
fn super_intel_tier_kernel_empty_held_out_fails_closed() {
    let tier = measure_tier_kernel_exists(
        &KernelSource::ratio(1.0, 10),
        DomainId::from("ph50"),
        &HeldOutSplit::new("empty", vec![cx(1)], Vec::new()),
        &FixedClock::new(8),
    );

    assert!(!tier.passed);
    assert_eq!(tier.measured_value, 0.0);
    assert_eq!(tier.cheapest_fix.as_deref(), Some(KERNEL_FIX_HELD_OUT));
}

#[test]
fn super_intel_tier_kernel_overlapping_split_fails_closed() {
    let tier = measure_tier_kernel_exists(
        &KernelSource::ratio(1.0, 10),
        DomainId::from("ph50"),
        &HeldOutSplit::new("leak", vec![cx(1)], vec![cx(1)]),
        &FixedClock::new(9),
    );

    assert!(!tier.passed);
    assert!(
        tier.cheapest_fix
            .as_deref()
            .expect("leak fix")
            .contains("CALYX_RECALL_INVALID_PARAMS")
    );
}

#[test]
fn super_intel_tier_oracle_no_recurrence_preserves_code() {
    let tier = measure_tier_oracle_clean_with_source(
        &OracleSource::err(OracleError::NoRecurrence {
            domain: DomainId::from("missing"),
        }),
        DomainId::from("missing"),
        &FixedClock::new(10),
    );

    assert!(!tier.passed);
    assert!(
        tier.cheapest_fix
            .as_deref()
            .expect("oracle fix")
            .contains(CALYX_ORACLE_NO_RECURRENCE)
    );
}

#[test]
fn super_intel_tiers_domain_not_found_can_measure_all_false() {
    let oracle = OracleSource::err(OracleError::DomainNotFound);
    let assay = FailingAssay;
    let kernel = KernelSource::err(calyx_lodestar::LodestarError::RecallEmptyCorpus);
    let panel = panel(&[1]);
    let held_out = held_out();
    let clock = FixedClock::new(11);
    let report = measure_super_intelligence_tiers_1_to_3(TierMeasurementRequest {
        oracle: &oracle,
        assay: &assay,
        kernel: &kernel,
        panel: &panel,
        domain: DomainId::from("missing"),
        held_out: &held_out,
        clock: &clock,
        short_circuit: ShortCircuit::MeasureAll,
    });

    assert!(!report.overall);
    assert_eq!(report.tiers.len(), 3);
    assert!(report.tiers.iter().all(|tier| !tier.passed));
    assert_eq!(report.failing_tier, Some(Tier::OracleClean));
}

#[test]
fn super_intel_tiers_short_circuit_stops_after_tier_one() {
    let oracle = OracleSource::err(OracleError::DomainNotFound);
    let assay = FailingAssay;
    let kernel = KernelSource::ratio(1.0, 10);
    let panel = panel(&[1]);
    let held_out = held_out();
    let clock = FixedClock::new(12);
    let tiers = measure_tiers_1_to_3(TierMeasurementRequest {
        oracle: &oracle,
        assay: &assay,
        kernel: &kernel,
        panel: &panel,
        domain: DomainId::from("missing"),
        held_out: &held_out,
        clock: &clock,
        short_circuit: ShortCircuit::Enabled,
    });

    assert_eq!(tiers.len(), 1);
    assert_eq!(tiers[0].tier, Tier::OracleClean);
    assert!(!tiers[0].passed);
}

#[test]
fn super_intel_tier_query_error_fails_closed() {
    let tier = measure_tier_kernel_exists(
        &KernelSource::err(calyx_lodestar::LodestarError::RecallInvalidParams {
            detail: "fixture".to_string(),
        }),
        DomainId::from("ph50"),
        &held_out(),
        &FixedClock::new(13),
    );

    assert!(!tier.passed);
    assert!(
        tier.cheapest_fix
            .as_deref()
            .expect("kernel fix")
            .contains("CALYX_RECALL_INVALID_PARAMS")
    );
}

proptest! {
    #[test]
    fn super_intel_tier_passed_matches_measured_threshold(
        measured in 0.0f32..=2.0,
        threshold in 0.0f32..=2.0,
    ) {
        let tier = measured_tier(Tier::KernelExists, measured, threshold, || "fix".to_string());
        prop_assert_eq!(tier.passed, measured >= threshold);
    }
}

#[derive(Clone)]
struct OracleSource(Result<OracleSelfConsistency, OracleError>);

impl OracleSource {
    fn ok(flakiness: f32, validity: f32, provisional: bool) -> Self {
        Self(Ok(OracleSelfConsistency::with_provenance(
            flakiness,
            validity,
            provisional,
            None,
        )))
    }

    fn err(error: OracleError) -> Self {
        Self(Err(error))
    }
}

impl OracleConsistencySource for OracleSource {
    fn oracle_self_consistency(
        &self,
        _domain: DomainId,
        _clock: &dyn Clock,
    ) -> Result<OracleSelfConsistency, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone)]
struct StaticAssay(PanelSufficiency);

impl SufficiencyAssay for StaticAssay {
    fn panel_sufficiency(
        &self,
        _panel: &Panel,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        Ok(self.0.clone())
    }
}

struct FailingAssay;

impl SufficiencyAssay for FailingAssay {
    fn panel_sufficiency(
        &self,
        _panel: &Panel,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        Err(CalyxError::assay_insufficient_samples("fixture").into())
    }
}

#[derive(Clone)]
struct KernelSource(Result<RecallReport, calyx_lodestar::LodestarError>);

impl KernelSource {
    fn ratio(ratio: f32, n_queries_tested: usize) -> Self {
        Self(Ok(RecallReport {
            kernel_only: ratio,
            full: 1.0,
            ratio,
            n_queries_tested,
            recall_test_params: Some(RecallTestParams::default()),
            ..RecallReport::default()
        }))
    }

    fn err(error: calyx_lodestar::LodestarError) -> Self {
        Self(Err(error))
    }
}

impl KernelRecallSource for KernelSource {
    fn kernel_recall_report(
        &self,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, calyx_lodestar::LodestarError> {
        self.0.clone()
    }
}

fn report(panel_bits: f32, entropy_bits: f32, slots: &[(SlotId, f32)]) -> PanelSufficiency {
    panel_sufficiency_with_context(
        panel_bits,
        entropy_bits,
        &per_sensor_attribution(slots, 0.10),
        TrustTag::Trusted,
        DeficitRoutingContext {
            panel_id: "ph50-panel".to_string(),
            anchor: AnchorKind::Reward,
            computed_at_seq: 1,
            observation_scope: None,
        },
    )
}

fn held_out() -> HeldOutSplit {
    HeldOutSplit::new("held-out", vec![cx(1), cx(2)], vec![cx(3), cx(4)])
}

fn panel(slots: &[u16]) -> Panel {
    Panel {
        version: 50,
        slots: slots.iter().copied().map(slot).collect(),
        created_at: 1,
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
        axis: Some("ph50".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 50,
    }
}

fn kernel(members: &[CxId]) -> Kernel {
    Kernel {
        kernel_id: cx(200),
        panel_version: 50,
        anchor_kind: Some("reward".to_string()),
        corpus_shard_hash: [0; 32],
        members: members.to_vec(),
        kernel_graph: members.to_vec(),
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "ph50-test; trust=anchored".to_string(),
        warnings: Vec::new(),
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
