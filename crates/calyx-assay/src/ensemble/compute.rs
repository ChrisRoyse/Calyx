use std::collections::BTreeSet;

use calyx_core::{CalyxError, Result};

use crate::attribution::per_sensor_attribution;
use crate::estimate::TrustTag;
use crate::formulas::marginal_value;
use crate::ksg::MIN_ASSAY_SAMPLES;
use crate::logistic::{logistic_probe_mi_multiseed, logistic_probe_mi_multiseed_calibrated};
use crate::n_eff::stable_rank;
use crate::nmi::partitioned_histogram_nmi;
use crate::sufficiency::{PanelSufficiency, entropy_bits, panel_sufficiency_from_estimate};

use super::a37::a37_diversity_gate;
use super::model::{
    DeficitProposal, ENSEMBLE_CARD_PID_METHOD, ENSEMBLE_CARD_SCHEMA_VERSION, EnsembleCard,
    EnsembleConfig, EnsembleDecision, EnsembleLensInput, EnsembleLensValue, EnsemblePairValue,
    MIN_ENSEMBLE_PANEL_LENSES, PidBits,
};

pub const CALYX_ASSAY_PANEL_TOO_SMALL: &str = "CALYX_ASSAY_PANEL_TOO_SMALL";

pub fn ensemble_card(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
    config: &EnsembleConfig,
) -> Result<EnsembleCard> {
    validate_inputs(lenses, labels, groups, config)?;
    let groups = groups.map(|value| value as &[String]);
    let solo = lens_estimates(lenses, labels, groups)?;
    let panel = estimate_panel(lenses, labels, groups)?;
    let mut pairs = pair_values(lenses, labels, groups, &solo, config.nmi_bins)?;
    pairs.sort_by(|left, right| {
        left.a
            .cmp(&right.a)
            .then(left.b.cmp(&right.b))
            .then(left.slot_a.cmp(&right.slot_a))
    });
    let max_corr = max_pairwise(lenses, &pairs, |pair| pair.corr);
    let max_nmi = max_pairwise(lenses, &pairs, |pair| pair.nmi);
    let max_synergy = max_pairwise(lenses, &pairs, |pair| pair.synergy_gain_bits);
    let n_eff = stable_rank(&correlation_matrix(lenses, &pairs)).n_eff;

    let mut lens_values = Vec::with_capacity(lenses.len());
    for (idx, lens) in lenses.iter().enumerate() {
        let without = estimate_panel_without(lenses, labels, groups, idx)?;
        let marginal = marginal_value(panel.estimate.bits, without.estimate.bits)?;
        let marginal_ci = [
            (panel.estimate.ci_low - without.estimate.ci_high).max(0.0),
            (panel.estimate.ci_high - without.estimate.ci_low).max(marginal),
        ];
        let pid = PidBits {
            unique_bits: marginal,
            redundant_bits: (solo[idx].estimate.bits - marginal).max(0.0),
            synergistic_bits: max_synergy[idx],
        };
        let (decision, reason) = decision_for(
            marginal,
            max_corr[idx],
            max_nmi[idx],
            config.min_marginal_bits,
            config.max_redundancy,
        );
        lens_values.push(EnsembleLensValue {
            name: lens.name.clone(),
            slot: lens.slot,
            solo_bits: solo[idx].estimate.bits,
            solo_ci: [solo[idx].estimate.ci_low, solo[idx].estimate.ci_high],
            panel_without_bits: without.estimate.bits,
            marginal_bits: marginal,
            marginal_ci,
            pid,
            max_pairwise_corr: max_corr[idx],
            max_pairwise_nmi: max_nmi[idx],
            decision,
            decision_reason: reason,
        });
    }

    let slot_bits = lens_values
        .iter()
        .map(|lens| (lens.slot, lens.marginal_bits))
        .collect::<Vec<_>>();
    let a37_diversity = a37_diversity_gate(&lens_values, &pairs, n_eff, config);
    let sufficiency = panel_sufficiency_from_estimate(
        &panel.estimate,
        entropy_bits(labels),
        &per_sensor_attribution(&slot_bits, config.min_marginal_bits),
        TrustTag::Provisional,
    )?;
    let (keep_count, park_count, retire_count) = decision_counts(&lens_values);
    Ok(EnsembleCard {
        schema_version: ENSEMBLE_CARD_SCHEMA_VERSION,
        source: config.source.clone(),
        pid_method: ENSEMBLE_CARD_PID_METHOD.to_string(),
        panel_lens_count: lenses.len(),
        n_samples: labels.len(),
        anchor_entropy_bits: entropy_bits(labels),
        panel_bits: panel.estimate.bits,
        panel_ci: [panel.estimate.ci_low, panel.estimate.ci_high],
        n_eff,
        sufficient: sufficiency.sufficient,
        deficit_bits: sufficiency.deficit_bits,
        a37_diversity,
        deficit_proposal: deficit_proposal(&sufficiency, &lens_values),
        sufficiency,
        lenses: lens_values,
        pairs,
        keep_count,
        park_count,
        retire_count,
    })
}

