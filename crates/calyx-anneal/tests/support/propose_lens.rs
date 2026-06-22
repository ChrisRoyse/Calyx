use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use calyx_anneal::{
    ActionMetricSnapshot, AnchorId, AssayAttribution, CALYX_ASSAY_INVALID_METRIC,
    CALYX_REGISTRY_HOT_ADD_FAIL, CandidateLens, ChangeId, ChangeOutcome, HotAddAction, HotAddPlan,
    HotAddReceipt, LensHotAdder, LensProfiler, PairNMI, ProposalSubstrate, ShadowRevertReason,
    TripwireMetric,
};
use calyx_core::{
    Anchor, Asymmetry, CalyxError, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Result, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, CapabilitySignalKind, CostMetrics, CoverageMetrics,
    LensHealth, MetricSource, Registry, SeparationMetrics, SlotSpec, SpreadMetrics, SwapController,
};

#[allow(clippy::duplicate_mod)]
#[path = "../fsv_support/mod.rs"]
mod fsv_support;

pub const TEST_TS: u64 = 1_785_500_421;

pub struct TestSubstrate {
    pub outcome: ChangeOutcome,
    pub proposed: usize,
    pub rolled_back: Vec<ChangeId>,
}

impl TestSubstrate {
    pub fn promote(change_id: ChangeId) -> Self {
        Self {
            outcome: ChangeOutcome::Promoted(change_id),
            proposed: 0,
            rolled_back: Vec::new(),
        }
    }

    pub fn revert(change_id: ChangeId, reason: ShadowRevertReason) -> Self {
        Self {
            outcome: ChangeOutcome::Reverted { reason, change_id },
            proposed: 0,
            rolled_back: Vec::new(),
        }
    }
}

impl ProposalSubstrate for TestSubstrate {
    fn ensure_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _prior_ptr: calyx_anneal::ArtifactPtr,
    ) -> Result<()> {
        Ok(())
    }

    fn propose_hot_add(&mut self, _plan: &HotAddPlan) -> Result<ChangeOutcome> {
        self.proposed += 1;
        Ok(self.outcome.clone())
    }

    fn rollback_hot_add(&mut self, change_id: ChangeId) -> Result<()> {
        self.rolled_back.push(change_id);
        Ok(())
    }
}

pub enum HotAddMode {
    Succeed,
    FailAfterMutate,
}

pub struct TestHotAdder {
    mode: HotAddMode,
    pub apply_calls: usize,
}

impl TestHotAdder {
    pub fn succeed() -> Self {
        Self {
            mode: HotAddMode::Succeed,
            apply_calls: 0,
        }
    }

    pub fn fail_after_mutate() -> Self {
        Self {
            mode: HotAddMode::FailAfterMutate,
            apply_calls: 0,
        }
    }
}

impl LensHotAdder for TestHotAdder {
    fn plan_hot_add(
        &mut self,
        _panel: &Panel,
        _candidate: &CandidateLens,
        _corpus: &[Constellation],
    ) -> Result<HotAddPlan> {
        Ok(HotAddPlan {
            artifact_key: calyx_anneal::ArtifactKey::ConfigCache([0xAB; 32]),
            prior_ptr: calyx_anneal::ArtifactPtr::ConfigCacheKeyHash([0x11; 32]),
            candidate_ptr: calyx_anneal::ArtifactPtr::ConfigCacheKeyHash([0x22; 32]),
            candidate_action: HotAddAction::stable(),
            incumbent_action: HotAddAction::from_metrics(ActionMetricSnapshot::from_values([
                (TripwireMetric::RecallAtK, 0.94),
                (TripwireMetric::GuardFAR, 0.001),
                (TripwireMetric::GuardFRR, 0.001),
                (TripwireMetric::SearchP99, 55.0),
                (TripwireMetric::IngestP95, 85.0),
            ])),
            description: "test hot add".to_string(),
        })
    }

    fn apply_hot_add(
        &mut self,
        controller: &mut SwapController,
        _candidate: &CandidateLens,
        _corpus: &[Constellation],
        now: u64,
    ) -> Result<HotAddReceipt> {
        self.apply_calls += 1;
        let receipt = add_test_lens(controller, now)?;
        match self.mode {
            HotAddMode::Succeed => Ok(receipt),
            HotAddMode::FailAfterMutate => Err(CalyxError {
                code: CALYX_REGISTRY_HOT_ADD_FAIL,
                message: "injected registry hot-add failure".to_string(),
                remediation: "repair registry hot-add path",
            }),
        }
    }
}

pub struct FixtureAssay {
    sufficiency: Mutex<VecDeque<f64>>,
    entropy: f64,
    expected: Vec<Modality>,
    lens_modalities: BTreeMap<LensId, Modality>,
}

impl FixtureAssay {
    pub fn new<const N: usize>(sufficiency: [f64; N], entropy: f64) -> Self {
        Self {
            sufficiency: Mutex::new(VecDeque::from(sufficiency)),
            entropy,
            expected: Vec::new(),
            lens_modalities: BTreeMap::from([(existing_lens(), Modality::Structured)]),
        }
    }

    pub fn with_expected_modalities(mut self, expected: Vec<Modality>) -> Self {
        self.expected = expected;
        self
    }
}

impl AssayAttribution for FixtureAssay {
    fn per_sensor_bits(&self, _anchor: &AnchorId) -> Result<Vec<(LensId, f64)>> {
        Ok(vec![(existing_lens(), 0.20)])
    }

