use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;

use calyx_anneal::{
    ActionMetricSnapshot, AnchorId, AssayAttribution, CALYX_ASSAY_INVALID_METRIC,
    CALYX_REGISTRY_HOT_ADD_FAIL, CandidateLens, ChangeId, ChangeOutcome, HotAddAction, HotAddPlan,
    HotAddReceipt, LensHotAdder, LensProfiler, PairNMI, ProposalSubstrate, ProposeLens,
    ProposeLensRequest, ShadowRevertReason, TripwireMetric,
};
use calyx_core::{
    Anchor, Asymmetry, CalyxError, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    LensId, Modality, Panel, QuantPolicy, Result as CalyxResult, Slot, SlotId, SlotKey, SlotShape,
    SlotState, VaultId,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, CapabilitySignalKind, CostMetrics, CoverageMetrics,
    LensHealth, MetricSource, Registry, SeparationMetrics, SlotSpec, SpreadMetrics, SwapController,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(crate) struct Fixture {
    anchor: String,
    entropy: MetricFixture,
    sufficiency: Vec<MetricFixture>,
    profile_bits: MetricFixture,
    #[serde(default = "default_profile_signal_kind")]
    profile_signal_kind: CapabilitySignalKind,
    corr: MetricFixture,
    #[serde(default = "default_clock")]
    clock_ts: u64,
    #[serde(default = "default_panel")]
    panel: Vec<LensId>,
    #[serde(default = "default_substrate")]
    substrate: SubstrateMode,
    #[serde(default = "default_hot_add")]
    hot_add: HotAddMode,
    #[serde(default = "default_corpus_rows")]
    corpus_rows: usize,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SubstrateMode {
    Promote,
    RevertBudget,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HotAddMode {
    Succeed,
    FailAfterMutate,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum MetricFixture {
    Number(f64),
    String(String),
}

impl MetricFixture {
    fn value(&self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::String(value) if value.eq_ignore_ascii_case("nan") => Ok(f64::NAN),
            Self::String(value) if value.eq_ignore_ascii_case("inf") => Ok(f64::INFINITY),
            Self::String(value) if value.eq_ignore_ascii_case("-inf") => Ok(f64::NEG_INFINITY),
            Self::String(value) => value
                .parse::<f64>()
                .map_err(|error| format!("{CALYX_ASSAY_INVALID_METRIC}: parse metric: {error}")),
        }
    }
}

pub(crate) fn execute_fixture(
    fixture_path: &Path,
    fixture: Fixture,
    fixture_bytes: &[u8],
) -> Result<serde_json::Value, String> {
    let anchor = AnchorId::new(fixture.anchor).map_err(format_calyx)?;
    let entropy = fixture.entropy.value()?;
    let sufficiency = fixture
        .sufficiency
        .iter()
        .map(MetricFixture::value)
        .collect::<Result<Vec<_>, _>>()?;
    let mut controller = controller(&fixture.panel, fixture.clock_ts);
    let before = panel_json(controller.panel());
    let clock = FixedClock::new(fixture.clock_ts);
    let assay = FixtureAssay::new(fixture.panel.clone(), sufficiency, entropy);
    let profiler = FixtureProfiler {
        bits: fixture.profile_bits.value()? as f32,
        signal_kind: fixture.profile_signal_kind,
    };
    let nmi = FixtureNmi {
        corr: fixture.corr.value()?,
    };
    let mut substrate = FixtureSubstrate::new(fixture.substrate, fixture.clock_ts);
    let mut hot_add = FixtureHotAdder::new(fixture.hot_add);
    let corpus = corpus(fixture.corpus_rows, fixture.clock_ts);
    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .map_err(format_calyx)?;
    Ok(json!({
        "source_of_truth": "fixture JSON bytes plus SwapController panel state after ProposeLens execution",
        "fixture_path": fixture_path.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(fixture_bytes).to_hex().to_string(),
        "trigger": "calyx anneal propose-lens-run --fixture",
        "before": before,
        "after": panel_json(controller.panel()),
        "outcome": outcome,
        "substrate": {
            "proposed": substrate.proposed,
            "rolled_back": substrate.rolled_back,
        },
        "hot_add_apply_calls": hot_add.apply_calls,
    }))
}

struct FixtureSubstrate {
    mode: SubstrateMode,
    change_id: ChangeId,
    proposed: usize,
    rolled_back: Vec<ChangeId>,
}

impl FixtureSubstrate {
    fn new(mode: SubstrateMode, ts: u64) -> Self {
        Self {
            mode,
            change_id: ChangeId(ts.saturating_mul(1_000_000).saturating_add(421)),
            proposed: 0,
            rolled_back: Vec::new(),
        }
    }
}

impl ProposalSubstrate for FixtureSubstrate {
    fn ensure_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _prior_ptr: calyx_anneal::ArtifactPtr,
    ) -> CalyxResult<()> {
        Ok(())
    }

    fn propose_hot_add(&mut self, _plan: &HotAddPlan) -> CalyxResult<ChangeOutcome> {
        self.proposed += 1;
        Ok(match self.mode {
            SubstrateMode::Promote => ChangeOutcome::Promoted(self.change_id),
            SubstrateMode::RevertBudget => ChangeOutcome::Reverted {
                reason: ShadowRevertReason::BudgetExhausted,
                change_id: self.change_id,
            },
        })
    }

    fn rollback_hot_add(&mut self, change_id: ChangeId) -> CalyxResult<()> {
        self.rolled_back.push(change_id);
        Ok(())
    }
}

struct FixtureHotAdder {
    mode: HotAddMode,
    apply_calls: usize,
}

impl FixtureHotAdder {
    fn new(mode: HotAddMode) -> Self {
        Self {
            mode,
            apply_calls: 0,
        }
    }
}

impl LensHotAdder for FixtureHotAdder {
    fn plan_hot_add(
        &mut self,
        _panel: &Panel,
        _candidate: &CandidateLens,
        _corpus: &[Constellation],
    ) -> CalyxResult<HotAddPlan> {
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
            description: "fixture hot add".to_string(),
        })
    }

    fn apply_hot_add(
        &mut self,
        controller: &mut SwapController,
        _candidate: &CandidateLens,
        _corpus: &[Constellation],
        now: u64,
    ) -> CalyxResult<HotAddReceipt> {
        self.apply_calls += 1;
        let receipt = add_test_lens(controller, now)?;
        match self.mode {
            HotAddMode::Succeed => Ok(receipt),
            HotAddMode::FailAfterMutate => Err(CalyxError {
                code: CALYX_REGISTRY_HOT_ADD_FAIL,
                message: "fixture registry hot-add failure".to_string(),
                remediation: "repair registry hot-add path",
            }),
        }
    }
}

