use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use calyx_anneal::{
    AnchorId, AssayAttribution, CandidateLens, HotAddPlan, HotAddReceipt, LensHotAdder,
    LensProfiler, PairNMI, RegistryHotAdder,
};
use calyx_core::{Constellation, CxId, LensId, Modality, Panel, Result, SlotState};
use calyx_registry::{CapabilityCard, Registry, SwapController};

use super::core::{cosine, dense};
use super::model::BitsOut;
use super::propose_backfill::CandidateBackfill;
use super::propose_profile::{
    ProfileMeasurement, capability_card, hot_add_fail, invalid_metric, measure_candidate,
    measure_registered_lens, measured_bits, measured_cost, observed_modalities, per_sensor_bits,
};

#[derive(Default)]
pub(super) struct LiveProposalState {
    after_sufficiency: Cell<Option<f64>>,
    backfill: RefCell<Option<CandidateBackfill>>,
    profiles: RefCell<BTreeMap<LensId, ProfileMeasurement>>,
}

impl LiveProposalState {
    pub(super) fn take_backfill(&self) -> Option<CandidateBackfill> {
        self.backfill.borrow_mut().take()
    }
}

pub(super) struct LiveAssay {
    before_bits: f64,
    entropy_bits: f64,
    per_sensor_bits: Vec<(LensId, f64)>,
    expected_modalities: Vec<Modality>,
    lens_modalities: BTreeMap<LensId, Modality>,
}

impl LiveAssay {
    pub(super) fn new(
        panel: &Panel,
        docs: &BTreeMap<CxId, Constellation>,
        anchor: &calyx_core::AnchorKind,
        measured: &BitsOut,
    ) -> Self {
        let before_bits = measured_bits(measured).min(measured.dpi_ceiling);
        Self {
            before_bits,
            entropy_bits: measured.dpi_ceiling,
            per_sensor_bits: per_sensor_bits(panel, measured),
            expected_modalities: observed_modalities(docs, anchor),
            lens_modalities: panel
                .slots
                .iter()
                .filter(|slot| slot.state == SlotState::Active)
                .map(|slot| (slot.lens_id, slot.modality))
                .collect(),
        }
    }

    pub(super) fn deficit_gap(&self) -> f64 {
        (self.entropy_bits - self.before_bits).max(0.0)
    }

    pub(super) fn before_bits(&self) -> f64 {
        self.before_bits
    }

    pub(super) fn entropy_bits(&self) -> f64 {
        self.entropy_bits
    }
}

pub(super) struct LiveAssayView<'a> {
    assay: &'a LiveAssay,
    state: &'a LiveProposalState,
}

impl<'a> LiveAssayView<'a> {
    pub(super) fn new(assay: &'a LiveAssay, state: &'a LiveProposalState) -> Self {
        Self { assay, state }
    }
}

impl AssayAttribution for LiveAssayView<'_> {
    fn per_sensor_bits(&self, _anchor: &AnchorId) -> Result<Vec<(LensId, f64)>> {
        Ok(self.assay.per_sensor_bits.clone())
    }

    fn panel_sufficiency(&self, _anchor: &AnchorId) -> Result<f64> {
        Ok(self
            .state
            .after_sufficiency
            .get()
            .unwrap_or(self.assay.before_bits))
    }

    fn entropy(&self, _anchor: &AnchorId) -> Result<f64> {
        Ok(self.assay.entropy_bits)
    }

    fn expected_modalities(&self, _anchor: &AnchorId) -> Result<Vec<Modality>> {
        Ok(self.assay.expected_modalities.clone())
    }

    fn lens_modality(&self, lens: &LensId) -> Result<Option<Modality>> {
        Ok(self.assay.lens_modalities.get(lens).copied())
    }
}

pub(super) struct LiveProfiler<'a> {
    vault_dir: &'a Path,
    anchor: &'a calyx_core::AnchorKind,
    state: &'a LiveProposalState,
}

impl<'a> LiveProfiler<'a> {
    pub(super) fn new(
        vault_dir: &'a Path,
        anchor: &'a calyx_core::AnchorKind,
        state: &'a LiveProposalState,
    ) -> Self {
        Self {
            vault_dir,
            anchor,
            state,
        }
    }
}

