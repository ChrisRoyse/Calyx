use std::collections::BTreeMap;

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, DeficitRoutingContext, EstimatorKind, MiEstimate,
    PanelSufficiency, TrustTag, panel_sufficiency_with_context, per_sensor_attribution,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    AnchorKind, Asymmetry, CalyxError, Clock, FixedClock, LensId, Modality, Panel, QuantPolicy,
    Slot, SlotId, SlotKey, SlotShape, SlotState, VaultId,
};
use proptest::prelude::*;

use super::{SufficiencyAssay, check_sufficiency, check_sufficiency_with_assay};
use crate::{CALYX_ORACLE_INSUFFICIENT, DomainId, OracleError};

const DOMAIN: &str = "swe_bench_lite_form_only";

#[test]
fn insufficient_panel_returns_oracle_error_with_sensor_deficit() {
    let vault = vault();
    let panel = panel(&[1, 2]);
    put_evidence(
        &vault,
        &panel,
        0.46,
        1.0,
        &[(SlotId::new(1), 0.04), (SlotId::new(2), 0.42)],
    );

    let error = check_sufficiency(
        &vault,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(100),
    )
    .expect_err("insufficient panel must refuse");

    assert_eq!(error.code(), CALYX_ORACLE_INSUFFICIENT);
    let bound = insufficient_bound(error);
    assert_close(bound.i_panel_oracle, 0.46);
    assert_eq!(bound.dpi_ceiling, bound.i_panel_oracle);
    assert!(!bound.sufficient);
    assert!(!bound.per_sensor_deficit.is_empty());
}

#[test]
fn sufficient_panel_returns_ok_bound() {
    let vault = vault();
    let panel = panel(&[1, 2]);
    put_evidence(
        &vault,
        &panel,
        1.05,
        1.0,
        &[(SlotId::new(1), 0.50), (SlotId::new(2), 0.55)],
    );

    let bound = check_sufficiency(
        &vault,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(101),
    )
    .expect("sufficient panel should pass");

    assert!(bound.sufficient);
    assert_eq!(bound.i_panel_oracle, 1.05);
    assert_eq!(bound.dpi_ceiling, 1.05);
    assert!(bound.per_sensor_deficit.is_empty());
}

#[test]
fn zero_lens_panel_is_insufficient_when_outcome_has_entropy() {
    let vault = vault();
    let panel = panel(&[]);
    put_evidence(&vault, &panel, 0.0, 1.0, &[]);

    let error = check_sufficiency(
        &vault,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(102),
    )
    .expect_err("empty panel cannot carry one outcome bit");

    assert_eq!(error.code(), CALYX_ORACLE_INSUFFICIENT);
    let bound = insufficient_bound(error);
    assert!(!bound.sufficient);
    assert!(bound.per_sensor_deficit.is_empty());
}

#[test]
fn deterministic_zero_entropy_outcome_is_sufficient() {
    let vault = vault();
    let panel = panel(&[]);
    put_evidence(&vault, &panel, 0.0, 0.0, &[]);

    let bound = check_sufficiency(
        &vault,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(103),
    )
    .expect("zero entropy outcome needs no panel bits");

    assert!(bound.sufficient);
    assert_eq!(bound.i_panel_oracle, 0.0);
    assert_eq!(bound.dpi_ceiling, 0.0);
}

#[test]
fn exact_equality_is_sufficient_boundary() {
    let vault = vault();
    let panel = panel(&[1]);
    put_evidence(&vault, &panel, 1.0, 1.0, &[(SlotId::new(1), 1.0)]);

    let bound = check_sufficiency(
        &vault,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(104),
    )
    .expect("I(panel;oracle) == H(outcome) is sufficient");

    assert!(bound.sufficient);
    assert_eq!(bound.dpi_ceiling, bound.i_panel_oracle);
}

#[test]
fn assay_failure_propagates_without_sufficient_bound() {
    let panel = panel(&[1]);
    let error = check_sufficiency_with_assay(
        &FailingAssay,
        &panel,
        DomainId::from(DOMAIN),
        &FixedClock::new(105),
    )
    .expect_err("assay failures must fail closed");

    assert_eq!(error.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(error.remediation().contains("anchor more outcomes"));
    assert!(!matches!(error, OracleError::Insufficient { .. }));
}

proptest! {
    #[test]
    fn boundary_logic_is_strict(panel_raw in 0_u16..=200, entropy_raw in 0_u16..=200) {
        let panel_bits = panel_raw as f32 / 100.0;
        let entropy_bits = entropy_raw as f32 / 100.0;
        let panel = panel(&[1]);
        let assay = StaticAssay(report(
            panel_bits,
            entropy_bits,
            &[(SlotId::new(1), panel_bits.max(0.01))],
        ));

        let result = check_sufficiency_with_assay(
            &assay,
            &panel,
            DomainId::from("prop"),
            &FixedClock::new(106),
        );

        if panel_bits < entropy_bits {
            let refused = matches!(result, Err(OracleError::Insufficient { .. }));
            prop_assert!(refused);
        } else {
            prop_assert!(result.expect("sufficient boundary").sufficient);
        }
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
        Err(CalyxError::assay_insufficient_samples("fixture assay failure").into())
    }
}

fn put_evidence(
    vault: &AsterVault<FixedClock>,
    panel: &Panel,
    panel_bits: f32,
    entropy_bits: f32,
    slot_bits: &[(SlotId, f32)],
) {
    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(panel.version, DOMAIN, vault_id(), AnchorKind::Reward);
    store.put(
        key.clone(),
        AssaySubject::Panel,
        estimate(panel_bits, EstimatorKind::PanelSufficiency),
        "oracle panel bits",
        1,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        estimate(entropy_bits, EstimatorKind::OutcomeEntropy),
        "oracle outcome entropy",
        1,
    );
    for (slot, bits) in slot_bits {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: *slot },
            estimate(*bits, EstimatorKind::Ksg),
            "oracle lens attribution",
            1,
        );
    }
    store.persist_to_vault(vault).expect("persist assay rows");
}

fn report(panel_bits: f32, entropy_bits: f32, slots: &[(SlotId, f32)]) -> PanelSufficiency {
    let attributions = per_sensor_attribution(slots, 0.10);
    panel_sufficiency_with_context(
        panel_bits,
        entropy_bits,
        &attributions,
        TrustTag::Trusted,
        DeficitRoutingContext {
            panel_id: "prop-panel".to_string(),
            anchor: AnchorKind::Reward,
            computed_at_seq: 1,
            observation_scope: None,
        },
    )
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 120, estimator, TrustTag::Trusted)
}

fn panel(slots: &[u16]) -> Panel {
    Panel {
        version: 431,
        slots: slots.iter().copied().map(slot).collect(),
        created_at: 1_785_500_000,
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
        axis: Some("oracle-fixture".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 431,
    }
}

fn insufficient_bound(error: OracleError) -> crate::SufficiencyBound {
    match error {
        OracleError::Insufficient { bound } => bound,
        other => panic!("expected insufficient error, got {other}"),
    }
}

fn vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"issue431-salt", FixedClock::new(1))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-6);
}
