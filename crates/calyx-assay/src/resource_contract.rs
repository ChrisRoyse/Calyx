//! Resource-aware lens admission and panel packing.

use calyx_core::{CalyxError, LensCost, Placement, Result};
use serde::{Deserialize, Serialize};

use crate::admit_lens;

pub const CALYX_ASSAY_INVALID_RESOURCE: &str = "CALYX_ASSAY_INVALID_RESOURCE";
pub const CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED: &str = "CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED";

const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;
const RESOURCE_EPSILON: f32 = 1e-6;
const BLOCKED_RESOURCE_FRACTION: f32 = 1.0e30;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelResourceBudget {
    pub max_vram_mb: f32,
    pub max_ram_mb: f32,
    pub max_ms_per_input: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub vram_mb: f32,
    pub ram_mb: f32,
    pub ms_per_input: f32,
}

impl ResourceUsage {
    pub fn from_lens_cost(cost: LensCost) -> Self {
        Self {
            vram_mb: cost.vram_bytes as f32 / BYTES_PER_MIB,
            ram_mb: cost.ram_bytes as f32 / BYTES_PER_MIB,
            ms_per_input: cost.ms_per_input,
        }
    }

    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            vram_mb: self.vram_mb + other.vram_mb,
            ram_mb: self.ram_mb + other.ram_mb,
            ms_per_input: self.ms_per_input + other.ms_per_input,
        }
    }

    pub fn remaining_after(self, used: Self) -> Self {
        Self {
            vram_mb: (self.vram_mb - used.vram_mb).max(0.0),
            ram_mb: (self.ram_mb - used.ram_mb).max(0.0),
            ms_per_input: (self.ms_per_input - used.ms_per_input).max(0.0),
        }
    }

    pub fn fits_within(self, budget: PanelResourceBudget) -> bool {
        self.vram_mb <= budget.max_vram_mb + RESOURCE_EPSILON
            && self.ram_mb <= budget.max_ram_mb + RESOURCE_EPSILON
            && self.ms_per_input <= budget.max_ms_per_input + RESOURCE_EPSILON
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceDensity {
    pub placement: Placement,
    pub zero_vram: bool,
    pub bits_per_vram_mb: Option<f32>,
    pub bits_per_ram_mb: Option<f32>,
    pub bits_per_ms: Option<f32>,
    pub dominant_budget_fraction: f32,
    pub bits_per_budget_fraction: Option<f32>,
}

impl ResourceDensity {
    pub fn compute(
        signal_bits: f32,
        usage: ResourceUsage,
        placement: Placement,
        budget: PanelResourceBudget,
    ) -> Self {
        let bits = signal_bits.max(0.0);
        let vram_fraction = resource_fraction(usage.vram_mb, budget.max_vram_mb);
        let ram_fraction = resource_fraction(usage.ram_mb, budget.max_ram_mb);
        let ms_fraction = resource_fraction(usage.ms_per_input, budget.max_ms_per_input);
        let dominant_budget_fraction = vram_fraction.max(ram_fraction).max(ms_fraction);
        Self {
            placement,
            zero_vram: usage.vram_mb <= RESOURCE_EPSILON,
            bits_per_vram_mb: density_axis(bits, usage.vram_mb),
            bits_per_ram_mb: density_axis(bits, usage.ram_mb),
            bits_per_ms: density_axis(bits, usage.ms_per_input),
            dominant_budget_fraction,
            bits_per_budget_fraction: density_axis(bits, dominant_budget_fraction),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceAwareAdmissionDecision {
    pub admitted: bool,
    pub signal_bits: f32,
    pub max_pairwise_corr: f32,
    pub usage: ResourceUsage,
    pub placement: Placement,
    pub density: ResourceDensity,
    pub budget: PanelResourceBudget,
    pub remaining_budget: ResourceUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelAdmissionCandidate {
    pub lens: String,
    pub signal_bits: f32,
    pub max_pairwise_corr: f32,
    pub usage: ResourceUsage,
    pub placement: Placement,
    #[serde(default)]
    pub resident: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelLensDecision {
    pub lens: String,
    pub admitted: bool,
    pub resident: bool,
    pub signal_bits: f32,
    pub max_pairwise_corr: f32,
    pub usage: ResourceUsage,
    pub placement: Placement,
    pub density: ResourceDensity,
    pub rejection_reason: Option<String>,
    pub remaining_budget_after: Option<ResourceUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelPackingReport {
    pub budget: PanelResourceBudget,
    pub selected: Vec<PanelLensDecision>,
    pub rejected: Vec<PanelLensDecision>,
    pub evicted_lenses: Vec<String>,
    pub total_signal_bits: f32,
    pub used: ResourceUsage,
    pub remaining: ResourceUsage,
    pub aggregate_bits_per_vram_mb: Option<f32>,
    pub aggregate_bits_per_ram_mb: Option<f32>,
    pub aggregate_bits_per_ms: Option<f32>,
    pub aggregate_bits_per_budget_fraction: Option<f32>,
}

pub fn admit_lens_with_resources(
    signal_bits: f32,
    max_pairwise_corr: f32,
    cost: LensCost,
    placement: Placement,
    budget: PanelResourceBudget,
) -> Result<ResourceAwareAdmissionDecision> {
    let usage = ResourceUsage::from_lens_cost(cost);
    admit_lens_with_usage(signal_bits, max_pairwise_corr, usage, placement, budget)
}

pub fn admit_lens_with_usage(
    signal_bits: f32,
    max_pairwise_corr: f32,
    usage: ResourceUsage,
    placement: Placement,
    budget: PanelResourceBudget,
) -> Result<ResourceAwareAdmissionDecision> {
    let decision = admit_lens(signal_bits, max_pairwise_corr)?;
    validate_budget(budget)?;
    validate_usage(usage)?;
    if !usage.fits_within(budget) {
        return Err(resource_budget_error(format!(
            "lens usage vram={:.3}MiB ram={:.3}MiB ms={:.3} exceeds budget vram={:.3}MiB ram={:.3}MiB ms={:.3}",
            usage.vram_mb,
            usage.ram_mb,
            usage.ms_per_input,
            budget.max_vram_mb,
            budget.max_ram_mb,
            budget.max_ms_per_input
        )));
    }
    let density = ResourceDensity::compute(signal_bits, usage, placement, budget);
    Ok(ResourceAwareAdmissionDecision {
        admitted: decision.admitted,
        signal_bits,
        max_pairwise_corr,
        usage,
        placement,
        density,
        budget,
        remaining_budget: budget_usage(budget).remaining_after(usage),
    })
}

pub fn pack_panel_by_density(
    candidates: &[PanelAdmissionCandidate],
    budget: PanelResourceBudget,
) -> Result<PanelPackingReport> {
    validate_budget(budget)?;
    let mut feasible = Vec::new();
    let mut rejected = Vec::new();
    for candidate in candidates {
        validate_candidate(candidate)?;
        let density = ResourceDensity::compute(
            candidate.signal_bits,
            candidate.usage,
            candidate.placement,
            budget,
        );
        match admit_lens(candidate.signal_bits, candidate.max_pairwise_corr) {
            Ok(_) if candidate.usage.fits_within(budget) => feasible.push(candidate.clone()),
            Ok(_) => rejected.push(decision_for(
                candidate,
                density,
                Some(format!(
                    "{CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED}: lens usage exceeds total panel budget"
                )),
                None,
            )),
            Err(error) => rejected.push(decision_for(
                candidate,
                density,
                Some(error.code.to_string()),
                None,
            )),
        }
    }

    feasible.sort_by(|left, right| compare_candidate_density(left, right, budget));
    let mut selected = Vec::new();
    let mut used = ResourceUsage::default();
    for candidate in &feasible {
        let next_used = used.saturating_add(candidate.usage);
        let density = ResourceDensity::compute(
            candidate.signal_bits,
            candidate.usage,
            candidate.placement,
            budget,
        );
        if next_used.fits_within(budget) {
            used = next_used;
            selected.push(decision_for(
                candidate,
                density,
                None,
                Some(budget_usage(budget).remaining_after(used)),
            ));
        } else {
            rejected.push(decision_for(
                candidate,
                density,
                Some(format!(
                    "{CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED}: cumulative panel budget exhausted"
                )),
                None,
            ));
        }
    }

    if let Some(candidate) = best_feasible_singleton(&feasible, budget)
        && candidate.signal_bits > total_signal(&selected) + RESOURCE_EPSILON
    {
        for decision in selected.drain(..) {
            rejected.push(PanelLensDecision {
                admitted: false,
                rejection_reason: Some(
                    "CALYX_ASSAY_REPLACED_BY_HIGHER_SIGNAL_SINGLETON".to_string(),
                ),
                remaining_budget_after: None,
                ..decision
            });
        }
        let density = ResourceDensity::compute(
            candidate.signal_bits,
            candidate.usage,
            candidate.placement,
            budget,
        );
        used = candidate.usage;
        selected.push(decision_for(
            candidate,
            density,
            None,
            Some(budget_usage(budget).remaining_after(used)),
        ));
    }

    rejected.sort_by(|left, right| left.lens.cmp(&right.lens));
    let evicted_lenses = rejected
        .iter()
        .filter(|decision| decision.resident)
        .map(|decision| decision.lens.clone())
        .collect::<Vec<_>>();
    let total_signal_bits = total_signal(&selected);
    let remaining = budget_usage(budget).remaining_after(used);
    let dominant_fraction = resource_fraction(used.vram_mb, budget.max_vram_mb)
        .max(resource_fraction(used.ram_mb, budget.max_ram_mb))
        .max(resource_fraction(
            used.ms_per_input,
            budget.max_ms_per_input,
        ));

    Ok(PanelPackingReport {
        budget,
        selected,
        rejected,
        evicted_lenses,
        total_signal_bits,
        used,
        remaining,
        aggregate_bits_per_vram_mb: density_axis(total_signal_bits, used.vram_mb),
        aggregate_bits_per_ram_mb: density_axis(total_signal_bits, used.ram_mb),
        aggregate_bits_per_ms: density_axis(total_signal_bits, used.ms_per_input),
        aggregate_bits_per_budget_fraction: density_axis(total_signal_bits, dominant_fraction),
    })
}

fn validate_candidate(candidate: &PanelAdmissionCandidate) -> Result<()> {
    if candidate.lens.trim().is_empty() {
        return Err(invalid_resource("lens name must be non-empty"));
    }
    validate_usage(candidate.usage)
}

fn validate_usage(usage: ResourceUsage) -> Result<()> {
    validate_nonnegative_resource("vram_mb", usage.vram_mb)?;
    validate_nonnegative_resource("ram_mb", usage.ram_mb)?;
    validate_nonnegative_resource("ms_per_input", usage.ms_per_input)?;
    Ok(())
}

fn validate_budget(budget: PanelResourceBudget) -> Result<()> {
    validate_nonnegative_resource("max_vram_mb", budget.max_vram_mb)?;
    validate_nonnegative_resource("max_ram_mb", budget.max_ram_mb)?;
    validate_nonnegative_resource("max_ms_per_input", budget.max_ms_per_input)?;
    if budget.max_vram_mb <= RESOURCE_EPSILON
        && budget.max_ram_mb <= RESOURCE_EPSILON
        && budget.max_ms_per_input <= RESOURCE_EPSILON
    {
        return Err(invalid_resource(
            "panel resource budget must expose at least one positive capacity",
        ));
    }
    Ok(())
}

fn validate_nonnegative_resource(name: &'static str, value: f32) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(invalid_resource(format!(
            "{name} must be finite and non-negative, got {value}"
        )));
    }
    Ok(())
}

fn compare_candidate_density(
    left: &PanelAdmissionCandidate,
    right: &PanelAdmissionCandidate,
    budget: PanelResourceBudget,
) -> std::cmp::Ordering {
    let left_density =
        ResourceDensity::compute(left.signal_bits, left.usage, left.placement, budget);
    let right_density =
        ResourceDensity::compute(right.signal_bits, right.usage, right.placement, budget);
    compare_optional_density(
        left_density.bits_per_budget_fraction,
        right_density.bits_per_budget_fraction,
    )
    .then_with(|| right.signal_bits.total_cmp(&left.signal_bits))
    .then_with(|| left.lens.cmp(&right.lens))
}

fn compare_optional_density(left: Option<f32>, right: Option<f32>) -> std::cmp::Ordering {
    match (left, right) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(left), Some(right)) => right.total_cmp(&left),
    }
}

fn best_feasible_singleton(
    candidates: &[PanelAdmissionCandidate],
    budget: PanelResourceBudget,
) -> Option<&PanelAdmissionCandidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.usage.fits_within(budget))
        .max_by(|left, right| {
            left.signal_bits
                .total_cmp(&right.signal_bits)
                .then_with(|| right.lens.cmp(&left.lens))
        })
}