struct FixtureAssay {
    panel: Vec<LensId>,
    sufficiency: Mutex<VecDeque<f64>>,
    entropy: f64,
}

impl FixtureAssay {
    fn new(panel: Vec<LensId>, sufficiency: Vec<f64>, entropy: f64) -> Self {
        Self {
            panel,
            sufficiency: Mutex::new(VecDeque::from(sufficiency)),
            entropy,
        }
    }
}

impl AssayAttribution for FixtureAssay {
    fn per_sensor_bits(&self, _anchor: &AnchorId) -> CalyxResult<Vec<(LensId, f64)>> {
        Ok(self.panel.iter().map(|lens| (*lens, 0.20)).collect())
    }

    fn panel_sufficiency(&self, _anchor: &AnchorId) -> CalyxResult<f64> {
        Ok(self.sufficiency.lock().unwrap().pop_front().unwrap_or(0.0))
    }

    fn entropy(&self, _anchor: &AnchorId) -> CalyxResult<f64> {
        Ok(self.entropy)
    }

    fn lens_modality(&self, _lens: &LensId) -> CalyxResult<Option<Modality>> {
        Ok(Some(Modality::Structured))
    }
}

struct FixtureProfiler {
    bits: f32,
    signal_kind: CapabilitySignalKind,
}