    fn panel_sufficiency(&self, _anchor: &AnchorId) -> Result<f64> {
        Ok(self.sufficiency.lock().unwrap().pop_front().unwrap_or(0.0))
    }

    fn entropy(&self, _anchor: &AnchorId) -> Result<f64> {
        Ok(self.entropy)
    }

    fn expected_modalities(&self, _anchor: &AnchorId) -> Result<Vec<Modality>> {
        Ok(self.expected.clone())
    }

    fn lens_modality(&self, lens: &LensId) -> Result<Option<Modality>> {
        Ok(self.lens_modalities.get(lens).copied())
    }
}

pub struct StaticProfiler {
    bits: f32,
    cost: CostMetrics,
    signal_kind: CapabilitySignalKind,
}

impl StaticProfiler {
    pub fn new(bits: f32) -> Self {
        Self {
            bits,
            cost: default_cost(),
            signal_kind: CapabilitySignalKind::LearnedEncoder,
        }
    }

    pub fn with_cost(mut self, cost: CostMetrics) -> Self {
        self.cost = cost;
        self
    }

    pub fn with_signal_kind(mut self, signal_kind: CapabilitySignalKind) -> Self {
        self.signal_kind = signal_kind;
        self
    }
}

impl LensProfiler for StaticProfiler {
    fn profile(
        &self,
        _candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> Result<calyx_registry::CapabilityCard> {
        Ok(card(
            LensId::from_bytes([0xC8; 16]),
            self.bits,
            corpus_sample.len(),
            self.cost,
            self.signal_kind,
        ))
    }
}

pub struct StaticNmi {
    corr: f64,
}

impl StaticNmi {
    pub fn new(corr: f64) -> Self {
        Self { corr }
    }
}

impl PairNMI for StaticNmi {
    fn lens_embeddings(
        &self,
        _lens: &LensId,
        _corpus_sample: &[Constellation],
    ) -> Result<Vec<Vec<f32>>> {
        Ok(vec![vec![self.corr as f32]])
    }

    fn nmi(&self, _lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> Result<f64> {
        lens_b_embeddings
            .first()
            .and_then(|row| row.first())
            .copied()
            .map(f64::from)
            .ok_or_else(|| CalyxError {
                code: CALYX_ASSAY_INVALID_METRIC,
                message: "empty test NMI embeddings".to_string(),
                remediation: "repair test fixture",
            })
    }
}

pub fn controller() -> SwapController {
    SwapController::new(Panel {
        version: 1,
        slots: vec![slot(0, existing_lens(), "base")],
        created_at: TEST_TS,
        kernel_ref: None,
        guard_ref: None,
    })
}

pub fn corpus() -> Vec<Constellation> {
    vec![Constellation {
        cx_id: CxId::from_bytes([1; 16]),
        vault_id: fsv_support::vault_id(),
        panel_version: 1,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [9; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::from([("x".to_string(), 1.0)]),
        metadata: BTreeMap::new(),
        anchors: Vec::<Anchor>::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }]
}

pub fn anchor() -> AnchorId {
    AnchorId::new("quality").unwrap()
}

fn add_test_lens(controller: &mut SwapController, now: u64) -> Result<HotAddReceipt> {
    let mut registry = Registry::new();
    let name = format!("proposal-test-{}", controller.panel().slots.len());
    let lens = AlgorithmicLens::scalar(&name, Modality::Structured);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    registry.register_frozen(lens, contract)?;
    let outcome = controller.add_lens(
        &registry,
        SlotSpec {
            key: name,
            lens_id,
            shape: SlotShape::Dense(1),
            modality: Modality::Structured,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            axis: Some("proposal".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
        },
        std::iter::empty::<BackfillCandidate>(),
        now,
    )?;
    Ok(HotAddReceipt {
        lens_id: outcome.slot.lens_id,
        panel_version: outcome.panel_version,
        slot_count: controller.panel().slots.len(),
    })
}

fn slot(slot: u16, lens_id: LensId, key: &str) -> Slot {
    let slot_id = SlotId::new(slot);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, key),
        lens_id,
        shape: SlotShape::Dense(1),
        modality: Modality::Structured,
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

fn card(
    lens_id: LensId,
    bits: f32,
    probe_count: usize,
    cost: CostMetrics,
    signal_kind: CapabilitySignalKind,
) -> calyx_registry::CapabilityCard {
    calyx_registry::CapabilityCard {
        lens_id,
        probe_count,
        signal: Some(bits),
        signal_source: MetricSource::AssayStore,
        signal_kind,
        signal_reliability: None,
        proxy_signal: bits,
        differentiation: None,
        differentiation_source: MetricSource::AssayPending,
        proxy_differentiation: 0.0,
        spread: SpreadMetrics {
            participation_ratio: 1.0,
            normalized_participation_ratio: 1.0,
            stable_rank: 1.0,
            total_variance: 1.0,
            mean_pairwise_distance: 1.0,
        },
        separation: SeparationMetrics {
            score: bits,
            silhouette: bits,
            mean_pairwise_distance: 1.0,
            labeled_groups: 2,
            used_labels: true,
        },
        cost,
        coverage: CoverageMetrics {
            requested: probe_count,
            measured: probe_count,
            failed: 0,
            rate: 1.0,
        },
        health: LensHealth::Loaded,
        low_spread: false,
    }
}

fn default_cost() -> CostMetrics {
    CostMetrics {
        total_ms: 1.0,
        ms_per_input: 1.0,
        vram_bytes: 0,
        ram_bytes: 0,
        batch_ceiling: 1_000,
    }
}

fn existing_lens() -> LensId {
    LensId::from_bytes([1; 16])
}