fn validate_inputs(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
    config: &EnsembleConfig,
) -> Result<()> {
    if lenses.len() < MIN_ENSEMBLE_PANEL_LENSES {
        return Err(panel_too_small(format!(
            "ensemble verdicts require at least {MIN_ENSEMBLE_PANEL_LENSES} lenses; got {}",
            lenses.len()
        )));
    }
    if lenses.len() < config.min_gate_lenses {
        return Err(panel_too_small(format!(
            "gate ensemble verdicts require at least {} lenses; got {}",
            config.min_gate_lenses,
            lenses.len()
        )));
    }
    if labels.len() < MIN_ASSAY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "ensemble assay requires at least {MIN_ASSAY_SAMPLES} labeled samples; got {}",
            labels.len()
        )));
    }
    if labels.iter().all(|value| *value) || labels.iter().all(|value| !*value) {
        return Err(CalyxError::assay_insufficient_samples(
            "ensemble anchor labels must contain both classes",
        ));
    }
    if !config.min_marginal_bits.is_finite() || config.min_marginal_bits < 0.0 {
        return Err(CalyxError::assay_low_signal(
            "min_marginal_bits must be finite and non-negative",
        ));
    }
    if !config.max_redundancy.is_finite() || !(0.0..=1.0).contains(&config.max_redundancy) {
        return Err(CalyxError::assay_redundant(
            "max_redundancy must be finite and within [0,1]",
        ));
    }
    if let Some(groups) = groups
        && groups.len() != labels.len()
    {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "ensemble groups len {} != labels len {}",
            groups.len(),
            labels.len()
        )));
    }
    let mut slots = BTreeSet::new();
    for lens in lenses {
        if !slots.insert(lens.slot) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "duplicate ensemble slot {}",
                lens.slot
            )));
        }
        validate_lens(lens, labels.len())?;
    }
    Ok(())
}

fn validate_lens(lens: &EnsembleLensInput, expected_rows: usize) -> Result<()> {
    if lens.name.trim().is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "ensemble lens name must not be empty",
        ));
    }
    if lens.vectors.len() != expected_rows {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "lens {} rows {} != labels {}",
            lens.name,
            lens.vectors.len(),
            expected_rows
        )));
    }
    let dim = lens.vectors.first().map(Vec::len).unwrap_or(0);
    if dim == 0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "lens {} has empty vectors",
            lens.name
        )));
    }
    for (row_idx, row) in lens.vectors.iter().enumerate() {
        if row.len() != dim {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "lens {} row {} dim {} != {}",
                lens.name,
                row_idx,
                row.len(),
                dim
            )));
        }
        if row.iter().any(|value| !value.is_finite()) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "lens {} row {} contains non-finite value",
                lens.name, row_idx
            )));
        }
    }
    Ok(())
}

fn lens_estimates(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
) -> Result<Vec<crate::LogisticProbeReport>> {
    lenses
        .iter()
        .map(|lens| logistic_probe_mi_multiseed_calibrated(&lens.vectors, labels, groups))
        .collect()
}

fn estimate_panel(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
) -> Result<crate::LogisticProbeReport> {
    logistic_probe_mi_multiseed_calibrated(&concat_lenses(lenses, None), labels, groups)
}

fn estimate_panel_without(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
    excluded: usize,
) -> Result<crate::LogisticProbeReport> {
    logistic_probe_mi_multiseed(&concat_lenses(lenses, Some(excluded)), labels, groups)
}

fn pair_values(
    lenses: &[EnsembleLensInput],
    labels: &[bool],
    groups: Option<&[String]>,
    solo: &[crate::LogisticProbeReport],
    bins: usize,
) -> Result<Vec<EnsemblePairValue>> {
    let mut pairs = Vec::new();
    for a in 0..lenses.len() {
        for b in (a + 1)..lenses.len() {
            let pair_rows = concat_pair(&lenses[a], &lenses[b]);
            let pair = logistic_probe_mi_multiseed(&pair_rows, labels, groups)?;
            let sig_a = row_signature(&lenses[a].vectors);
            let sig_b = row_signature(&lenses[b].vectors);
            let nmi = partitioned_histogram_nmi(&sig_a, &sig_b, bins)?.nmi;
            let corr = pearson_abs(&sig_a, &sig_b);
            pairs.push(EnsemblePairValue {
                a: lenses[a].name.clone(),
                b: lenses[b].name.clone(),
                slot_a: lenses[a].slot,
                slot_b: lenses[b].slot,
                corr,
                nmi,
                pair_bits: pair.estimate.bits,
                pair_ci: [pair.estimate.ci_low, pair.estimate.ci_high],
                synergy_gain_bits: (pair.estimate.bits
                    - solo[a].estimate.bits.max(solo[b].estimate.bits))
                .max(0.0),
            });
        }
    }
    Ok(pairs)
}

