use calyx_assay::{PanelLensDecision, PanelPackingReport, PanelResourceBudget, ResourceUsage};
use serde::Serialize;

use super::cost::LensCostMap;
use super::data::AssayCorpus;
use super::report::LensMeasurement;
use super::selection::budget_usage;

const CONTROL_LENS_LIMIT: usize = 2;
const EPSILON: f32 = 1.0e-6;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PanelComparisonReport {
    pub(crate) note: String,
    pub(crate) control_lens_limit: usize,
    pub(crate) density_panel: ComparisonPanel,
    pub(crate) best_single_lens_control: Option<ComparisonPanel>,
    pub(crate) best_few_lens_control: Option<ComparisonPanel>,
    pub(crate) largest_resource_singleton_control: Option<ComparisonPanel>,
    pub(crate) density_panel_beats_best_few_lens_control: Option<bool>,
    pub(crate) signal_gain_bits: Option<f32>,
    pub(crate) signal_gain_ratio: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ComparisonPanel {
    pub(crate) name: String,
    pub(crate) lenses: Vec<String>,
    pub(crate) lens_count: usize,
    pub(crate) total_signal_bits: f32,
    pub(crate) used: ResourceUsage,
    pub(crate) remaining: ResourceUsage,
    pub(crate) dominant_budget_fraction: f32,
    pub(crate) aggregate_bits_per_budget_fraction: Option<f32>,
}

#[derive(Clone, Debug)]
struct Candidate {
    index: usize,
    name: String,
    bits: f32,
    usage: ResourceUsage,
}

pub(crate) fn compare_density_panel(
    corpus: &AssayCorpus,
    measurements: &[LensMeasurement],
    cost: &LensCostMap,
    budget: PanelResourceBudget,
    packed_panel: &PanelPackingReport,
    min_bits: f32,
    max_corr: f32,
) -> Result<PanelComparisonReport, String> {
    let candidates = comparison_candidates(corpus, measurements, cost, budget, min_bits)?;
    let density_panel =
        comparison_from_decisions("density_signal_panel", &packed_panel.selected, budget);
    let best_single = best_single_control(&candidates, budget);
    let best_few = best_few_control(corpus, &candidates, budget, max_corr);
    let largest_resource = largest_resource_singleton_control(&candidates, budget);
    let (beats, gain, ratio) = match &best_few {
        Some(control) => {
            let gain = density_panel.total_signal_bits - control.total_signal_bits;
            (
                Some(gain > EPSILON),
                Some(gain),
                density_axis(gain, control.total_signal_bits),
            )
        }
        None => (None, None, None),
    };
    if density_panel.lens_count > CONTROL_LENS_LIMIT && beats == Some(false) {
        return Err(format!(
            "CALYX_FSV_ASSAY_PANEL_CONTROL_NOT_BEATEN: density_panel_bits={:.6} control_bits={:.6}",
            density_panel.total_signal_bits,
            best_few
                .as_ref()
                .map(|control| control.total_signal_bits)
                .unwrap_or(0.0)
        ));
    }
    Ok(PanelComparisonReport {
        note: "real measured comparison: selected density panel vs best raw-signal \
               one- or two-lens control under the same resource budget"
            .to_string(),
        control_lens_limit: CONTROL_LENS_LIMIT,
        density_panel,
        best_single_lens_control: best_single,
        best_few_lens_control: best_few,
        largest_resource_singleton_control: largest_resource,
        density_panel_beats_best_few_lens_control: beats,
        signal_gain_bits: gain,
        signal_gain_ratio: ratio,
    })
}

fn comparison_candidates(
    corpus: &AssayCorpus,
    measurements: &[LensMeasurement],
    cost: &LensCostMap,
    budget: PanelResourceBudget,
    min_bits: f32,
) -> Result<Vec<Candidate>, String> {
    let mut candidates = Vec::new();
    for measurement in measurements {
        if corpus.lenses[measurement.index].redundant || measurement.estimate.bits < min_bits {
            continue;
        }
        let lens_cost = cost.require(&measurement.name)?;
        let usage = lens_cost.usage();
        if usage.fits_within(budget) {
            candidates.push(Candidate {
                index: measurement.index,
                name: measurement.name.clone(),
                bits: measurement.estimate.bits,
                usage,
            });
        }
    }
    Ok(candidates)
}

fn best_single_control(
    candidates: &[Candidate],
    budget: PanelResourceBudget,
) -> Option<ComparisonPanel> {
    candidates
        .iter()
        .max_by(|left, right| compare_signal(left.bits, &left.name, right.bits, &right.name))
        .map(|candidate| panel_from_candidates("best_single_lens_control", &[candidate], budget))
}

fn best_few_control(
    corpus: &AssayCorpus,
    candidates: &[Candidate],
    budget: PanelResourceBudget,
    max_corr: f32,
) -> Option<ComparisonPanel> {
    let mut best = best_single_control(candidates, budget);
    for left_idx in 0..candidates.len() {
        for right_idx in (left_idx + 1)..candidates.len() {
            let left = &candidates[left_idx];
            let right = &candidates[right_idx];
            let used = left.usage.saturating_add(right.usage);
            if !used.fits_within(budget) {
                continue;
            }
            let corr = lens_pair_correlation(
                &corpus.lens_vectors[left.index],
                &corpus.lens_vectors[right.index],
            );
            if corr > max_corr {
                continue;
            }
            let panel = panel_from_candidates("best_few_lens_control", &[left, right], budget);
            if best
                .as_ref()
                .is_none_or(|current| panel_better(&panel, current))
            {
                best = Some(panel);
            }
        }
    }
    best
}

fn largest_resource_singleton_control(
    candidates: &[Candidate],
    budget: PanelResourceBudget,
) -> Option<ComparisonPanel> {
    candidates
        .iter()
        .max_by(|left, right| {
            dominant_fraction(left.usage, budget)
                .total_cmp(&dominant_fraction(right.usage, budget))
                .then_with(|| compare_signal(left.bits, &left.name, right.bits, &right.name))
        })
        .map(|candidate| {
            panel_from_candidates("largest_resource_singleton_control", &[candidate], budget)
        })
}

fn comparison_from_decisions(
    name: &str,
    decisions: &[PanelLensDecision],
    budget: PanelResourceBudget,
) -> ComparisonPanel {
    let used = decisions
        .iter()
        .fold(ResourceUsage::default(), |sum, decision| {
            sum.saturating_add(decision.usage)
        });
    let total_signal_bits = decisions
        .iter()
        .map(|decision| decision.signal_bits)
        .sum::<f32>();
    let lenses = decisions
        .iter()
        .map(|decision| decision.lens.clone())
        .collect::<Vec<_>>();
    comparison_panel(name, lenses, total_signal_bits, used, budget)
}

fn panel_from_candidates(
    name: &str,
    candidates: &[&Candidate],
    budget: PanelResourceBudget,
) -> ComparisonPanel {
    let used = candidates
        .iter()
        .fold(ResourceUsage::default(), |sum, candidate| {
            sum.saturating_add(candidate.usage)
        });
    let total_signal_bits = candidates
        .iter()
        .map(|candidate| candidate.bits)
        .sum::<f32>();
    let lenses = candidates
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect::<Vec<_>>();
    comparison_panel(name, lenses, total_signal_bits, used, budget)
}

fn comparison_panel(
    name: &str,
    lenses: Vec<String>,
    total_signal_bits: f32,
    used: ResourceUsage,
    budget: PanelResourceBudget,
) -> ComparisonPanel {
    let dominant_budget_fraction = dominant_fraction(used, budget);
    ComparisonPanel {
        name: name.to_string(),
        lens_count: lenses.len(),
        lenses,
        total_signal_bits,
        used,
        remaining: budget_usage(budget).remaining_after(used),
        dominant_budget_fraction,
        aggregate_bits_per_budget_fraction: density_axis(
            total_signal_bits,
            dominant_budget_fraction,
        ),
    }
}

fn panel_better(left: &ComparisonPanel, right: &ComparisonPanel) -> bool {
    left.total_signal_bits > right.total_signal_bits + EPSILON
        || ((left.total_signal_bits - right.total_signal_bits).abs() <= EPSILON
            && left.lenses < right.lenses)
}

fn compare_signal(
    left_bits: f32,
    left_name: &str,
    right_bits: f32,
    right_name: &str,
) -> std::cmp::Ordering {
    left_bits
        .total_cmp(&right_bits)
        .then_with(|| right_name.cmp(left_name))
}

fn dominant_fraction(usage: ResourceUsage, budget: PanelResourceBudget) -> f32 {
    fraction(usage.vram_mb, budget.max_vram_mb)
        .max(fraction(usage.ram_mb, budget.max_ram_mb))
        .max(fraction(usage.ms_per_input, budget.max_ms_per_input))
}

fn fraction(used: f32, budget: f32) -> f32 {
    if used <= EPSILON {
        0.0
    } else if budget <= EPSILON {
        1.0e30
    } else {
        used / budget
    }
}

fn density_axis(bits: f32, resource: f32) -> Option<f32> {
    if resource <= EPSILON {
        None
    } else {
        Some(bits / resource)
    }
}

fn lens_pair_correlation(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 || a.first().map(Vec::len) != b.first().map(Vec::len) {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for (left, right) in a.iter().zip(b).take(n) {
        sum += cosine(left, right);
    }
    sum / n as f32
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
