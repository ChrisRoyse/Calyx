use std::collections::BTreeMap;
use std::path::Path;

use calyx_anneal::{
    AlgParams, AlgorithmicKind, CALYX_ASSAY_INVALID_METRIC, CandidateLens, ConversionTarget,
    ExpectedTargetCost,
};
use calyx_core::{
    CalyxError, Constellation, CxId, Lens, LensId, Modality, Result, SlotId, SlotVector,
};
use calyx_registry::{
    AlgorithmicLens, CapabilityCard, CapabilitySignalKind, CostMetrics, CoverageMetrics,
    LensHealth, MetricSource, Registry, SeparationMetrics, SpreadMetrics,
};

use super::core::{cosine, dense, has_anchor, has_anchor_kind};
use super::model::BitsOut;
use super::propose_backfill::input_for_constellation;

const F32_BYTES: u64 = 4;

#[derive(Clone, Debug)]
pub(super) struct ProfileMeasurement {
    pub(super) lens_id: LensId,
    pub(super) bits: f64,
    pub(super) ordered: Vec<Vec<f32>>,
    pub(super) vectors: BTreeMap<CxId, SlotVector>,
    pub(super) cost: Option<CostMetrics>,
    pub(super) signal_kind: CapabilitySignalKind,
}

pub(super) fn measure_candidate(
    vault_dir: &Path,
    anchor: &calyx_core::AnchorKind,
    candidate: &CandidateLens,
    corpus: &[Constellation],
) -> Result<ProfileMeasurement> {
    match candidate {
        CandidateLens::Algorithmic { kind, params } => {
            let lens = algorithmic_lens(*kind, params);
            measure_with_lens(
                vault_dir,
                anchor,
                corpus,
                &lens,
                lens.modality(),
                None,
                CapabilitySignalKind::Algorithmic,
            )
        }
        CandidateLens::Commission { spec } => {
            let target = spec
                .suggested_targets
                .first()
                .ok_or_else(|| hot_add_fail("commissioned candidate has no ranked target"))?;
            measure_commission_target(vault_dir, anchor, corpus, target)
        }
    }
}

pub(super) fn measure_registered_lens(
    registry: &Registry,
    vault_dir: &Path,
    anchor: &calyx_core::AnchorKind,
    docs: &BTreeMap<CxId, Constellation>,
    slot_id: SlotId,
    lens_id: LensId,
    modality: Modality,
) -> Result<ProfileMeasurement> {
    let mut ordered = Vec::with_capacity(docs.len());
    let mut vectors = BTreeMap::new();
    for cx in docs.values() {
        let input = input_for_constellation(vault_dir, cx, modality)?;
        let vector = registry.measure(lens_id, &input)?;
        ordered.push(dense_projection(&vector)?);
        vectors.insert(cx.cx_id, vector);
    }
    Ok(ProfileMeasurement {
        lens_id,
        bits: bits_for_vectors(slot_id, docs, anchor, &vectors),
        ordered,
        vectors,
        cost: None,
        signal_kind: CapabilitySignalKind::Unknown,
    })
}

pub(super) fn measured_cost(
    elapsed_ms: f32,
    corpus: &[Constellation],
    vectors: &BTreeMap<CxId, SlotVector>,
) -> CostMetrics {
    let measured = vectors.len().max(1) as f32;
    CostMetrics {
        total_ms: elapsed_ms,
        ms_per_input: elapsed_ms / measured,
        vram_bytes: 0,
        ram_bytes: corpus_input_bytes(corpus).saturating_add(vector_bytes(vectors)),
        batch_ceiling: batch_ceiling(elapsed_ms / measured),
    }
}

pub(super) fn capability_card(
    lens_id: LensId,
    bits: f64,
    probe_count: usize,
    cost: CostMetrics,
    signal_kind: CapabilitySignalKind,
) -> CapabilityCard {
    let bits = bits.max(0.0).min(f64::from(f32::MAX)) as f32;
    CapabilityCard {
        lens_id,
        probe_count,
        signal: Some(bits),
        signal_source: MetricSource::AssayStore,
        signal_kind,
        signal_reliability: None,
        proxy_signal: bits,
        differentiation: Some(bits),
        differentiation_source: MetricSource::AssayStore,
        proxy_differentiation: bits,
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

pub(super) fn per_sensor_bits(panel: &calyx_core::Panel, measured: &BitsOut) -> Vec<(LensId, f64)> {
    measured
        .per_slot
        .iter()
        .filter_map(|slot_bits| {
            panel
                .slots
                .iter()
                .find(|slot| slot.slot_id.get() == slot_bits.slot)
                .map(|slot| (slot.lens_id, slot_bits.bits))
        })
        .collect()
}

pub(super) fn measured_bits(measured: &BitsOut) -> f64 {
    measured.per_slot.iter().map(|slot| slot.bits).sum()
}

pub(super) fn observed_modalities(
    docs: &BTreeMap<CxId, Constellation>,
    anchor: &calyx_core::AnchorKind,
) -> Vec<Modality> {
    let mut out = Vec::new();
    for cx in docs.values().filter(|cx| has_anchor_kind(cx, anchor)) {
        if !out.contains(&cx.modality) {
            out.push(cx.modality);
        }
    }
    out
}

pub(super) fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_METRIC,
        message: message.into(),
        remediation: "re-measure candidate capability before proposing a lens",
    }
}

