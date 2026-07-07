//! Empirical copula tail/co-movement summaries (#59).
//!
//! The implementation is rank based and assumes continuous margins. Ties fail
//! closed so the empirical copula, tail lambdas, and diagonal association
//! summaries are not silently made arbitrary by tie-breaking.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const MIN_COPULA_SAMPLES: usize = 20;
pub const DEFAULT_TAIL_Q: f64 = 0.10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopulaTailReport {
    pub estimator: String,
    pub n_samples: usize,
    pub tail_q: f64,
    pub blomqvist_beta: f64,
    pub hoeffding_d_cvm: f64,
    pub gini_gamma: f64,
    pub lower_tail_lambda: f64,
    pub upper_tail_lambda: f64,
    pub lower_tail_count: usize,
    pub upper_tail_count: usize,
}

pub fn empirical_copula_tail_dependence(x: &[f64], y: &[f64]) -> Result<CopulaTailReport> {
    empirical_copula_tail_dependence_with_q(x, y, DEFAULT_TAIL_Q)
}

pub fn empirical_copula_tail_dependence_with_q(
    x: &[f64],
    y: &[f64],
    tail_q: f64,
) -> Result<CopulaTailReport> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "empirical copula requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    let n = x.len();
    if n < MIN_COPULA_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "empirical copula requires at least {MIN_COPULA_SAMPLES} samples; got {n}"
        )));
    }
    if !(tail_q > 0.0 && tail_q < 0.5 && tail_q.is_finite()) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "empirical copula tail_q must be finite in (0, 0.5); got {tail_q}"
        )));
    }
    let u = pseudo_observations("x", x)?;
    let v = pseudo_observations("y", y)?;

    let c_mid = empirical_copula_at(&u, &v, 0.5, 0.5);
    let blomqvist_beta = (4.0 * c_mid - 1.0).clamp(-1.0, 1.0);

    let lower_tail_count = u
        .iter()
        .zip(&v)
        .filter(|&(&a, &b)| a <= tail_q && b <= tail_q)
        .count();
    let upper_tail_count = u
        .iter()
        .zip(&v)
        .filter(|&(&a, &b)| a >= 1.0 - tail_q && b >= 1.0 - tail_q)
        .count();
    let lower_tail_lambda = ((lower_tail_count as f64 / n as f64) / tail_q).clamp(0.0, 1.0);
    let upper_tail_lambda = ((upper_tail_count as f64 / n as f64) / tail_q).clamp(0.0, 1.0);

    let hoeffding_d_cvm = 30.0
        * u.iter()
            .zip(&v)
            .map(|(&a, &b)| {
                let c = empirical_copula_at(&u, &v, a, b);
                (c - a * b).powi(2)
            })
            .sum::<f64>()
        / n as f64;

    let gini_gamma = empirical_gini_gamma(&u, &v).clamp(-1.0, 1.0);

    Ok(CopulaTailReport {
        estimator: "empirical_rank_copula_tail_dependence".to_string(),
        n_samples: n,
        tail_q,
        blomqvist_beta,
        hoeffding_d_cvm,
        gini_gamma,
        lower_tail_lambda,
        upper_tail_lambda,
        lower_tail_count,
        upper_tail_count,
    })
}

fn pseudo_observations(name: &str, values: &[f64]) -> Result<Vec<f64>> {
    let mut indexed = Vec::with_capacity(values.len());
    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "empirical copula {name}[{index}] is not finite ({value})"
            )));
        }
        indexed.push((index, value));
    }
    indexed.sort_by(|left, right| left.1.total_cmp(&right.1));
    for pair in indexed.windows(2) {
        if pair[0].1 == pair[1].1 {
            return Err(CalyxError::assay_degenerate_input(format!(
                "empirical copula requires continuous margins; {name} has tied value {}",
                pair[0].1
            )));
        }
    }
    let n_plus_one = (values.len() + 1) as f64;
    let mut out = vec![0.0; values.len()];
    for (rank_index, (original_index, _)) in indexed.into_iter().enumerate() {
        out[original_index] = (rank_index + 1) as f64 / n_plus_one;
    }
    Ok(out)
}

fn empirical_copula_at(u: &[f64], v: &[f64], a: f64, b: f64) -> f64 {
    u.iter()
        .zip(v)
        .filter(|&(&left, &right)| left <= a && right <= b)
        .count() as f64
        / u.len() as f64
}

fn empirical_gini_gamma(u: &[f64], v: &[f64]) -> f64 {
    let n = u.len();
    let integral = (1..=n)
        .map(|i| {
            let t = i as f64 / (n + 1) as f64;
            empirical_copula_at(u, v, t, t) + empirical_copula_at(u, v, t, 1.0 - t)
        })
        .sum::<f64>()
        / n as f64;
    4.0 * integral - 2.0
}
