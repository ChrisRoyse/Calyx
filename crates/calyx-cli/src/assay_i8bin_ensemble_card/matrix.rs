use std::collections::BTreeMap;

use calyx_assay::{
    EnsembleCard, EnsembleLensInput, EnsembleRedundancyEvidence, EnsembleRedundancySketchInput,
    ensemble_redundancy_from_sketches, linear_cka_sketch_from_row_fn, linear_cka_tuple_plan,
};
use calyx_core::SlotId;
use calyx_sextant::index::DenseVectorFile;
use serde::Serialize;

use crate::assay_bits_validation::calyx_error_detail;

use super::plan::{LoadedPlan, PlanSlot};
use super::rows::{LabelRows, SampleRows, signature_indices};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MatrixReadout {
    pub(crate) signature_rows: usize,
    pub(crate) redundancy_metric: String,
    pub(crate) n_eff: f32,
    pub(crate) mean_pairwise_corr: f32,
    pub(crate) mean_pairwise_nmi: f32,
    pub(crate) correlation_matrix: Vec<Vec<f32>>,
    pub(crate) linear_cka_point_matrix: Vec<Vec<f32>>,
    pub(crate) linear_cka_mc_standard_error_matrix: Vec<Vec<f32>>,
    pub(crate) linear_cka_gate_score_matrix: Vec<Vec<f32>>,
    pub(crate) nmi_matrix: Vec<Vec<f32>>,
    pub(crate) pairs: Vec<PairReadout>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PairReadout {
    pub(crate) a: String,
    pub(crate) b: String,
    pub(crate) slot_a: u16,
    pub(crate) slot_b: u16,
    pub(crate) corr: f32,
    pub(crate) nmi: f32,
    pub(crate) raw_linear_cka: f32,
    pub(crate) linear_cka_point: f32,
    pub(crate) linear_cka_mc_standard_error: f32,
    pub(crate) linear_cka_gate_score: f32,
}

pub(crate) struct VectorReadout {
    pub(crate) lenses: Vec<EnsembleLensInput>,
    pub(crate) redundancy: EnsembleRedundancyEvidence,
    pub(crate) dims: Vec<usize>,
}

pub(crate) fn read_vectors(
    plan: &LoadedPlan,
    rows: &LabelRows,
    sample: &SampleRows,
    signature_row_limit: Option<usize>,
    nmi_bins: usize,
) -> Result<VectorReadout, String> {
    let total_rows = rows.labels.len();
    let signature_idx = signature_indices(total_rows, signature_row_limit);
    let tuple_plan = linear_cka_tuple_plan(signature_idx.len()).map_err(calyx_error_detail)?;
    let mut lenses = Vec::with_capacity(plan.slots.len());
    let mut sketches = Vec::with_capacity(plan.slots.len());
    let mut dims = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        let file = DenseVectorFile::open(&slot.corpus).map_err(calyx_error_detail)?;
        validate_file(slot, &file, total_rows)?;
        dims.push(file.dim());
        lenses.push(sample_lens(slot, &file, sample)?);
        let nmi_signature = row_signatures(&file, &signature_idx);
        let linear_cka = linear_cka_sketch_from_row_fn(&tuple_plan, file.dim(), |local_row| {
            file.row_f32(signature_idx[local_row])
        })
        .map_err(calyx_error_detail)?;
        sketches.push(EnsembleRedundancySketchInput::new(
            slot.name.clone(),
            SlotId::new(slot.slot),
            nmi_signature,
            linear_cka,
        ));
    }
    let redundancy = ensemble_redundancy_from_sketches(&tuple_plan, &sketches, nmi_bins)
        .map_err(calyx_error_detail)?;
    Ok(VectorReadout {
        lenses,
        redundancy,
        dims,
    })
}