fn concat_lenses(lenses: &[EnsembleLensInput], excluded: Option<usize>) -> Vec<Vec<f32>> {
    let rows = lenses.first().map(|lens| lens.vectors.len()).unwrap_or(0);
    let mut joint = vec![Vec::new(); rows];
    for (idx, lens) in lenses.iter().enumerate() {
        if excluded == Some(idx) {
            continue;
        }
        for (sample, row) in lens.vectors.iter().enumerate() {
            joint[sample].extend_from_slice(row);
        }
    }
    joint
}

fn concat_pair(a: &EnsembleLensInput, b: &EnsembleLensInput) -> Vec<Vec<f32>> {
    a.vectors
        .iter()
        .zip(&b.vectors)
        .map(|(left, right)| left.iter().chain(right).copied().collect())
        .collect()
}

fn row_signature(rows: &[Vec<f32>]) -> Vec<f32> {
    rows.iter()
        .map(|row| row.iter().sum::<f32>() / row.len().max(1) as f32)
        .collect()
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

fn max_pairwise<F>(lenses: &[EnsembleLensInput], pairs: &[EnsemblePairValue], value: F) -> Vec<f32>
where
    F: Fn(&EnsemblePairValue) -> f32,
{
    let mut out = vec![0.0_f32; lenses.len()];
    for pair in pairs {
        let v = value(pair);
        if let Some(a) = slot_position(lenses, pair.slot_a) {
            out[a] = out[a].max(v);
        }
        if let Some(b) = slot_position(lenses, pair.slot_b) {
            out[b] = out[b].max(v);
        }
    }
    out
}

fn correlation_matrix(lenses: &[EnsembleLensInput], pairs: &[EnsemblePairValue]) -> Vec<Vec<f32>> {
    let mut matrix = vec![vec![0.0; lenses.len()]; lenses.len()];
    for (idx, row) in matrix.iter_mut().enumerate() {
        row[idx] = 1.0;
    }
    for pair in pairs {
        if let (Some(a), Some(b)) = (
            slot_position(lenses, pair.slot_a),
            slot_position(lenses, pair.slot_b),
        ) {
            matrix[a][b] = pair.corr;
            matrix[b][a] = pair.corr;
        }
    }
    matrix
}

fn slot_position(lenses: &[EnsembleLensInput], slot: calyx_core::SlotId) -> Option<usize> {
    lenses.iter().position(|lens| lens.slot == slot)
}

fn decision_for(
    marginal_bits: f32,
    max_corr: f32,
    max_nmi: f32,
    min_bits: f32,
    max_redundancy: f32,
) -> (EnsembleDecision, String) {
    let redundant = max_corr > max_redundancy || max_nmi > max_redundancy;
    match (marginal_bits >= min_bits, redundant) {
        (true, false) => (
            EnsembleDecision::Keep,
            format!("marginal_bits >= {min_bits:.6} and redundancy <= {max_redundancy:.6}"),
        ),
        (false, true) => (
            EnsembleDecision::Retire,
            format!("marginal_bits < {min_bits:.6} and redundancy > {max_redundancy:.6}"),
        ),
        (false, false) => (
            EnsembleDecision::Park,
            format!("marginal_bits < {min_bits:.6}; await better anchor/lens evidence"),
        ),
        (true, true) => (
            EnsembleDecision::Park,
            format!("signal present but redundancy > {max_redundancy:.6}; inspect pair terms"),
        ),
    }
}

fn decision_counts(lenses: &[EnsembleLensValue]) -> (usize, usize, usize) {
    let keep = lenses
        .iter()
        .filter(|lens| lens.decision == EnsembleDecision::Keep)
        .count();
    let park = lenses
        .iter()
        .filter(|lens| lens.decision == EnsembleDecision::Park)
        .count();
    let retire = lenses
        .iter()
        .filter(|lens| lens.decision == EnsembleDecision::Retire)
        .count();
    (keep, park, retire)
}

fn deficit_proposal(
    sufficiency: &PanelSufficiency,
    lenses: &[EnsembleLensValue],
) -> Option<DeficitProposal> {
    if sufficiency.deficit_bits <= 0.0 {
        return None;
    }
    let mut ordered = lenses.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.marginal_bits.total_cmp(&right.marginal_bits));
    Some(DeficitProposal {
        action: "propose_lens".to_string(),
        deficit_bits: sufficiency.deficit_bits,
        weakest_slots: ordered.iter().take(3).map(|lens| lens.slot).collect(),
        reason: "panel_bits below anchor entropy; propose a lens against weakest marginal slots"
            .to_string(),
    })
}

fn panel_too_small(message: String) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_PANEL_TOO_SMALL,
        message,
        remediation: "run Assay over a real panel with at least ten frozen lenses",
    }
}
