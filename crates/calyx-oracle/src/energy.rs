//! PH51 energy descent substrate for `complete()`.

use calyx_core::CalyxError;
use calyx_forge::cpu::{cosine_batch, normalize_f32};

use crate::{DomainId, OracleError};

pub const MAX_STEPS: usize = 20;
pub const DEFAULT_EPS: f32 = 1.0e-4;
pub const DEFAULT_BETA: f32 = 1.0;
pub const CALYX_ORACLE_ENERGY_EMPTY_REGION: &str = "CALYX_ORACLE_ENERGY_EMPTY_REGION";
pub const CALYX_ORACLE_ENERGY_INVALID_INPUT: &str = "CALYX_ORACLE_ENERGY_INVALID_INPUT";

const ENERGY_REMEDIATION: &str = "provide finite, same-dimensional non-empty region members";
const FORGE_REMEDIATION: &str = "repair Forge cosine/normalize inputs before energy descent";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DescentResult {
    pub steps_taken: usize,
    pub converged: bool,
    pub final_energy: f32,
}

pub trait AnnealConfig {
    fn energy_beta(&self, domain: &DomainId) -> Option<f32>;
}

pub fn energy(x: &[f32], region_members: &[&[f32]], beta: f32) -> Result<f32, OracleError> {
    validate_beta(beta)?;
    let region = FlatRegion::new(x, region_members)?;
    energy_prepared(x, &region, beta)
}

pub fn energy_softmax_weights(
    x: &[f32],
    region_members: &[&[f32]],
    beta: f32,
) -> Result<Vec<f32>, OracleError> {
    validate_beta(beta)?;
    let region = FlatRegion::new(x, region_members)?;
    let similarities = scaled_similarities_prepared(x, &region, beta)?;
    weights_from_scaled(&similarities, &region, beta)
}

pub fn descent_step(
    free_slot: &mut [f32],
    region_members: &[&[f32]],
    beta: f32,
) -> Result<(), OracleError> {
    validate_beta(beta)?;
    let region = FlatRegion::new(free_slot, region_members)?;
    let similarities = scaled_similarities_prepared(free_slot, &region, beta)?;
    descent_step_prepared(free_slot, &region, &similarities, beta)
}

pub fn descend(
    free_slot: &mut [f32],
    region_members: &[&[f32]],
    beta: f32,
    max_steps: usize,
    eps: f32,
) -> Result<DescentResult, OracleError> {
    validate_eps(eps)?;
    validate_beta(beta)?;
    let region = FlatRegion::new(free_slot, region_members)?;
    let mut similarities = scaled_similarities_prepared(free_slot, &region, beta)?;
    let mut previous = energy_from_scaled(&similarities, &region, beta);
    if max_steps == 0 {
        return Ok(DescentResult {
            steps_taken: 0,
            converged: false,
            final_energy: previous,
        });
    }
    for step in 1..=max_steps {
        descent_step_prepared(free_slot, &region, &similarities, beta)?;
        similarities = scaled_similarities_prepared(free_slot, &region, beta)?;
        let next = energy_from_scaled(&similarities, &region, beta);
        if region.member_count == 1 || (next - previous).abs() < eps {
            return Ok(DescentResult {
                steps_taken: step,
                converged: true,
                final_energy: next,
            });
        }
        previous = next;
    }
    Ok(DescentResult {
        steps_taken: max_steps,
        converged: false,
        final_energy: previous,
    })
}

pub fn get_beta(domain: DomainId, anneal: &dyn AnnealConfig) -> f32 {
    anneal
        .energy_beta(&domain)
        .filter(|beta| beta.is_finite() && *beta >= 0.0)
        .unwrap_or(DEFAULT_BETA)
}

#[derive(Clone, Debug)]
struct FlatRegion {
    dim: usize,
    member_count: usize,
    members: Vec<f32>,
}

impl FlatRegion {
    fn new(x: &[f32], region_members: &[&[f32]]) -> Result<Self, OracleError> {
        let dim = validate_region_shape(x, region_members)?;
        Ok(Self {
            dim,
            member_count: region_members.len(),
            members: flatten_region(region_members, dim),
        })
    }
}

fn energy_prepared(x: &[f32], region: &FlatRegion, beta: f32) -> Result<f32, OracleError> {
    let scaled = scaled_similarities_prepared(x, region, beta)?;
    Ok(energy_from_scaled(&scaled, region, beta))
}

fn energy_from_scaled(scaled: &[f32], region: &FlatRegion, beta: f32) -> f32 {
    if beta == 0.0 {
        -(region.member_count as f32).ln()
    } else {
        -log_sum_exp(scaled)
    }
}