impl MatrixReadout {
    pub(crate) fn from_card(card: &EnsembleCard) -> Result<Self, String> {
        let method = card.redundancy_method.as_ref().ok_or_else(|| {
            "CALYX_FSV_ASSAY_I8BIN_CARD_REDUNDANCY_MISSING: card has no redundancy method"
                .to_string()
        })?;
        let size = card.lenses.len();
        let expected_pairs = size.saturating_sub(1) * size / 2;
        if card.pairs.len() != expected_pairs {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_REDUNDANCY_MISSING: pairs {} != {expected_pairs}",
                card.pairs.len()
            ));
        }
        let positions = card
            .lenses
            .iter()
            .enumerate()
            .map(|(index, lens)| (lens.slot, index))
            .collect::<BTreeMap<_, _>>();
        let mut point_matrix = identity_matrix(size);
        let mut error_matrix = vec![vec![0.0; size]; size];
        let mut gate_matrix = identity_matrix(size);
        let mut nmi_matrix = identity_matrix(size);
        let mut pairs = Vec::with_capacity(card.pairs.len());
        for pair in &card.pairs {
            let estimate = pair.redundancy.as_ref().ok_or_else(|| {
                format!(
                    "CALYX_FSV_ASSAY_I8BIN_CARD_REDUNDANCY_MISSING: pair {}:{} lacks CKA evidence",
                    pair.slot_a, pair.slot_b
                )
            })?;
            if (pair.corr - estimate.mc_gate_upper_estimate).abs() > 1.0e-6 {
                return Err(format!(
                    "CALYX_FSV_ASSAY_I8BIN_CARD_REDUNDANCY_MISMATCH: pair {}:{} corr {} != gate {}",
                    pair.slot_a, pair.slot_b, pair.corr, estimate.mc_gate_upper_estimate
                ));
            }
            let (&a, &b) = positions
                .get(&pair.slot_a)
                .zip(positions.get(&pair.slot_b))
                .ok_or_else(|| {
                    "CALYX_FSV_ASSAY_I8BIN_CARD_REDUNDANCY_MISMATCH: pair slot absent from card"
                        .to_string()
                })?;
            set_symmetric(&mut point_matrix, a, b, estimate.redundancy_point);
            set_symmetric(&mut error_matrix, a, b, estimate.mc_standard_error);
            set_symmetric(&mut gate_matrix, a, b, estimate.mc_gate_upper_estimate);
            set_symmetric(&mut nmi_matrix, a, b, pair.nmi);
            pairs.push(PairReadout {
                a: pair.a.clone(),
                b: pair.b.clone(),
                slot_a: pair.slot_a.get(),
                slot_b: pair.slot_b.get(),
                corr: pair.corr,
                nmi: pair.nmi,
                raw_linear_cka: estimate.raw_signed_point,
                linear_cka_point: estimate.redundancy_point,
                linear_cka_mc_standard_error: estimate.mc_standard_error,
                linear_cka_gate_score: estimate.mc_gate_upper_estimate,
            });
        }
        Ok(Self {
            signature_rows: method.row_count,
            redundancy_metric: method.metric.clone(),
            n_eff: card.n_eff,
            mean_pairwise_corr: card.a37_diversity.mean_pairwise_corr,
            mean_pairwise_nmi: card.a37_diversity.mean_pairwise_nmi,
            correlation_matrix: gate_matrix.clone(),
            linear_cka_point_matrix: point_matrix,
            linear_cka_mc_standard_error_matrix: error_matrix,
            linear_cka_gate_score_matrix: gate_matrix,
            nmi_matrix,
            pairs,
        })
    }
}

fn validate_file(
    slot: &PlanSlot,
    file: &DenseVectorFile,
    expected_rows: usize,
) -> Result<(), String> {
    if file.count() as usize != expected_rows {
        return Err(format!(
            "CALYX_FSV_ASSAY_I8BIN_CARD_VECTOR_MISMATCH: slot {} {} rows {} != labels {}",
            slot.slot,
            slot.name,
            file.count(),
            expected_rows
        ));
    }
    if let Some(dim) = slot.dim
        && dim != file.dim()
    {
        return Err(format!(
            "CALYX_FSV_ASSAY_I8BIN_CARD_VECTOR_MISMATCH: slot {} {} dim {} != report {}",
            slot.slot,
            slot.name,
            file.dim(),
            dim
        ));
    }
    Ok(())
}

fn sample_lens(
    slot: &PlanSlot,
    file: &DenseVectorFile,
    sample: &SampleRows,
) -> Result<EnsembleLensInput, String> {
    let mut vectors = Vec::with_capacity(sample.indices.len());
    for idx in &sample.indices {
        vectors.push(file.row_f32(*idx));
    }
    Ok(EnsembleLensInput::new(
        slot.name.clone(),
        SlotId::new(slot.slot),
        vectors,
    ))
}

fn row_signatures(file: &DenseVectorFile, indices: &[u64]) -> Vec<f32> {
    indices
        .iter()
        .map(|idx| {
            let row = file.row_f32(*idx);
            (row.iter().map(|value| f64::from(*value)).sum::<f64>() / row.len().max(1) as f64)
                as f32
        })
        .collect()
}

fn identity_matrix(size: usize) -> Vec<Vec<f32>> {
    let mut matrix = vec![vec![0.0; size]; size];
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    matrix
}

fn set_symmetric(matrix: &mut [Vec<f32>], a: usize, b: usize, value: f32) {
    matrix[a][b] = value;
    matrix[b][a] = value;
}
