use calyx_assay::{CorrelationEvidence, DEFAULT_BOOTSTRAP_RESAMPLES, bootstrap_mean_ci};

use super::calyx_error_detail;

const CORRELATION_BOOTSTRAP_SEED: u64 = 20_260_618;

pub(crate) fn lens_pair_correlation_evidence(
    a: &[Vec<f32>],
    b: &[Vec<f32>],
) -> Result<CorrelationEvidence, String> {
    let values = row_correlations(a, b);
    if values.is_empty() {
        return CorrelationEvidence::new(0.0, 0.0, 0.0).map_err(calyx_error_detail);
    }
    let point = values.iter().sum::<f32>() / values.len() as f32;
    let ci = bootstrap_mean_ci(
        &values,
        DEFAULT_BOOTSTRAP_RESAMPLES,
        CORRELATION_BOOTSTRAP_SEED,
    )
    .ok_or_else(|| "CALYX_ASSAY_UNRESOLVED: correlation CI requires samples".to_string())?;
    CorrelationEvidence::new(point.max(0.0), ci.ci_low.max(0.0), ci.ci_high.max(0.0))
        .map_err(calyx_error_detail)
}

fn row_correlations(a: &[Vec<f32>], b: &[Vec<f32>]) -> Vec<f32> {
    let n = a.len().min(b.len());
    if n == 0 || a.first().map(Vec::len) != b.first().map(Vec::len) {
        return Vec::new();
    }
    a.iter()
        .zip(b)
        .take(n)
        .map(|(left, right)| cosine(left, right).max(0.0))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dim = a.len().min(b.len());
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for idx in 0..dim {
        dot += a[idx] * b[idx];
        norm_a += a[idx] * a[idx];
        norm_b += b[idx] * b[idx];
    }
    if norm_a <= 0.0 || norm_b <= 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}