impl LensProfiler for FixtureProfiler {
    fn profile(
        &self,
        _candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> CalyxResult<calyx_registry::CapabilityCard> {
        Ok(card(
            LensId::from_bytes([0xC8; 16]),
            self.bits,
            corpus_sample.len(),
            self.signal_kind,
        ))
    }
}

struct FixtureNmi {
    corr: f64,
}

impl PairNMI for FixtureNmi {
    fn lens_embeddings(
        &self,
        _lens: &LensId,
        _corpus_sample: &[Constellation],
    ) -> CalyxResult<Vec<Vec<f32>>> {
        Ok(vec![vec![self.corr as f32]])
    }

    fn nmi(&self, _lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> CalyxResult<f64> {
        lens_b_embeddings
            .first()
            .and_then(|row| row.first())
            .copied()
            .map(f64::from)
            .ok_or_else(|| CalyxError {
                code: CALYX_ASSAY_INVALID_METRIC,
                message: "empty fixture NMI embeddings".to_string(),
                remediation: "repair fixture",
            })
    }
}

fn controller(lenses: &[LensId], created_at: u64) -> SwapController {
    SwapController::new(Panel {
        version: 1,
        slots: lenses
            .iter()
            .enumerate()
            .map(|(index, lens)| slot(index as u16, *lens, &format!("base_{index}")))
            .collect(),
        created_at,
        kernel_ref: None,
        guard_ref: None,
    })
}

fn add_test_lens(controller: &mut SwapController, now: u64) -> CalyxResult<HotAddReceipt> {
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

fn panel_json(panel: &Panel) -> serde_json::Value {
    json!({
        "version": panel.version,
        "slot_count": panel.slots.len(),
        "lenses": panel.slots.iter().map(|slot| slot.lens_id.to_string()).collect::<Vec<_>>(),
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

fn corpus(rows: usize, ts: u64) -> Vec<Constellation> {
    (0..rows)
        .map(|index| Constellation {
            cx_id: CxId::from_bytes([index as u8; 16]),
            vault_id: vault_id(),
            panel_version: 1,
            created_at: ts.saturating_add(index as u64),
            input_ref: InputRef {
                hash: [index as u8; 32],
                pointer: None,
                redacted: false,
            },
            modality: Modality::Structured,
            slots: BTreeMap::new(),
            scalars: BTreeMap::from([("x".to_string(), index as f64)]),
            metadata: BTreeMap::new(),
            anchors: Vec::<Anchor>::new(),
            provenance: LedgerRef {
                seq: index as u64,
                hash: [1; 32],
            },
            flags: CxFlags::default(),
        })
        .collect()
}

fn card(
    lens_id: LensId,
    bits: f32,
    probe_count: usize,
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
        cost: CostMetrics {
            total_ms: 1.0,
            ms_per_input: 1.0,
            vram_bytes: 0,
            ram_bytes: 0,
            batch_ceiling: 1_000,
        },
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

fn default_clock() -> u64 {
    1_785_500_421
}

fn default_panel() -> Vec<LensId> {
    vec![LensId::from_bytes([1; 16])]
}

fn default_substrate() -> SubstrateMode {
    SubstrateMode::Promote
}

fn default_hot_add() -> HotAddMode {
    HotAddMode::Succeed
}

fn default_profile_signal_kind() -> CapabilitySignalKind {
    CapabilitySignalKind::LearnedEncoder
}

fn default_corpus_rows() -> usize {
    1
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn format_calyx(error: CalyxError) -> String {
    format!("{}: {} ({})", error.code, error.message, error.remediation)
}
