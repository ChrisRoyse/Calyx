mod plan;
mod sketch;

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CalyxError, Result, SlotId};

use crate::ksg::MIN_ASSAY_SAMPLES;
use crate::nmi::partitioned_histogram_nmi;

use super::model::{
    EnsembleLensInput, EnsemblePairRedundancyEvidence, EnsembleRedundancyEvidence,
    EnsembleRedundancyMethod,
};

pub use plan::{
    DEFAULT_LINEAR_CKA_SEED, LINEAR_CKA_JACKKNIFE_BLOCKS, LINEAR_CKA_TUPLES_PER_ROW,
    LinearCkaTuplePlan, MAX_LINEAR_CKA_TUPLES, MIN_LINEAR_CKA_TUPLES, linear_cka_tuple_plan,
};
pub use sketch::{LinearCkaSketch, linear_cka_sketch_from_row_fn, linear_cka_sketch_from_rows};

pub const LINEAR_CKA_REDUNDANCY_METHOD: &str = "debiased_linear_cka_hsic1_u4_v1";

#[derive(Clone, Debug)]
pub struct EnsembleRedundancySketchInput {
    name: String,
    slot: SlotId,
    nmi_signature: Vec<f32>,
    linear_cka: LinearCkaSketch,
}

impl EnsembleRedundancySketchInput {
    pub fn new(
        name: impl Into<String>,
        slot: SlotId,
        nmi_signature: Vec<f32>,
        linear_cka: LinearCkaSketch,
    ) -> Self {
        Self {
            name: name.into(),
            slot,
            nmi_signature,
            linear_cka,
        }
    }
}

pub fn ensemble_redundancy_from_lenses(
    lenses: &[EnsembleLensInput],
    nmi_bins: usize,
) -> Result<EnsembleRedundancyEvidence> {
    let row_count = lenses.first().map(|lens| lens.vectors.len()).unwrap_or(0);
    let plan = linear_cka_tuple_plan(row_count)?;
    let mut sketches = Vec::with_capacity(lenses.len());
    for lens in lenses {
        let linear_cka = linear_cka_sketch_from_rows(&plan, &lens.vectors)?;
        sketches.push(EnsembleRedundancySketchInput::new(
            lens.name.clone(),
            lens.slot,
            row_signature(&lens.vectors)?,
            linear_cka,
        ));
    }
    ensemble_redundancy_from_sketches(&plan, &sketches, nmi_bins)
}

pub fn ensemble_redundancy_from_sketches(
    plan: &LinearCkaTuplePlan,
    lenses: &[EnsembleRedundancySketchInput],
    nmi_bins: usize,
) -> Result<EnsembleRedundancyEvidence> {
    validate_sketch_inputs(plan, lenses)?;
    let mut pairs = Vec::new();
    for a in 0..lenses.len() {
        for b in (a + 1)..lenses.len() {
            let linear_cka = sketch::estimate_pair(
                &lenses[a].linear_cka,
                &lenses[b].linear_cka,
                plan.is_exact(),
            )?;
            let nmi = partitioned_histogram_nmi(
                &lenses[a].nmi_signature,
                &lenses[b].nmi_signature,
                nmi_bins,
            )?
            .nmi;
            pairs.push(EnsemblePairRedundancyEvidence {
                a: lenses[a].name.clone(),
                b: lenses[b].name.clone(),
                slot_a: lenses[a].slot,
                slot_b: lenses[b].slot,
                linear_cka,
                nmi,
            });
        }
    }
    Ok(EnsembleRedundancyEvidence {
        method: redundancy_method(plan),
        pairs,
    })
}

