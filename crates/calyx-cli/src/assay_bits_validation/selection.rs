use calyx_assay::{
    PanelLensDecision, PanelPackingReport, PanelResourceBudget, ResourceDensity, ResourceUsage,
};
use calyx_core::Placement;
use serde::Serialize;

use super::cost::LensCostMap;
use super::report::{LensMeasurement, LensReport};

#[derive(Clone, Debug)]
pub(crate) struct SelectionMeasurement {
    pub(crate) name: String,
    pub(crate) bits: f32,
}

impl From<&LensMeasurement> for SelectionMeasurement {
    fn from(measurement: &LensMeasurement) -> Self {
        Self {
            name: measurement.name.clone(),
            bits: measurement.estimate.bits,
        }
    }
}

/// Lenses ranked by the same signal-per-dominant-budget-fraction policy used
/// by density-ordered panel admission.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SignalDensityReport {
    pub(crate) note: String,
    pub(crate) ranked: Vec<DensityRank>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DensityRank {
    pub(crate) name: String,
    pub(crate) bits_about: f32,
    pub(crate) placement: Placement,
    pub(crate) zero_vram: bool,
    pub(crate) vram_mb: f32,
    pub(crate) ram_mb: f32,
    pub(crate) ms_per_input: f32,
    pub(crate) bits_per_vram_mb: Option<f32>,
    pub(crate) bits_per_ram_mb: Option<f32>,
    pub(crate) bits_per_ms: Option<f32>,
    pub(crate) dominant_budget_fraction: f32,
    pub(crate) bits_per_budget_fraction: Option<f32>,
}

pub(crate) fn raw_bits_order(measurements: &[SelectionMeasurement]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..measurements.len()).collect();
    order.sort_by(|&a, &b| {
        measurements[b]
            .bits
            .total_cmp(&measurements[a].bits)
            .then_with(|| measurements[a].name.cmp(&measurements[b].name))
    });
    order
}

pub(crate) fn density_order(
    measurements: &[SelectionMeasurement],
    cost: &LensCostMap,
    budget: PanelResourceBudget,
) -> Result<Vec<usize>, String> {
    let mut order = Vec::with_capacity(measurements.len());
    for (idx, measurement) in measurements.iter().enumerate() {
        let lens_cost = cost.require(&measurement.name)?;
        let density = ResourceDensity::compute(
            measurement.bits,
            lens_cost.usage(),
            lens_cost.placement,
            budget,
        );
        order.push((idx, measurement.name.clone(), measurement.bits, density));
    }
    order.sort_by(|left, right| {
        compare_optional_density(
            left.3.bits_per_budget_fraction,
            right.3.bits_per_budget_fraction,
        )
        .then_with(|| right.2.total_cmp(&left.2))
        .then_with(|| left.1.cmp(&right.1))
    });
    Ok(order.into_iter().map(|(idx, _, _, _)| idx).collect())
}

pub(crate) fn density_budget(
    measurements: &[SelectionMeasurement],
    cost: &LensCostMap,
    panel_budget: Option<PanelResourceBudget>,
) -> Result<PanelResourceBudget, String> {
    if let Some(budget) = panel_budget {
        return Ok(budget);
    }
    let mut usage = ResourceUsage::default();
    for measurement in measurements {
        usage = usage.saturating_add(cost.require(&measurement.name)?.usage());
    }
    Ok(PanelResourceBudget {
        max_vram_mb: usage.vram_mb.max(1.0),
        max_ram_mb: usage.ram_mb.max(1.0),
        max_ms_per_input: usage.ms_per_input.max(1.0),
    })
}