fn scaled_similarities_prepared(
    x: &[f32],
    region: &FlatRegion,
    beta: f32,
) -> Result<Vec<f32>, OracleError> {
    if x.len() != region.dim {
        return Err(energy_error(
            CALYX_ORACLE_ENERGY_INVALID_INPUT,
            format!(
                "free slot vector dim {} does not match prepared region dim {}",
                x.len(),
                region.dim
            ),
        ));
    }
    check_finite_slice("free_slot", x)?;
    if beta == 0.0 {
        return Ok(vec![0.0; region.member_count]);
    }
    let mut similarities = vec![0.0; region.member_count];
    cosine_batch(x, &region.members, region.dim, &mut similarities).map_err(forge_error)?;
    Ok(similarities
        .into_iter()
        .map(|similarity| beta * similarity)
        .collect())
}

fn weights_from_scaled(
    scaled: &[f32],
    region: &FlatRegion,
    beta: f32,
) -> Result<Vec<f32>, OracleError> {
    if beta == 0.0 {
        return Ok(vec![1.0 / region.member_count as f32; region.member_count]);
    }
    stable_softmax(scaled)
}

fn descent_step_prepared(
    free_slot: &mut [f32],
    region: &FlatRegion,
    similarities: &[f32],
    beta: f32,
) -> Result<(), OracleError> {
    let weights = weights_from_scaled(similarities, region, beta)?;
    let mut next = vec![0.0; region.dim];
    for (weight, member) in weights.iter().zip(region.members.chunks_exact(region.dim)) {
        for (dst, src) in next.iter_mut().zip(member.iter()) {
            *dst += weight * src;
        }
    }
    normalize_f32(&mut next, region.dim).map_err(forge_error)?;
    free_slot.copy_from_slice(&next);
    Ok(())
}

fn validate_region_shape(x: &[f32], region_members: &[&[f32]]) -> Result<usize, OracleError> {
    if region_members.is_empty() {
        return Err(energy_error(
            CALYX_ORACLE_ENERGY_EMPTY_REGION,
            "region_members must contain at least one attractor",
        ));
    }
    if x.is_empty() {
        return Err(energy_error(
            CALYX_ORACLE_ENERGY_INVALID_INPUT,
            "free slot vector must be non-empty",
        ));
    }
    check_finite_slice("free_slot", x)?;
    let dim = x.len();
    for (index, member) in region_members.iter().enumerate() {
        if member.len() != dim {
            return Err(energy_error(
                CALYX_ORACLE_ENERGY_INVALID_INPUT,
                format!(
                    "region member {index} dim {} does not match free slot dim {dim}",
                    member.len()
                ),
            ));
        }
        check_finite_slice("region_member", member)?;
    }
    Ok(dim)
}

fn validate_beta(beta: f32) -> Result<(), OracleError> {
    if beta.is_finite() && beta >= 0.0 {
        return Ok(());
    }
    Err(energy_error(
        CALYX_ORACLE_ENERGY_INVALID_INPUT,
        format!("beta must be finite and non-negative, got {beta}"),
    ))
}

fn validate_eps(eps: f32) -> Result<(), OracleError> {
    if eps.is_finite() && eps >= 0.0 {
        return Ok(());
    }
    Err(energy_error(
        CALYX_ORACLE_ENERGY_INVALID_INPUT,
        format!("eps must be finite and non-negative, got {eps}"),
    ))
}

fn check_finite_slice(label: &str, values: &[f32]) -> Result<(), OracleError> {
    if values.iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    Err(energy_error(
        CALYX_ORACLE_ENERGY_INVALID_INPUT,
        format!("{label} contains NaN or Inf"),
    ))
}

fn flatten_region(region_members: &[&[f32]], dim: usize) -> Vec<f32> {
    let mut flattened = Vec::with_capacity(region_members.len() * dim);
    for member in region_members {
        flattened.extend_from_slice(member);
    }
    flattened
}

fn stable_softmax(scores: &[f32]) -> Result<Vec<f32>, OracleError> {
    let log_z = log_sum_exp(scores);
    let weights: Vec<_> = scores.iter().map(|score| (*score - log_z).exp()).collect();
    check_finite_slice("softmax_weights", &weights)?;
    Ok(weights)
}

fn log_sum_exp(scores: &[f32]) -> f32 {
    let max = scores
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, |acc, score| acc.max(score));
    let sum = scores.iter().map(|score| (*score - max).exp()).sum::<f32>();
    max + sum.ln()
}

fn forge_error(error: calyx_forge::ForgeError) -> OracleError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: FORGE_REMEDIATION,
    }
    .into()
}

fn energy_error(code: &'static str, message: impl Into<String>) -> OracleError {
    CalyxError {
        code,
        message: message.into(),
        remediation: ENERGY_REMEDIATION,
    }
    .into()
}

#[cfg(test)]
#[path = "energy_tests.rs"]
mod tests;
