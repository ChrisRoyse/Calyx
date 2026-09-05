use std::collections::BTreeMap;

use calyx_assay::{
    MIN_ASSAY_SAMPLES, MiEstimate, entropy_bits, ksg_mi_continuous_discrete_cuda_strict,
    partitioned_histogram_nmi,
};
use calyx_core::{
    AnchorKind, AnchorValue, CalyxError, Constellation, CxId, Modality, Panel, Slot, SlotVector,
};

mod representation;

#[cfg(test)]
mod estimator_tests;

use representation::{assay_block_features, assay_vector, cosine_to_centroid};

use super::model::{BitsExplainOut, BitsOut, PairRedundancyOut, SlotBitsOut, hex};
use crate::error::{CliError, CliResult};

const SCHEMA_VERSION: u32 = 2;
const K: usize = 3;
const SAMPLE_TARGET: usize = 192;
const MIN_CLASS_SAMPLE: usize = 32;
const MAX_SAMPLE: usize = 2_048;
const LOW_SIGNAL_BITS: f64 = 0.05;
const ESTIMATOR: &str = "ross_mixed_continuous_discrete_ksg_k3_cuda_strict_no_replacement_999";
const PAIR_ESTIMATOR: &str = "partitioned_histogram_nmi";
const PANEL_REPRESENTATION: &str = "equal_slot_l2_block_concatenation";

struct LabeledDoc<'a> {
    cx_id: CxId,
    cx: &'a Constellation,
    label: String,
}

struct SlotMeasurement {
    slot: u16,
    name: String,
    representation: String,
    vectors: Vec<Vec<f32>>,
    estimate: MiEstimate,
    centroid_scores: Vec<f32>,
}

pub(super) fn calculate(
    panel: &Panel,
    docs: &BTreeMap<CxId, Constellation>,
    anchor: &AnchorKind,
    label: &str,
    explain: bool,
    key: &[u8],
) -> CliResult<BitsOut> {
    let population = load_categorical_population(docs, anchor, label)?;
    let population_classes = class_counts(population.iter().map(|row| row.label.as_str()));
    validate_population(label, population.len(), &population_classes)?;
    let modality = single_population_modality(&population, label)?;
    let slots = panel
        .slots
        .iter()
        .filter(|slot| slot.counts_toward_degraded(modality))
        .collect::<Vec<_>>();
    if slots.is_empty() {
        return Err(CalyxError::assay_degenerate_input(format!(
            "categorical bits for {label} requires at least one active primary {modality:?} content slot"
        ))
        .into());
    }
    let selected = deterministic_balanced_sample(&population, label, &population_classes)?;

    let class_codes = population_classes
        .keys()
        .enumerate()
        .map(|(index, value)| (value.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let labels = selected
        .iter()
        .map(|row| class_codes[&row.label])
        .collect::<Vec<_>>();
    let sample_classes = class_counts(selected.iter().map(|row| row.label.as_str()));
    let sample_entropy = f64::from(entropy_bits(&labels));
    let population_entropy = entropy_from_counts(&population_classes);
    if !sample_entropy.is_finite() || sample_entropy <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(format!(
            "categorical bits for {label} has non-positive sampled outcome entropy: {sample_entropy}"
        ))
        .into());
    }

    let mut measurements = Vec::with_capacity(slots.len());
    for slot in slots {
        measurements.push(measure_slot(slot, &selected, &labels, label)?);
    }

    let panel_vectors = concatenate_panel(&measurements)?;
    let panel_estimate = ksg_mi_continuous_discrete_cuda_strict(&panel_vectors, &labels, K)
        .map_err(|error| contextual_assay_error(error, format!(
            "joint panel categorical assay failed for anchor {label} using {PANEL_REPRESENTATION}"
        )))?;
    validate_estimate("joint panel", &panel_estimate, selected.len())?;

    let pairwise_redundancy = pairwise_redundancy(&measurements)?;
    let mut per_slot = measurements
        .iter()
        .map(|measurement| SlotBitsOut {
            slot: measurement.slot,
            name: measurement.name.clone(),
            n: measurement.estimate.n_samples,
            bits: f64::from(measurement.estimate.bits),
            ci: [
                f64::from(measurement.estimate.ci_low),
                f64::from(measurement.estimate.ci_high),
            ],
            estimator: ESTIMATOR.to_string(),
            representation: measurement.representation.clone(),
            trust: format!("{:?}", measurement.estimate.trust).to_ascii_lowercase(),
            state: "active".to_string(),
            low_signal: f64::from(measurement.estimate.bits) < LOW_SIGNAL_BITS,
        })
        .collect::<Vec<_>>();
    per_slot.sort_by(|left, right| {
        right
            .bits
            .total_cmp(&left.bits)
            .then_with(|| left.slot.cmp(&right.slot))
    });

    let panel_bits = f64::from(panel_estimate.bits);
    let panel_ci = [
        f64::from(panel_estimate.ci_low),
        f64::from(panel_estimate.ci_high),
    ];
    Ok(BitsOut {
        schema_version: SCHEMA_VERSION,
        anchor: label.to_string(),
        panel_sufficiency: (panel_bits / sample_entropy).clamp(0.0, 1.0),
        n: selected.len(),
        dpi_ceiling: sample_entropy,
        per_slot,
        population_n: population.len(),
        outcome_classes: sample_classes,
        population_outcome_classes: population_classes,
        population_outcome_entropy_bits: population_entropy,
        sample_cx_ids: selected.iter().map(|row| row.cx_id.to_string()).collect(),
        panel_bits,
        panel_ci,
        sufficiency_passed: panel_ci[0] >= sample_entropy,
        pairwise_redundancy,
        explain: explain.then(|| BitsExplainOut {
            positive_anchor_count: selected.len(),
            comparison_count: 0,
            persisted_cf: "assay".to_string(),
            persisted_key_hex: hex(key),
            outcome_mode: "categorical_enum_or_text_exactly_one_anchor_per_cx".to_string(),
            sample_policy: format!(
                "cx_id_ordered_equal_class_stratified_target_{SAMPLE_TARGET}_min_{MIN_CLASS_SAMPLE}_per_class"
            ),
            strict_cuda_required: true,
        }),
    })
}

