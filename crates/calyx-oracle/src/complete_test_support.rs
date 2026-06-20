use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use calyx_assay::{DeficitSuggestedAction, PanelSufficiency, SufficiencyDeficit, TrustTag};
use calyx_core::{
    AbsentReason, AnchorKind, Asymmetry, Clock, Constellation, CxFlags, CxId, FixedClock, InputRef,
    LedgerRef, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotShape, SlotState,
    SlotVector, VaultId,
};

use super::*;

pub(super) fn run_complete(
    fixture: &Fixture,
    clamp: SlotSet,
    free: SlotSet,
    ceiling: f32,
) -> Result<CompletionResult, OracleError> {
    complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::default(),
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        clamp,
        free,
        &MapRegion::for_panel(&fixture.panel),
        OracleSelfConsistency::measured(0.0, ceiling),
        &FixedAnneal,
        &fixture.clock,
    )
}

pub(super) struct Fixture {
    pub panel: Panel,
    pub cx: Constellation,
    pub clock: FixedClock,
}

impl Fixture {
    pub(super) fn new(count: u8) -> Self {
        let panel = panel(count);
        let cx = constellation(&panel, absent_slots(&panel));
        Self {
            panel,
            cx,
            clock: FixedClock::new(1_785_500_000),
        }
    }

    pub(super) fn with_dense_slots(mut self) -> Self {
        let mut slots = BTreeMap::new();
        for slot in &self.panel.slots {
            slots.insert(
                slot.slot_id,
                SlotVector::Dense {
                    dim: 2,
                    data: expected_vector(slot.lens_id),
                },
            );
        }
        self.cx = constellation(&self.panel, slots);
        self
    }

    pub(super) fn with_slot(mut self, slot_index: u8, data: Vec<f32>) -> Self {
        self.cx.slots.insert(
            SlotId::new(slot_index as u16),
            SlotVector::Dense { dim: 2, data },
        );
        self
    }
}

#[derive(Clone)]
pub(super) struct FakeAssay {
    report: PanelSufficiency,
}

impl FakeAssay {
    pub(super) fn sufficient() -> Self {
        Self {
            report: PanelSufficiency {
                panel_bits: 2.0,
                sufficiency_basis_bits: 2.0,
                anchor_entropy_bits: 1.0,
                sufficient: true,
                deficit_bits: 0.0,
                deficits: Vec::new(),
                observation_scope: None,
                trust: TrustTag::Trusted,
                estimate_bound: calyx_assay::EstimateBound::LowerBound,
                power_calibration: None,
            },
        }
    }

    pub(super) fn insufficient() -> Self {
        let gaps = BTreeMap::from([(SlotId::new(1), 0.40), (SlotId::new(2), 0.35)]);
        Self {
            report: PanelSufficiency {
                panel_bits: 0.25,
                sufficiency_basis_bits: 0.25,
                anchor_entropy_bits: 1.0,
                sufficient: false,
                deficit_bits: 0.75,
                deficits: vec![SufficiencyDeficit {
                    panel_id: "panel:synthetic".to_string(),
                    anchor: AnchorKind::Reward,
                    slot: Some(SlotId::new(1)),
                    per_slot_gaps: gaps,
                    deficit_bits: 0.75,
                    suggested_action: DeficitSuggestedAction::ProposeLens,
                    computed_at_seq: 1,
                    observation_scope: None,
                    reason: "synthetic panel below anchor entropy".to_string(),
                }],
                observation_scope: None,
                trust: TrustTag::Provisional,
                estimate_bound: calyx_assay::EstimateBound::LowerBound,
                power_calibration: None,
            },
        }
    }
}

impl SufficiencyAssay for FakeAssay {
    fn panel_sufficiency(
        &self,
        _panel: &Panel,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        Ok(self.report.clone())
    }
}

#[derive(Default)]
pub(super) struct MapRegion {
    pub members: HashMap<LensId, Vec<Vec<f32>>>,
}

impl MapRegion {
    pub(super) fn for_panel(panel: &Panel) -> Self {
        let mut members = HashMap::new();
        for slot in &panel.slots {
            let expected = expected_vector(slot.lens_id);
            members.insert(slot.lens_id, vec![expected.clone(), expected]);
        }
        Self { members }
    }
}

impl CompletionRegion for MapRegion {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        Ok(self.members.get(&lens_id).cloned().unwrap_or_default())
    }
}

pub(super) struct PanicRegion;

impl CompletionRegion for PanicRegion {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        _lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        panic!("region must not be queried when sufficiency fails");
    }
}

#[derive(Default)]
pub(super) struct MemoryLedger {
    rows: Mutex<Vec<CompletionLedgerPayload>>,
    fail: bool,
}

impl MemoryLedger {
    pub(super) fn failing() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    pub(super) fn payloads(&self) -> Vec<CompletionLedgerPayload> {
        self.rows.lock().unwrap().clone()
    }
}

impl CompletionLedger for MemoryLedger {
    fn append_completion(
        &self,
        payload: CompletionLedgerPayload,
    ) -> Result<LedgerRef, OracleError> {
        if self.fail {
            return Err(OracleError::LedgerWriteFailure);
        }
        let mut rows = self.rows.lock().unwrap();
        rows.push(payload);
        let seq = rows.len() as u64;
        Ok(LedgerRef {
            seq,
            hash: [seq as u8; 32],
        })
    }
}

pub(super) struct FixedAnneal;

impl AnnealConfig for FixedAnneal {
    fn energy_beta(&self, _domain: &DomainId) -> Option<f32> {
        Some(1.0)
    }
}

fn panel(count: u8) -> Panel {
    Panel {
        version: 1,
        slots: (1..=count).map(slot).collect(),
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn absent_slots(panel: &Panel) -> BTreeMap<SlotId, SlotVector> {
    panel
        .slots
        .iter()
        .map(|slot| {
            (
                slot.slot_id,
                SlotVector::Absent {
                    reason: AbsentReason::Deferred,
                },
            )
        })
        .collect()
}

fn slot(index: u8) -> Slot {
    Slot {
        slot_id: SlotId::new(index as u16),
        slot_key: SlotId::new(index as u16).with_key(format!("slot-{index}")),
        lens_id: lens(index),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: None,
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn constellation(panel: &Panel, slots: BTreeMap<SlotId, SlotVector>) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([9; 16]),
        vault_id: vault_id(),
        panel_version: panel.version,
        created_at: 1,
        input_ref: InputRef {
            hash: [7; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

pub(super) fn set(indices: &[u8]) -> SlotSet {
    indices.iter().map(|index| lens(*index)).collect()
}

pub(super) fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

pub(super) fn expected_vector(lens_id: LensId) -> Vec<f32> {
    if lens_id.to_bytes()[0].is_multiple_of(2) {
        vec![0.0, 1.0]
    } else {
        vec![1.0, 0.0]
    }
}

pub(super) fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    dot / (left_norm * right_norm)
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