pub(super) fn validate_evidence(
    lenses: &[EnsembleLensInput],
    evidence: &EnsembleRedundancyEvidence,
) -> Result<()> {
    let method = &evidence.method;
    if method.metric != LINEAR_CKA_REDUNDANCY_METHOD
        || method.row_count < MIN_ASSAY_SAMPLES
        || method.tuple_count == 0
        || method.seed_hex.trim().is_empty()
        || method.tuple_plan_blake3.len() != 64
    {
        return Err(CalyxError::assay_degenerate_input(
            "ensemble redundancy method metadata is incomplete or unsupported",
        ));
    }
    let expected_pairs = lenses.len().saturating_sub(1) * lenses.len() / 2;
    if evidence.pairs.len() != expected_pairs {
        return Err(CalyxError::assay_degenerate_input(format!(
            "ensemble redundancy pairs {} != expected {expected_pairs}",
            evidence.pairs.len()
        )));
    }
    let roster = lenses
        .iter()
        .map(|lens| (lens.slot, lens.name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut pair_keys = BTreeSet::new();
    for pair in &evidence.pairs {
        let Some(expected_a) = roster.get(&pair.slot_a) else {
            return Err(CalyxError::assay_degenerate_input(format!(
                "redundancy evidence has unknown slot {}",
                pair.slot_a
            )));
        };
        let Some(expected_b) = roster.get(&pair.slot_b) else {
            return Err(CalyxError::assay_degenerate_input(format!(
                "redundancy evidence has unknown slot {}",
                pair.slot_b
            )));
        };
        if pair.slot_a == pair.slot_b || pair.a != *expected_a || pair.b != *expected_b {
            return Err(CalyxError::assay_degenerate_input(
                "redundancy evidence pair names do not match its slots",
            ));
        }
        let key = if pair.slot_a < pair.slot_b {
            (pair.slot_a, pair.slot_b)
        } else {
            (pair.slot_b, pair.slot_a)
        };
        if !pair_keys.insert(key) {
            return Err(CalyxError::assay_degenerate_input(
                "redundancy evidence contains a duplicate pair",
            ));
        }
        validate_pair(pair)?;
    }
    Ok(())
}

fn validate_pair(pair: &EnsemblePairRedundancyEvidence) -> Result<()> {
    let estimate = &pair.linear_cka;
    let expected_point = estimate.raw_signed_point.max(0.0);
    let valid = estimate.raw_signed_point.is_finite()
        && (-1.0..=1.0).contains(&estimate.raw_signed_point)
        && estimate.redundancy_point.is_finite()
        && (0.0..=1.0).contains(&estimate.redundancy_point)
        && (estimate.redundancy_point - expected_point).abs() <= 1.0e-5
        && estimate.mc_standard_error.is_finite()
        && estimate.mc_standard_error >= 0.0
        && estimate.mc_gate_upper_estimate.is_finite()
        && (estimate.redundancy_point..=1.0).contains(&estimate.mc_gate_upper_estimate)
        && pair.nmi.is_finite()
        && (0.0..=1.0).contains(&pair.nmi);
    if !valid {
        return Err(CalyxError::assay_degenerate_input(format!(
            "invalid redundancy evidence for {} and {}",
            pair.a, pair.b
        )));
    }
    Ok(())
}

fn validate_sketch_inputs(
    plan: &LinearCkaTuplePlan,
    lenses: &[EnsembleRedundancySketchInput],
) -> Result<()> {
    if plan.row_count() < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "ensemble redundancy requires at least {MIN_ASSAY_SAMPLES} rows; got {}",
            plan.row_count()
        )));
    }
    let mut slots = BTreeSet::new();
    let mut names = BTreeSet::new();
    for lens in lenses {
        if !slots.insert(lens.slot) || !names.insert(lens.name.as_str()) {
            return Err(CalyxError::assay_insufficient_samples(
                "ensemble redundancy lens names and slots must be unique",
            ));
        }
        if !lens.linear_cka.matches(plan) {
            return Err(CalyxError::assay_degenerate_input(format!(
                "lens {} was not sketched with the shared tuple plan",
                lens.name
            )));
        }
        if lens.nmi_signature.len() != plan.row_count() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "lens {} NMI signature rows {} != {}",
                lens.name,
                lens.nmi_signature.len(),
                plan.row_count()
            )));
        }
    }
    Ok(())
}

fn redundancy_method(plan: &LinearCkaTuplePlan) -> EnsembleRedundancyMethod {
    EnsembleRedundancyMethod {
        metric: LINEAR_CKA_REDUNDANCY_METHOD.to_string(),
        tuple_design: if plan.is_exact() {
            "complete_four_subset_enumeration_v1"
        } else {
            "blake3_counter_uniform_four_distinct_with_replacement_v1"
        }
        .to_string(),
        row_count: plan.row_count(),
        tuple_count: plan.tuple_count(),
        seed_hex: format!("0x{:016x}", plan.seed()),
        tuple_plan_blake3: plan.digest_hex(),
        exact: plan.is_exact(),
        uncertainty_method: if plan.is_exact() {
            "none_complete_tuple_population"
        } else {
            "delete_32_group_jackknife_ratio_v1"
        }
        .to_string(),
        uncertainty_blocks: if plan.is_exact() {
            0
        } else {
            LINEAR_CKA_JACKKNIFE_BLOCKS
        },
        gate_score_method: "max_0_raw_plus_4_mc_se_clamped_1_fail_closed_v1".to_string(),
    }
}

fn row_signature(rows: &[Vec<f32>]) -> Result<Vec<f32>> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.is_empty() || row.iter().any(|value| !value.is_finite()) {
                return Err(CalyxError::assay_degenerate_input(format!(
                    "ensemble NMI signature row {index} is empty or non-finite"
                )));
            }
            Ok((row.iter().map(|value| f64::from(*value)).sum::<f64>() / row.len() as f64) as f32)
        })
        .collect()
}

#[cfg(test)]
pub(super) fn test_tuple_z(rows: [&[f32]; 4]) -> Result<[f64; 3]> {
    sketch::tuple_z(rows, 1.0)
}

#[cfg(test)]
pub(super) fn test_pair_estimate(
    left: &LinearCkaSketch,
    right: &LinearCkaSketch,
    exact: bool,
) -> Result<super::model::LinearCkaEstimate> {
    sketch::estimate_pair(left, right, exact)
}

#[cfg(test)]
pub(super) fn test_tuples(plan: &LinearCkaTuplePlan) -> &[[usize; 4]] {
    plan.tuples()
}