fn load_categorical_population<'a>(
    docs: &'a BTreeMap<CxId, Constellation>,
    anchor: &AnchorKind,
    label: &str,
) -> CliResult<Vec<LabeledDoc<'a>>> {
    let mut rows = Vec::with_capacity(docs.len());
    for (cx_id, cx) in docs {
        let matches = cx
            .anchors
            .iter()
            .filter(|candidate| &candidate.kind == anchor)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            continue;
        }
        if matches.len() != 1 {
            return Err(CalyxError::assay_degenerate_input(format!(
                "categorical bits for {label} permits at most one matching anchor on cx {cx_id}; found {}",
                matches.len()
            ))
            .into());
        }
        let value = match &matches[0].value {
            AnchorValue::Enum(value) | AnchorValue::Text(value) => value.trim(),
            other => {
                return Err(CalyxError::assay_degenerate_input(format!(
                    "categorical bits for {label} requires enum/text values; cx {cx_id} stores {other:?}"
                ))
                .into());
            }
        };
        if value.is_empty() {
            return Err(CalyxError::assay_degenerate_input(format!(
                "categorical bits for {label} has an empty category on cx {cx_id}"
            ))
            .into());
        }
        rows.push(LabeledDoc {
            cx_id: *cx_id,
            cx,
            label: value.to_string(),
        });
    }
    Ok(rows)
}

fn validate_population(
    label: &str,
    population_n: usize,
    classes: &BTreeMap<String, usize>,
) -> CliResult {
    if population_n < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "categorical bits for {label} requires at least {MIN_ASSAY_SAMPLES} stored outcomes; got {population_n}"
        ))
        .into());
    }
    if classes.len() < 2 {
        return Err(CalyxError::assay_degenerate_input(format!(
            "categorical bits for {label} requires at least two outcome classes; got {classes:?}"
        ))
        .into());
    }
    Ok(())
}

