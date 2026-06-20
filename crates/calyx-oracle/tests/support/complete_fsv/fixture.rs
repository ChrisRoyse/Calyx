use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_assay::{DeficitSuggestedAction, PanelSufficiency, SufficiencyDeficit, TrustTag};
use calyx_core::{
    AbsentReason, AnchorKind, Asymmetry, Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef,
    LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotShape, SlotState, SlotVector, VaultId,
};
use calyx_oracle::{
    AnnealConfig, CompletionLedger, CompletionLedgerPayload, CompletionRegion, CompletionResult,
    DomainId, OracleError, OracleSelfConsistency, SlotSet, SlotTag, SufficiencyAssay,
    complete_with_assay_and_region,
};
use serde::Serialize;

const SEED: u8 = 42;
const DIM: usize = 4;
const OUTPUT_PATH: &str = "/tmp/ph51_complete_fsv.json";

#[derive(Clone, Debug, Serialize)]
pub(super) struct FsvCase {
    test_name: String,
    cosine_similarities: Vec<f32>,
    tags: Vec<String>,
    slot_tags: Vec<TagReadback>,
    confidence: Option<f32>,
    converged: Option<bool>,
    error_code: Option<String>,
    descent_calls: Option<usize>,
    ledger_writes: usize,
}

#[derive(Clone, Debug, Serialize)]
struct TagReadback {
    lens_id: String,
    role: String,
    tag: String,
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
        let gaps = BTreeMap::from([(SlotId::new(4), 0.7), (SlotId::new(5), 0.3)]);
        Self {
            report: PanelSufficiency {
                panel_bits: 0.3,
                sufficiency_basis_bits: 0.3,
                anchor_entropy_bits: 1.0,
                sufficient: false,
                deficit_bits: 0.7,
                deficits: vec![SufficiencyDeficit {
                    panel_id: "ph51:fsv".to_string(),
                    anchor: AnchorKind::Reward,
                    slot: Some(SlotId::new(4)),
                    per_slot_gaps: gaps,
                    deficit_bits: 0.7,
                    suggested_action: DeficitSuggestedAction::ProposeLens,
                    computed_at_seq: 42,
                    observation_scope: None,
                    reason: "synthetic insufficient panel".to_string(),
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
pub(super) struct RegionFixture {
    members: BTreeMap<LensId, Vec<Vec<f32>>>,
    pub(super) calls: Cell<usize>,
}

impl RegionFixture {
    pub(super) fn from_full(panel: &Panel, full: &Constellation) -> Self {
        let mut members = BTreeMap::new();
        for slot in &panel.slots {
            let vector = full
                .slots
                .get(&slot.slot_id)
                .and_then(SlotVector::as_dense)
                .expect("dense full slot")
                .to_vec();
            members.insert(slot.lens_id, vec![vector]);
        }
        Self {
            members,
            calls: Cell::new(0),
        }
    }
}

impl CompletionRegion for RegionFixture {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.members.get(&lens_id).cloned().unwrap_or_default())
    }
}

#[derive(Default)]
pub(super) struct MemoryLedger {
    payloads: RefCell<Vec<CompletionLedgerPayload>>,
}

impl MemoryLedger {
    pub(super) fn writes(&self) -> usize {
        self.payloads.borrow().len()
    }
}

impl CompletionLedger for MemoryLedger {
    fn append_completion(
        &self,
        payload: CompletionLedgerPayload,
    ) -> Result<LedgerRef, OracleError> {
        let mut payloads = self.payloads.borrow_mut();
        payloads.push(payload);
        let seq = payloads.len() as u64;
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

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_ok(
    cx: &Constellation,
    panel: &Panel,
    clamp: &[u8],
    free: &[u8],
    region: &RegionFixture,
    ceiling: f32,
    clock: &dyn Clock,
) -> CompletionResult {
    complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::default(),
        cx,
        panel,
        DomainId::from("synthetic"),
        set(clamp),
        set(free),
        region,
        OracleSelfConsistency::measured(0.0, ceiling),
        &FixedAnneal,
        clock,
    )
    .expect("completion succeeds")
}

pub(super) fn case(
    name: &str,
    result: &CompletionResult,
    cosine_similarities: Vec<f32>,
) -> FsvCase {
    FsvCase {
        test_name: name.to_string(),
        cosine_similarities,
        tags: result
            .filled_cx
            .iter()
            .map(|slot| tag_name(slot.tag).to_string())
            .collect(),
        slot_tags: result
            .filled_cx
            .iter()
            .map(|slot| TagReadback {
                lens_id: slot.lens_id.to_string(),
                role: "filled".to_string(),
                tag: tag_name(slot.tag).to_string(),
            })
            .collect(),
        confidence: Some(result.confidence),
        converged: Some(result.converged),
        error_code: None,
        descent_calls: None,
        ledger_writes: result.provenance.seq as usize,
    }
}

pub(super) fn error_case(
    name: &str,
    code: &str,
    descent_calls: usize,
    ledger_writes: usize,
) -> FsvCase {
    FsvCase {
        test_name: name.to_string(),
        cosine_similarities: Vec::new(),
        tags: Vec::new(),
        slot_tags: Vec::new(),
        confidence: None,
        converged: None,
        error_code: Some(code.to_string()),
        descent_calls: Some(descent_calls),
        ledger_writes,
    }
}

pub(super) fn tag_scan_case(
    name: &str,
    results: [&CompletionResult; 3],
    partitions: [(&[u8], &[u8]); 3],
) -> FsvCase {
    let mut slot_tags = Vec::new();
    for (result, (clamp, free)) in results.into_iter().zip(partitions) {
        assert_tags(result, clamp, free);
        let clamped = set(clamp);
        for slot in &result.filled_cx {
            let role = if clamped.contains(&slot.lens_id) {
                "clamped"
            } else {
                "free"
            };
            slot_tags.push(TagReadback {
                lens_id: slot.lens_id.to_string(),
                role: role.to_string(),
                tag: tag_name(slot.tag).to_string(),
            });
        }
    }
    FsvCase {
        test_name: name.to_string(),
        cosine_similarities: Vec::new(),
        tags: slot_tags.iter().map(|tag| tag.tag.clone()).collect(),
        slot_tags,
        confidence: None,
        converged: Some(true),
        error_code: None,
        descent_calls: None,
        ledger_writes: 0,
    }
}

pub(super) fn assert_tags(result: &CompletionResult, clamp: &[u8], free: &[u8]) {
    let clamp = set(clamp);
    let free = set(free);
    for slot in &result.filled_cx {
        if clamp.contains(&slot.lens_id) {
            assert_eq!(slot.tag, SlotTag::Measured);
        }
        if free.contains(&slot.lens_id) {
            assert_eq!(slot.tag, SlotTag::Inferred);
        }
    }
}

pub(super) fn assert_full_copy(result: &CompletionResult, full: &Constellation, panel: &Panel) {
    for slot in &panel.slots {
        let measured = result
            .filled_cx
            .iter()
            .find(|item| item.lens_id == slot.lens_id)
            .expect("result slot");
        assert_eq!(measured.tag, SlotTag::Measured);
        assert_eq!(measured.vector, dense(full, slot.slot_id));
    }
}

pub(super) fn cosines_to_full(
    result: &CompletionResult,
    full: &Constellation,
    panel: &Panel,
    lens_indices: &[u8],
) -> Vec<f32> {
    lens_indices
        .iter()
        .map(|index| {
            let lens_id = lens(*index);
            let actual = result
                .filled_cx
                .iter()
                .find(|slot| slot.lens_id == lens_id)
                .expect("completed lens");
            let slot_id = panel
                .slots
                .iter()
                .find(|slot| slot.lens_id == lens_id)
                .expect("panel lens")
                .slot_id;
            cosine(&actual.vector, &dense(full, slot_id))
        })
        .collect()
}

pub(super) fn make_panel(count: u8) -> Panel {
    Panel {
        version: 1,
        slots: (1..=count).map(slot).collect(),
        created_at: 42,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(index: u8) -> Slot {
    Slot {
        slot_id: SlotId::new(index as u16),
        slot_key: SlotId::new(index as u16).with_key(format!("slot-{index}")),
        lens_id: lens(index),
        shape: SlotShape::Dense(DIM as u32),
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

pub(super) fn constellation(panel: &Panel, dense_indices: &[u8]) -> Constellation {
    let dense = dense_indices.iter().copied().collect::<BTreeSet<_>>();
    let mut slots = BTreeMap::new();
    for slot in &panel.slots {
        let index = slot.slot_id.get() as u8;
        let vector = if dense.contains(&index) {
            SlotVector::Dense {
                dim: DIM as u32,
                data: expected_vector(index),
            }
        } else {
            SlotVector::Absent {
                reason: AbsentReason::Deferred,
            }
        };
        slots.insert(slot.slot_id, vector);
    }
    Constellation {
        cx_id: CxId::from_bytes([panel.slots.len() as u8; 16]),
        vault_id: vault_id(),
        panel_version: panel.version,
        created_at: 42,
        input_ref: InputRef {
            hash: [SEED; 32],
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

fn dense(cx: &Constellation, slot_id: SlotId) -> Vec<f32> {
    cx.slots
        .get(&slot_id)
        .and_then(SlotVector::as_dense)
        .expect("dense slot")
        .to_vec()
}

pub(super) fn set(indices: &[u8]) -> SlotSet {
    indices.iter().map(|index| lens(*index)).collect()
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

fn expected_vector(index: u8) -> Vec<f32> {
    let mut vector = vec![0.0; DIM];
    vector[(usize::from(SEED) + usize::from(index)) % DIM] = 1.0;
    vector
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    dot / (left_norm * right_norm)
}

fn tag_name(tag: SlotTag) -> &'static str {
    match tag {
        SlotTag::Measured => "measured",
        SlotTag::Inferred => "inferred",
        SlotTag::Provisional => "provisional",
    }
}

pub(super) fn write_outputs(outputs: &[FsvCase]) {
    write_json(Path::new(OUTPUT_PATH), outputs);
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).expect("create fsv root");
        write_json(&root.join("ph51_complete_fsv.json"), outputs);
    }
}

fn write_json(path: &Path, value: &[FsvCase]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create output dir");
    }
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