impl LensProfiler for LiveProfiler<'_> {
    fn profile(
        &self,
        candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> Result<CapabilityCard> {
        let started = Instant::now();
        let measured = measure_candidate(self.vault_dir, self.anchor, candidate, corpus_sample)?;
        let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
        let cost = measured
            .cost
            .unwrap_or_else(|| measured_cost(elapsed_ms, corpus_sample, &measured.vectors));
        let card = capability_card(
            measured.lens_id,
            measured.bits,
            corpus_sample.len(),
            cost,
            measured.signal_kind,
        );
        self.state
            .profiles
            .borrow_mut()
            .insert(measured.lens_id, measured);
        Ok(card)
    }
}

pub(super) struct LivePairNmi<'a> {
    panel: &'a Panel,
    state: &'a LiveProposalState,
}

impl<'a> LivePairNmi<'a> {
    pub(super) fn new(panel: &'a Panel, state: &'a LiveProposalState) -> Self {
        Self { panel, state }
    }
}

impl PairNMI for LivePairNmi<'_> {
    fn lens_embeddings(
        &self,
        lens: &LensId,
        corpus_sample: &[Constellation],
    ) -> Result<Vec<Vec<f32>>> {
        let Some(slot_id) = self.panel.slots.iter().find_map(|slot| {
            (slot.lens_id == *lens && slot.state == SlotState::Active).then_some(slot.slot_id)
        }) else {
            return Ok(Vec::new());
        };
        Ok(corpus_sample
            .iter()
            .filter_map(|cx| dense(cx, slot_id).map(<[f32]>::to_vec))
            .collect())
    }

    fn nmi(&self, lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> Result<f64> {
        let profiles = self.state.profiles.borrow();
        let Some(candidate) = profiles.get(lens_a) else {
            return Err(invalid_metric(format!(
                "candidate profile for lens {lens_a} was not measured"
            )));
        };
        let mut total = 0.0;
        let mut count = 0usize;
        for (left, right) in candidate.ordered.iter().zip(lens_b_embeddings) {
            if let Some(corr) = cosine(left, right) {
                total += f64::from(corr.abs());
                count += 1;
            }
        }
        Ok(if count == 0 {
            0.0
        } else {
            total / count as f64
        })
    }
}

pub(super) struct LiveHotAdder<'a> {
    registry: &'a mut Registry,
    vault_dir: &'a Path,
    docs: &'a BTreeMap<CxId, Constellation>,
    anchor: &'a calyx_core::AnchorKind,
    before_bits: f64,
    entropy_bits: f64,
    state: &'a LiveProposalState,
}

impl<'a> LiveHotAdder<'a> {
    pub(super) fn new(
        registry: &'a mut Registry,
        vault_dir: &'a Path,
        docs: &'a BTreeMap<CxId, Constellation>,
        anchor: &'a calyx_core::AnchorKind,
        before_bits: f64,
        entropy_bits: f64,
        state: &'a LiveProposalState,
    ) -> Self {
        Self {
            registry,
            vault_dir,
            docs,
            anchor,
            before_bits,
            entropy_bits,
            state,
        }
    }
}

impl LensHotAdder for LiveHotAdder<'_> {
    fn plan_hot_add(
        &mut self,
        panel: &Panel,
        candidate: &CandidateLens,
        corpus: &[Constellation],
    ) -> Result<HotAddPlan> {
        RegistryHotAdder::new(self.registry).plan_hot_add(panel, candidate, corpus)
    }

    fn apply_hot_add(
        &mut self,
        controller: &mut SwapController,
        candidate: &CandidateLens,
        corpus: &[Constellation],
        now: u64,
    ) -> Result<HotAddReceipt> {
        let receipt = RegistryHotAdder::new(self.registry)
            .apply_hot_add(controller, candidate, corpus, now)?;
        let slot = controller
            .panel()
            .slots
            .iter()
            .find(|slot| slot.lens_id == receipt.lens_id && slot.state == SlotState::Active)
            .ok_or_else(|| hot_add_fail("hot-added slot is missing from the controller panel"))?;
        let measured = measure_registered_lens(
            self.registry,
            self.vault_dir,
            self.anchor,
            self.docs,
            slot.slot_id,
            slot.lens_id,
            slot.modality,
        )?;
        self.state.after_sufficiency.set(Some(
            (self.before_bits + measured.bits).min(self.entropy_bits),
        ));
        self.state.backfill.replace(Some(CandidateBackfill {
            slot_id: slot.slot_id,
            lens_id: slot.lens_id,
            bits: measured.bits,
            vectors: measured.vectors,
        }));
        Ok(receipt)
    }
}