fn single_population_modality(population: &[LabeledDoc<'_>], label: &str) -> CliResult<Modality> {
    let modality = population
        .first()
        .map(|row| row.cx.modality)
        .ok_or_else(|| {
            CliError::from(CalyxError::assay_insufficient_samples(format!(
                "categorical bits for {label} has no anchored outcomes"
            )))
        })?;
    if let Some(row) = population.iter().find(|row| row.cx.modality != modality) {
        return Err(CalyxError::assay_degenerate_input(format!(
            "categorical bits for {label} requires one modality per assay; expected {modality:?}, cx {} is {:?}",
            row.cx_id, row.cx.modality
        ))
        .into());
    }
    Ok(modality)
}

fn deterministic_balanced_sample<'rows, 'docs>(
    population: &'rows [LabeledDoc<'docs>],
    label: &str,
    classes: &BTreeMap<String, usize>,
) -> CliResult<Vec<&'rows LabeledDoc<'docs>>> {
    let class_count = classes.len();
    let target_per_class = SAMPLE_TARGET.div_ceil(class_count).max(MIN_CLASS_SAMPLE);
    let sample_n = target_per_class.checked_mul(class_count).ok_or_else(|| {
        CliError::from(CalyxError::assay_insufficient_samples(format!(
            "categorical bits sample size overflow for {class_count} classes"
        )))
    })?;
    if sample_n > MAX_SAMPLE {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "categorical bits for {label} would require {sample_n} rows for {class_count} classes (max {MAX_SAMPLE})"
        ))
        .into());
    }
    if let Some((class, count)) = classes.iter().find(|(_, count)| **count < target_per_class) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "categorical bits for {label} requires {target_per_class} deterministic rows per class for stable k={K} intervals; class {class:?} has {count}"
        ))
        .into());
    }

    let mut selected_per_class = BTreeMap::<String, usize>::new();
    let mut selected = Vec::with_capacity(sample_n);
    for row in population {
        let count = selected_per_class.entry(row.label.clone()).or_default();
        if *count < target_per_class {
            *count += 1;
            selected.push(row);
        }
    }
    if selected.len() != sample_n
        || selected_per_class
            .values()
            .any(|count| *count != target_per_class)
    {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "categorical bits deterministic sampler did not fill its declared design: selected={} expected={sample_n} counts={selected_per_class:?}",
            selected.len()
        ))
        .into());
    }
    Ok(selected)
}

fn measure_slot(
    slot: &Slot,
    selected: &[&LabeledDoc<'_>],
    labels: &[usize],
    anchor_label: &str,
) -> CliResult<SlotMeasurement> {
    let mut rows = Vec::with_capacity(selected.len());
    for row in selected {
        let stored = row.cx.slots.get(&slot.slot_id).ok_or_else(|| {
            CliError::from(CalyxError::assay_degenerate_input(format!(
                "active slot {} ({}) is missing from sampled cx {}",
                slot.slot_id,
                slot.slot_key.key(),
                row.cx_id
            )))
        })?;
        rows.push((row.cx_id, stored));
    }
    let (vectors, representation) = measure_slot_vectors(slot, &rows)?;
    let estimate = ksg_mi_continuous_discrete_cuda_strict(&vectors, labels, K).map_err(|error| {
        contextual_assay_error(
            error,
            format!(
                "categorical assay failed for anchor {anchor_label}, slot {} ({}), representation {representation}",
                slot.slot_id,
                slot.slot_key.key()
            ),
        )
    })?;
    validate_estimate(slot.slot_key.key(), &estimate, selected.len())?;
    let centroid_scores = cosine_to_centroid(&vectors, slot)?;
    Ok(SlotMeasurement {
        slot: slot.slot_id.get(),
        name: slot.slot_key.key().to_string(),
        representation,
        vectors,
        estimate,
        centroid_scores,
    })
}

/// Produces one continuous feature vector per sampled row for a slot, choosing
/// the native estimator by declared shape: Dense/Sparse keep their per-row
/// projection; typed `Multi`/`SegmentedMulti` blocks use the native
/// ordered-block MaxSim estimator (#1892) with no pooling or flattening.
fn measure_slot_vectors(
    slot: &Slot,
    rows: &[(CxId, &SlotVector)],
) -> CliResult<(Vec<Vec<f32>>, String)> {
    let is_block = matches!(
        rows.first().map(|row| row.1),
        Some(SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. })
    );
    if is_block {
        return assay_block_features(slot, rows);
    }
    let mut vectors = Vec::with_capacity(rows.len());
    let mut representation = None::<String>;
    for (cx_id, stored) in rows {
        let (vector, current) = assay_vector(slot, *cx_id, stored)?;
        if let Some(expected) = &representation {
            if expected != &current {
                return Err(CalyxError::assay_degenerate_input(format!(
                    "active slot {} ({}) changes representation within the sample: expected {expected}, cx {cx_id} has {current}",
                    slot.slot_id,
                    slot.slot_key.key()
                ))
                .into());
            }
        } else {
            representation = Some(current);
        }
        vectors.push(vector);
    }
    let representation = representation.ok_or_else(|| {
        CliError::from(CalyxError::assay_insufficient_samples(format!(
            "active slot {} ({}) has no sampled rows",
            slot.slot_id,
            slot.slot_key.key()
        )))
    })?;
    Ok((vectors, representation))
}

