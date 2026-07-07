use std::cmp::Ordering;

use crate::error::{PolyError, Result};
use crate::parameter_adaptation_types::*;

pub(crate) fn proposed_parameters(
    request: &ParameterAdaptationRequest,
) -> Result<ParameterSetSnapshot> {
    let sigma = standard_deviation(request.observations.iter().map(|row| row.scalar_value))?;
    let quantile_edges =
        quantile_edges(request.observations.iter().map(|row| row.heavy_tail_value))?;
    let te_lag = best_te_lag(&request.observations, request.schedule.max_te_lag)?;
    let knn_k = best_knn_k(&request.observations, &request.schedule.candidate_knn_k)?;
    let version_hash = version_hash(request, sigma, &quantile_edges, te_lag, knn_k);
    Ok(ParameterSetSnapshot {
        version: format!(
            "{}:{}:{}:{}",
            request.domain,
            request.horizon_bucket,
            request.schedule.scheduled_at_ts,
            &version_hash[..12]
        ),
        encoder_sigma: sigma,
        quantile_edges,
        te_lag,
        knn_k,
    })
}

pub(crate) fn adaptation_metrics(
    request: &ParameterAdaptationRequest,
    proposed: &ParameterSetSnapshot,
) -> Result<ParameterAdaptationMetrics> {
    let current_knn_brier = loo_knn_brier(&request.observations, request.current.knn_k)?;
    let selected_knn_brier = loo_knn_brier(&request.observations, proposed.knn_k)?;
    Ok(ParameterAdaptationMetrics {
        current_knn_brier,
        selected_knn_brier,
        brier_improvement: current_knn_brier - selected_knn_brier,
        selected_te_score: lag_score(&request.observations, proposed.te_lag)?,
        selected_sigma: proposed.encoder_sigma,
        selected_knn_k: proposed.knn_k,
    })
}

pub(crate) fn validate_edges(edges: &[f64]) -> Result<()> {
    if edges.len() < 2 || edges.iter().any(|edge| !edge.is_finite()) {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_DEGENERATE,
            "quantile edges require at least two finite values",
        ));
    }
    if edges.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_DEGENERATE,
            "quantile edges must be strictly increasing",
        ));
    }
    Ok(())
}

fn standard_deviation(values: impl Iterator<Item = f64>) -> Result<f64> {
    let values = values.collect::<Vec<_>>();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / values.len() as f64;
    let sigma = variance.sqrt();
    if sigma.is_finite() && sigma > 1.0e-9 {
        Ok(sigma)
    } else {
        Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_DEGENERATE,
            "encoder sigma could not be identified from degenerate scalar history",
        ))
    }
}

fn quantile_edges(values: impl Iterator<Item = f64>) -> Result<Vec<f64>> {
    let mut sorted = values.collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mut edges = [0.0, 0.25, 0.5, 0.75, 1.0]
        .iter()
        .map(|q| quantile(&sorted, *q))
        .collect::<Vec<_>>();
    edges.dedup_by(|a, b| (*a - *b).abs() <= 1.0e-12);
    validate_edges(&edges)?;
    Ok(edges)
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
    }
}

fn best_te_lag(rows: &[ParameterObservation], max_lag: usize) -> Result<usize> {
    let mut best = (1, f64::NEG_INFINITY);
    for lag in 1..=max_lag {
        let score = lag_score(rows, lag)?;
        if score > best.1 {
            best = (lag, score);
        }
    }
    Ok(best.0)
}

fn lag_score(rows: &[ParameterObservation], lag: usize) -> Result<f64> {
    if lag == 0 || lag >= rows.len() {
        return invalid("lag must be in 1..observation_count");
    }
    let pairs = (lag..rows.len())
        .map(|idx| {
            (
                rows[idx - lag].lag_signal,
                if rows[idx].outcome_yes { 1.0 } else { 0.0 },
            )
        })
        .collect::<Vec<_>>();
    Ok(correlation_abs(&pairs))
}

fn correlation_abs(pairs: &[(f64, f64)]) -> f64 {
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n;
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for (x, y) in pairs {
        cov += (x - mean_x) * (y - mean_y);
        vx += (x - mean_x) * (x - mean_x);
        vy += (y - mean_y) * (y - mean_y);
    }
    if vx <= f64::EPSILON || vy <= f64::EPSILON {
        0.0
    } else {
        (cov / (vx.sqrt() * vy.sqrt())).abs()
    }
}

fn best_knn_k(rows: &[ParameterObservation], candidates: &[usize]) -> Result<usize> {
    let mut best = (0, f64::INFINITY);
    for &k in candidates {
        if k == 0 || k >= rows.len() {
            return invalid("candidate kNN k must be in 1..observation_count");
        }
        let brier = loo_knn_brier(rows, k)?;
        if brier < best.1 {
            best = (k, brier);
        }
    }
    Ok(best.0)
}

fn loo_knn_brier(rows: &[ParameterObservation], k: usize) -> Result<f64> {
    if k == 0 || k >= rows.len() {
        return invalid("kNN k must be in 1..observation_count");
    }
    let mut total = 0.0;
    for idx in 0..rows.len() {
        let mut sims = (0..rows.len())
            .filter(|other| *other != idx)
            .map(|other| {
                (
                    cosine(&rows[idx].knn_vector, &rows[other].knn_vector),
                    other,
                )
            })
            .collect::<Vec<_>>();
        sims.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let yes = sims
            .iter()
            .take(k)
            .filter(|(_, other)| rows[*other].outcome_yes)
            .count();
        let p = yes as f64 / k as f64;
        let y = if rows[idx].outcome_yes { 1.0 } else { 0.0 };
        total += (p - y) * (p - y);
    }
    Ok(total / rows.len() as f64)
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f64::EPSILON || nb <= f64::EPSILON {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

fn version_hash(
    request: &ParameterAdaptationRequest,
    sigma: f64,
    edges: &[f64],
    te_lag: usize,
    knn_k: usize,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.horizon_bucket.as_bytes());
    hasher.update(&request.schedule.scheduled_at_ts.to_le_bytes());
    hasher.update(&sigma.to_le_bytes());
    for edge in edges {
        hasher.update(&edge.to_le_bytes());
    }
    hasher.update(&te_lag.to_le_bytes());
    hasher.update(&knn_k.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PARAMETER_ADAPTATION_INVALID_REQUEST,
        message.into(),
    ))
}
