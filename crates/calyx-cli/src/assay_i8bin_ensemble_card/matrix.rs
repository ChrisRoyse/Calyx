use calyx_assay::{EnsembleLensInput, partitioned_histogram_nmi, stable_rank};
use calyx_core::SlotId;
use calyx_sextant::index::DenseVectorFile;
use serde::Serialize;

use crate::assay_bits_validation::calyx_error_detail;

use super::plan::{LoadedPlan, PlanSlot};
use super::rows::{LabelRows, SampleRows, signature_indices};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MatrixReadout {
    pub(crate) signature_rows: usize,
    pub(crate) n_eff: f32,
    pub(crate) mean_pairwise_corr: f32,
    pub(crate) mean_pairwise_nmi: f32,
    pub(crate) correlation_matrix: Vec<Vec<f32>>,
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
}

pub(crate) struct VectorReadout {
    pub(crate) lenses: Vec<EnsembleLensInput>,
    pub(crate) matrix: MatrixReadout,
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
    let mut lenses = Vec::with_capacity(plan.slots.len());
    let mut signatures = Vec::with_capacity(plan.slots.len());
    let mut dims = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        let file = DenseVectorFile::open(&slot.corpus).map_err(calyx_error_detail)?;
        validate_file(slot, &file, total_rows)?;
        dims.push(file.dim());
        lenses.push(sample_lens(slot, &file, sample)?);
        signatures.push(row_signatures(&file, &signature_idx));
    }
    let matrix = pair_matrix(&plan.slots, &signatures, nmi_bins)?;
    Ok(VectorReadout {
        lenses,
        matrix,
        dims,
    })
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
        .map(|idx| match file {
            DenseVectorFile::Fbin(file) => mean(file.row(*idx)),
            DenseVectorFile::I8Bin(file) => i8_normalized_mean(file.row_i8(*idx)),
        })
        .collect()
}

fn i8_normalized_mean(row: &[i8]) -> f32 {
    let mut sum = 0.0_f32;
    let mut norm_sq = 0.0_f32;
    for value in row {
        let value = f32::from(*value);
        sum += value;
        norm_sq += value * value;
    }
    if norm_sq <= f32::EPSILON {
        0.0
    } else {
        sum / norm_sq.sqrt() / row.len().max(1) as f32
    }
}

fn mean(row: &[f32]) -> f32 {
    row.iter().sum::<f32>() / row.len().max(1) as f32
}

fn pair_matrix(
    slots: &[PlanSlot],
    signatures: &[Vec<f32>],
    nmi_bins: usize,
) -> Result<MatrixReadout, String> {
    let mut corr_matrix = vec![vec![0.0; slots.len()]; slots.len()];
    let mut nmi_matrix = vec![vec![0.0; slots.len()]; slots.len()];
    for idx in 0..slots.len() {
        corr_matrix[idx][idx] = 1.0;
        nmi_matrix[idx][idx] = 1.0;
    }
    let mut pairs = Vec::new();
    for a in 0..slots.len() {
        for b in (a + 1)..slots.len() {
            let corr = pearson_abs(&signatures[a], &signatures[b]);
            let nmi = partitioned_histogram_nmi(&signatures[a], &signatures[b], nmi_bins)
                .map_err(calyx_error_detail)?
                .nmi;
            corr_matrix[a][b] = corr;
            corr_matrix[b][a] = corr;
            nmi_matrix[a][b] = nmi;
            nmi_matrix[b][a] = nmi;
            pairs.push(PairReadout {
                a: slots[a].name.clone(),
                b: slots[b].name.clone(),
                slot_a: slots[a].slot,
                slot_b: slots[b].slot,
                corr,
                nmi,
            });
        }
    }
    let n_eff = stable_rank(&corr_matrix).n_eff;
    let mean_pairwise_corr = mean_pairwise(&pairs, |pair| pair.corr);
    let mean_pairwise_nmi = mean_pairwise(&pairs, |pair| pair.nmi);
    Ok(MatrixReadout {
        signature_rows: signatures.first().map(Vec::len).unwrap_or(0),
        n_eff,
        mean_pairwise_corr,
        mean_pairwise_nmi,
        correlation_matrix: corr_matrix,
        nmi_matrix,
        pairs,
    })
}

fn mean_pairwise<F>(pairs: &[PairReadout], get: F) -> f32
where
    F: Fn(&PairReadout) -> f32,
{
    pairs.iter().map(get).sum::<f32>() / pairs.len().max(1) as f32
}

fn pearson_abs(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mean_a = a.iter().take(n).sum::<f32>() / n as f32;
    let mean_b = b.iter().take(n).sum::<f32>() / n as f32;
    let mut num = 0.0;
    let mut den_a = 0.0;
    let mut den_b = 0.0;
    for idx in 0..n {
        let da = a[idx] - mean_a;
        let db = b[idx] - mean_b;
        num += da * db;
        den_a += da * da;
        den_b += db * db;
    }
    if den_a <= f32::EPSILON || den_b <= f32::EPSILON {
        0.0
    } else {
        (num / (den_a.sqrt() * den_b.sqrt())).abs().min(1.0)
    }
}