pub(super) fn hot_add_fail(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: calyx_anneal::CALYX_REGISTRY_HOT_ADD_FAIL,
        message: message.into(),
        remediation: "repair registry hot-add and candidate measurement before admitting the lens",
    }
}

fn measure_with_lens(
    vault_dir: &Path,
    anchor: &calyx_core::AnchorKind,
    corpus: &[Constellation],
    lens: &dyn Lens,
    modality: Modality,
    cost: Option<CostMetrics>,
    signal_kind: CapabilitySignalKind,
) -> Result<ProfileMeasurement> {
    let mut ordered = Vec::with_capacity(corpus.len());
    let mut vectors = BTreeMap::new();
    for cx in corpus {
        let input = input_for_constellation(vault_dir, cx, modality)?;
        let vector = lens.measure(&input)?;
        ordered.push(dense_projection(&vector)?);
        vectors.insert(cx.cx_id, vector);
    }
    Ok(ProfileMeasurement {
        lens_id: lens.id(),
        bits: bits_for_dense(corpus, anchor, &ordered),
        ordered,
        vectors,
        cost,
        signal_kind,
    })
}

fn measure_commission_target(
    vault_dir: &Path,
    anchor: &calyx_core::AnchorKind,
    corpus: &[Constellation],
    target: &ConversionTarget,
) -> Result<ProfileMeasurement> {
    let lens_id = synthetic_lens_id(target);
    let mut ordered = Vec::with_capacity(corpus.len());
    let mut vectors = BTreeMap::new();
    for cx in corpus {
        let input = input_for_constellation(vault_dir, cx, target.modality)?;
        let data = synthetic_dense(target, lens_id, &input.bytes);
        ordered.push(data.clone());
        vectors.insert(
            cx.cx_id,
            SlotVector::Dense {
                dim: data.len() as u32,
                data,
            },
        );
    }
    Ok(ProfileMeasurement {
        lens_id,
        bits: target
            .expected_bits
            .max(bits_for_dense(corpus, anchor, &ordered)),
        ordered,
        vectors,
        cost: Some(target_cost(target.expected_cost, corpus.len())),
        signal_kind: CapabilitySignalKind::Placeholder,
    })
}

fn bits_for_vectors(
    slot: SlotId,
    docs: &BTreeMap<CxId, Constellation>,
    anchor: &calyx_core::AnchorKind,
    vectors: &BTreeMap<CxId, SlotVector>,
) -> f64 {
    let sample = docs
        .values()
        .map(|cx| {
            let mut cx = cx.clone();
            if let Some(vector) = vectors.get(&cx.cx_id) {
                cx.slots.insert(slot, vector.clone());
            }
            cx
        })
        .collect::<Vec<_>>();
    let ordered = sample
        .iter()
        .filter_map(|cx| dense(cx, slot).map(<[f32]>::to_vec))
        .collect::<Vec<_>>();
    bits_for_dense(&sample, anchor, &ordered)
}

fn bits_for_dense(
    corpus: &[Constellation],
    anchor: &calyx_core::AnchorKind,
    ordered: &[Vec<f32>],
) -> f64 {
    let observed = corpus
        .iter()
        .zip(ordered)
        .filter(|(cx, _)| has_anchor_kind(cx, anchor))
        .collect::<Vec<_>>();
    let positives = observed
        .iter()
        .filter(|(cx, _)| has_anchor(cx, anchor))
        .map(|(_, vector)| (*vector).clone())
        .collect::<Vec<_>>();
    let negatives = observed
        .iter()
        .filter(|(cx, _)| !has_anchor(cx, anchor))
        .map(|(_, vector)| (*vector).clone())
        .collect::<Vec<_>>();
    let comparisons = if negatives.is_empty() {
        corpus
            .iter()
            .zip(ordered)
            .filter(|(cx, _)| !has_anchor_kind(cx, anchor))
            .map(|(_, vector)| vector.clone())
            .collect::<Vec<_>>()
    } else {
        negatives
    };
    centroid_gap(&positives, &comparisons)
}