pub(crate) fn compute_signal_density(
    lenses: &mut [LensReport],
    cost: &LensCostMap,
    budget: PanelResourceBudget,
) -> Result<SignalDensityReport, String> {
    for lens in lenses.iter_mut() {
        let lens_cost = cost.require(&lens.name)?;
        let usage = lens_cost.usage();
        lens.usage = Some(usage);
        lens.placement = Some(lens_cost.placement);
        lens.density = Some(ResourceDensity::compute(
            lens.bits_about,
            usage,
            lens_cost.placement,
            budget,
        ));
    }
    let mut ranked: Vec<DensityRank> = lenses
        .iter()
        .map(|lens| {
            let d = lens.density.expect("density set above");
            let usage = lens.usage.expect("usage set above");
            DensityRank {
                name: lens.name.clone(),
                bits_about: lens.bits_about,
                placement: d.placement,
                zero_vram: d.zero_vram,
                vram_mb: usage.vram_mb,
                ram_mb: usage.ram_mb,
                ms_per_input: usage.ms_per_input,
                bits_per_vram_mb: d.bits_per_vram_mb,
                bits_per_ram_mb: d.bits_per_ram_mb,
                bits_per_ms: d.bits_per_ms,
                dominant_budget_fraction: d.dominant_budget_fraction,
                bits_per_budget_fraction: d.bits_per_budget_fraction,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        compare_optional_density(a.bits_per_budget_fraction, b.bits_per_budget_fraction)
            .then_with(|| b.bits_about.total_cmp(&a.bits_about))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(SignalDensityReport {
        note: "ranked by signal density: bits per dominant budget fraction \
               descending, then raw bits, with no CPU-only pre-rank; \
               packed_panel is the source of truth when --panel-budget-json \
               is supplied"
            .to_string(),
        ranked,
    })
}

pub(crate) fn remaining_budget(
    budget: PanelResourceBudget,
    used: ResourceUsage,
) -> PanelResourceBudget {
    let remaining = budget_usage(budget).remaining_after(used);
    PanelResourceBudget {
        max_vram_mb: remaining.vram_mb,
        max_ram_mb: remaining.ram_mb,
        max_ms_per_input: remaining.ms_per_input,
    }
}

pub(crate) fn budget_usage(budget: PanelResourceBudget) -> ResourceUsage {
    ResourceUsage {
        vram_mb: budget.max_vram_mb,
        ram_mb: budget.max_ram_mb,
        ms_per_input: budget.max_ms_per_input,
    }
}

pub(crate) fn packed_panel_report(
    budget: PanelResourceBudget,
    selected: Vec<PanelLensDecision>,
    mut rejected: Vec<PanelLensDecision>,
    used: ResourceUsage,
) -> PanelPackingReport {
    rejected.sort_by(|left, right| left.lens.cmp(&right.lens));
    let total_signal_bits = selected
        .iter()
        .map(|decision| decision.signal_bits)
        .sum::<f32>();
    let remaining = budget_usage(budget).remaining_after(used);
    let dominant_fraction = fraction(used.vram_mb, budget.max_vram_mb)
        .max(fraction(used.ram_mb, budget.max_ram_mb))
        .max(fraction(used.ms_per_input, budget.max_ms_per_input));
    PanelPackingReport {
        budget,
        selected,
        rejected,
        evicted_lenses: Vec::new(),
        total_signal_bits,
        used,
        remaining,
        aggregate_bits_per_vram_mb: density_axis(total_signal_bits, used.vram_mb),
        aggregate_bits_per_ram_mb: density_axis(total_signal_bits, used.ram_mb),
        aggregate_bits_per_ms: density_axis(total_signal_bits, used.ms_per_input),
        aggregate_bits_per_budget_fraction: density_axis(total_signal_bits, dominant_fraction),
    }
}

fn compare_optional_density(left: Option<f32>, right: Option<f32>) -> std::cmp::Ordering {
    match (left, right) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(left), Some(right)) => right.total_cmp(&left),
    }
}

fn density_axis(bits: f32, resource: f32) -> Option<f32> {
    if resource <= 1e-6 {
        None
    } else {
        Some(bits / resource)
    }
}

fn fraction(used: f32, budget: f32) -> f32 {
    if used <= 1e-6 {
        0.0
    } else if budget <= 1e-6 {
        1.0e30
    } else {
        used / budget
    }
}