fn concatenate_panel(measurements: &[SlotMeasurement]) -> CliResult<Vec<Vec<f32>>> {
    let n = measurements.first().map_or(0, |slot| slot.vectors.len());
    if measurements.is_empty() || n == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "joint categorical panel requires active measured slots and sampled rows",
        )
        .into());
    }
    let total_dim = measurements.iter().try_fold(0usize, |total, slot| {
        let dim = slot.vectors.first().map_or(0, Vec::len);
        total.checked_add(dim).ok_or_else(|| {
            CliError::from(CalyxError::assay_degenerate_input(
                "joint categorical panel dimension overflow",
            ))
        })
    })?;
    let mut panel = Vec::with_capacity(n);
    for row_index in 0..n {
        let mut row = Vec::with_capacity(total_dim);
        for slot in measurements {
            let vector = slot.vectors.get(row_index).ok_or_else(|| {
                CliError::from(CalyxError::assay_degenerate_input(format!(
                    "joint categorical panel row mismatch: slot {} has {} rows, expected {n}",
                    slot.slot,
                    slot.vectors.len()
                )))
            })?;
            row.extend_from_slice(vector);
        }
        panel.push(row);
    }
    Ok(panel)
}

fn pairwise_redundancy(measurements: &[SlotMeasurement]) -> CliResult<Vec<PairRedundancyOut>> {
    let n = measurements.first().map_or(0, |slot| slot.vectors.len());
    let bins = (n as f64).sqrt().round() as usize;
    let bins = bins.clamp(4, 32);
    let mut output = Vec::new();
    for left_index in 0..measurements.len() {
        for right_index in left_index + 1..measurements.len() {
            let left = &measurements[left_index];
            let right = &measurements[right_index];
            let report = partitioned_histogram_nmi(
                &left.centroid_scores,
                &right.centroid_scores,
                bins,
            )
            .map_err(|error| contextual_assay_error(error, format!(
                "pairwise redundancy failed for slots {} ({}) and {} ({}) using cosine-to-own-centroid scores with {bins} bins",
                left.slot, left.name, right.slot, right.name
            )))?;
            output.push(PairRedundancyOut {
                left_slot: left.slot,
                right_slot: right.slot,
                nmi: f64::from(report.nmi),
                mi_bits: f64::from(report.mi_bits),
                n: report.n_samples,
                estimator: PAIR_ESTIMATOR.to_string(),
                representation: format!("cosine_to_own_slot_centroid_equal_width_{bins}_bins"),
            });
        }
    }
    Ok(output)
}

fn validate_estimate(name: &str, estimate: &MiEstimate, expected_n: usize) -> CliResult {
    if estimate.n_samples != expected_n
        || !estimate.bits.is_finite()
        || !estimate.ci_low.is_finite()
        || !estimate.ci_high.is_finite()
        || estimate.ci_low > estimate.bits
        || estimate.bits > estimate.ci_high
    {
        return Err(CalyxError::assay_degenerate_input(format!(
            "categorical estimate invariant failed for {name}: n={} expected_n={expected_n} bits={} ci=[{},{}]",
            estimate.n_samples, estimate.bits, estimate.ci_low, estimate.ci_high
        ))
        .into());
    }
    Ok(())
}

fn class_counts<'a>(labels: impl Iterator<Item = &'a str>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for label in labels {
        *counts.entry(label.to_string()).or_default() += 1;
    }
    counts
}

fn entropy_from_counts(counts: &BTreeMap<String, usize>) -> f64 {
    let n = counts.values().sum::<usize>().max(1) as f64;
    counts
        .values()
        .map(|count| {
            let probability = *count as f64 / n;
            -probability * probability.log2()
        })
        .sum()
}

fn contextual_assay_error(error: CalyxError, context: String) -> CliError {
    CliError::from(CalyxError {
        code: error.code,
        message: format!("{context}: {}", error.message),
        remediation: error.remediation,
    })
}