fn centroid_gap(positives: &[Vec<f32>], comparisons: &[Vec<f32>]) -> f64 {
    let (Some(pos), Some(neg)) = (centroid(positives), centroid(comparisons)) else {
        return 0.0;
    };
    cosine(&pos, &neg)
        .map(|corr| ((1.0 - f64::from(corr)) / 2.0).clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

fn centroid(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = vectors.first()?;
    let mut out = vec![0.0; first.len()];
    let mut count = 0usize;
    for vector in vectors {
        if vector.len() != out.len() {
            return None;
        }
        for (sum, value) in out.iter_mut().zip(vector) {
            *sum += *value;
        }
        count += 1;
    }
    for value in &mut out {
        *value /= count as f32;
    }
    Some(out)
}

fn dense_projection(vector: &SlotVector) -> Result<Vec<f32>> {
    vector
        .as_dense()
        .map(<[f32]>::to_vec)
        .ok_or_else(|| invalid_metric("candidate lens emitted a non-dense vector"))
}

fn algorithmic_lens(kind: AlgorithmicKind, params: &AlgParams) -> AlgorithmicLens {
    let name = format!("anneal-{}-{}", algorithmic_key(kind), params.seed);
    match kind {
        AlgorithmicKind::Tfidf => AlgorithmicLens::byte_features(name, Modality::Text),
        AlgorithmicKind::TimeLag
        | AlgorithmicKind::FrequencyBand
        | AlgorithmicKind::ValueDivergence
        | AlgorithmicKind::ExceptionValue
        | AlgorithmicKind::ControlFlow
        | AlgorithmicKind::Pca => AlgorithmicLens::scalar(name, Modality::Structured),
    }
}

fn algorithmic_key(kind: AlgorithmicKind) -> &'static str {
    match kind {
        AlgorithmicKind::Pca => "pca",
        AlgorithmicKind::TimeLag => "time_lag",
        AlgorithmicKind::FrequencyBand => "frequency_band",
        AlgorithmicKind::ValueDivergence => "value_divergence",
        AlgorithmicKind::ExceptionValue => "exception_value",
        AlgorithmicKind::ControlFlow => "control_flow",
        AlgorithmicKind::Tfidf => "tfidf",
    }
}

fn target_cost(cost: ExpectedTargetCost, n: usize) -> CostMetrics {
    CostMetrics {
        total_ms: cost.ms_per_input as f32 * n.max(1) as f32,
        ms_per_input: cost.ms_per_input as f32,
        vram_bytes: mib_to_bytes(cost.vram_mb),
        ram_bytes: mib_to_bytes(cost.ram_mb),
        batch_ceiling: batch_ceiling(cost.ms_per_input as f32),
    }
}

fn synthetic_lens_id(target: &ConversionTarget) -> LensId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(target.hf_id.as_bytes());
    hasher.update(target.axis.as_bytes());
    LensId::from_bytes(
        hasher.finalize().as_bytes()[..16]
            .try_into()
            .expect("hash prefix"),
    )
}

fn synthetic_dense(target: &ConversionTarget, lens_id: LensId, bytes: &[u8]) -> Vec<f32> {
    let dim = match target.modality {
        Modality::Text | Modality::Code | Modality::Structured | Modality::Mixed => 384,
        _ => 16,
    };
    let mut out = Vec::with_capacity(dim);
    let mut counter = 0_u32;
    while out.len() < dim {
        let mut hasher = blake3::Hasher::new();
        hasher.update(target.hf_id.as_bytes());
        hasher.update(lens_id.as_bytes());
        hasher.update(bytes);
        hasher.update(&counter.to_le_bytes());
        for chunk in hasher.finalize().as_bytes().chunks_exact(4) {
            let raw = u32::from_le_bytes(chunk.try_into().expect("hash chunk"));
            out.push((raw as f32 / u32::MAX as f32) * 2.0 - 1.0);
            if out.len() == dim {
                break;
            }
        }
        counter = counter.saturating_add(1);
    }
    out
}

fn corpus_input_bytes(corpus: &[Constellation]) -> u64 {
    corpus
        .iter()
        .map(|cx| {
            cx.input_ref
                .pointer
                .as_ref()
                .map_or(0, |value| value.len() as u64)
        })
        .sum()
}

fn vector_bytes(vectors: &BTreeMap<CxId, SlotVector>) -> u64 {
    vectors
        .values()
        .filter_map(SlotVector::as_dense)
        .map(|values| values.len() as u64 * F32_BYTES)
        .sum()
}

fn batch_ceiling(ms_per_input: f32) -> u32 {
    if !ms_per_input.is_finite() || ms_per_input <= f32::EPSILON {
        return u32::MAX;
    }
    (1000.0 / ms_per_input).floor().max(1.0) as u32
}

fn mib_to_bytes(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        (value * 1024.0 * 1024.0).round() as u64
    }
}