fn decision_for(
    candidate: &PanelAdmissionCandidate,
    density: ResourceDensity,
    rejection_reason: Option<String>,
    remaining_budget_after: Option<ResourceUsage>,
) -> PanelLensDecision {
    PanelLensDecision {
        lens: candidate.lens.clone(),
        admitted: rejection_reason.is_none(),
        resident: candidate.resident,
        signal_bits: candidate.signal_bits,
        max_pairwise_corr: candidate.max_pairwise_corr,
        usage: candidate.usage,
        placement: candidate.placement,
        density,
        rejection_reason,
        remaining_budget_after,
    }
}

fn total_signal(selected: &[PanelLensDecision]) -> f32 {
    selected
        .iter()
        .map(|decision| decision.signal_bits)
        .sum::<f32>()
}

fn budget_usage(budget: PanelResourceBudget) -> ResourceUsage {
    ResourceUsage {
        vram_mb: budget.max_vram_mb,
        ram_mb: budget.max_ram_mb,
        ms_per_input: budget.max_ms_per_input,
    }
}

fn resource_fraction(usage: f32, budget: f32) -> f32 {
    if usage <= RESOURCE_EPSILON {
        return 0.0;
    }
    if budget <= RESOURCE_EPSILON {
        return BLOCKED_RESOURCE_FRACTION;
    }
    usage / budget
}

fn density_axis(bits: f32, resource: f32) -> Option<f32> {
    if resource <= RESOURCE_EPSILON {
        None
    } else {
        Some(bits / resource)
    }
}

fn invalid_resource(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_RESOURCE,
        message: message.into(),
        remediation: "provide finite measured lens cost and a positive panel resource budget",
    }
}

fn resource_budget_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED,
        message: message.into(),
        remediation: "raise the panel budget or evict lower-density resident lenses",
    }
}

#[cfg(test)]
#[path = "resource_contract_tests.rs"]
mod tests;
