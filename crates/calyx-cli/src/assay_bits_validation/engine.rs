use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, CorrelationEvidence, MiEstimate, PanelLensDecision,
    PanelResourceBudget, ResourceDensity, ResourceUsage, StratumBits, admit_lens_estimate,
    admit_lens_with_usage, ensure_informative_binary_labels,
    logistic_probe_mi_multiseed_calibrated, stratified_bits,
};
use calyx_aster::cf::CfRouter;
use calyx_core::{AnchorKind, SlotId, VaultId};
use ulid::Ulid;

use super::calyx_error_detail;
use super::comparison::compare_density_panel;
use super::correlation::lens_pair_correlation_evidence;
use super::cost::LensCostMap;
use super::data::AssayCorpus;
use super::report::{AssayBitsReport, LensMeasurement, LensReport, PanelReport, StratumReport};
use super::request::AssayBitsRequest;
use super::selection::{
    SelectionMeasurement, budget_usage, compute_signal_density, density_budget, density_order,
    packed_panel_report, raw_bits_order, remaining_budget,
};
use crate::assay_verdict_metadata::{calibration_status, estimate_bound_name};

const PANEL_VERSION: u32 = 70;
const CF_MEMTABLE_CAP: usize = 1_048_576;

pub(crate) fn evaluate_corpus(
    corpus: &AssayCorpus,
    request: &AssayBitsRequest,
    cost: Option<&LensCostMap>,
    panel_budget: Option<PanelResourceBudget>,
) -> Result<AssayBitsReport, String> {
    let anchor_audit = corpus.anchor_audit.clone();
    let anchor = corpus.anchor_labels(request.target_class);
    let positives = anchor.iter().filter(|&&v| v).count();
    if positives == 0 || positives == anchor.len() {
        return Err(format!(
            "CALYX_FSV_ASSAY_SINGLE_CLASS_ANCHOR: target_class={} positives={positives} total={}",
            request.target_class,
            anchor.len()
        ));
    }
    let anchor_entropy_bits =
        ensure_informative_binary_labels(&anchor).map_err(calyx_error_detail)?;

    // Per-lens bits_about about the grounded binary anchor.
    let mut measurements = Vec::with_capacity(corpus.lenses.len());
    for (index, lens) in corpus.lenses.iter().enumerate() {
        let report = logistic_probe_mi_multiseed_calibrated(
            &corpus.lens_vectors[index],
            &anchor,
            Some(&corpus.anchor_groups),
        )
        .map_err(calyx_error_detail)?;
        measurements.push(LensMeasurement {
            index,
            name: lens.name.clone(),
            redundant: lens.redundant,
            estimate: report.estimate,
        });
    }

    let selection_measurements: Vec<SelectionMeasurement> = measurements
        .iter()
        .map(SelectionMeasurement::from)
        .collect();
    let density_budget = match cost {
        Some(cost_map) => Some(density_budget(
            &selection_measurements,
            cost_map,
            panel_budget,
        )?),
        None => None,
    };
    let order = match (cost, panel_budget) {
        (Some(cost_map), Some(budget)) => density_order(&selection_measurements, cost_map, budget)?,
        _ => raw_bits_order(&selection_measurements),
    };

    let mut lens_reports: Vec<Option<LensReport>> = vec![None; measurements.len()];
    let mut admitted_indices: Vec<usize> = Vec::new();
    let mut used = ResourceUsage::default();
    let mut packed_selected = Vec::new();
    let mut packed_rejected = Vec::new();
    for &idx in &order {
        let measurement = &measurements[idx];
        let bits = measurement.estimate.bits;
        let resource = match (cost, density_budget) {
            (Some(cost_map), Some(budget)) => {
                let lens_cost = cost_map.require(&measurement.name)?;
                let usage = lens_cost.usage();
                Some((
                    usage,
                    lens_cost.placement,
                    ResourceDensity::compute(bits, usage, lens_cost.placement, budget),
                ))
            }
            _ => None,
        };
        let corr = max_corr_evidence(corpus, &measurements, &admitted_indices, measurement)?;
        let max_corr = corr.corr;
        let decision = match (resource, panel_budget) {
            (Some((usage, placement, _)), Some(budget)) => {
                let remaining = remaining_budget(budget, used);
                admit_lens_estimate(&measurement.estimate, corr).and_then(|_| {
                    admit_lens_with_usage(bits, max_corr, usage, placement, remaining).map(|_| ())
                })
            }
            _ => admit_lens_estimate(&measurement.estimate, corr).map(|_| ()),
        };
        let (admitted, rejection_reason) = match decision {
            Ok(_) => {
                if let (Some((usage, _, _)), Some(_)) = (resource, panel_budget) {
                    used = used.saturating_add(usage);
                }
                admitted_indices.push(idx);
                (true, None)
            }
            Err(error) => (false, Some(error.code.to_string())),
        };
        if let (Some((usage, placement, density)), Some(budget)) = (resource, panel_budget) {
            let remaining = if admitted {
                Some(budget_usage(budget).remaining_after(used))
            } else {
                None
            };
            let decision = PanelLensDecision {
                lens: measurement.name.clone(),
                admitted,
                resident: false,
                signal_bits: bits,
                max_pairwise_corr: max_corr,
                usage,
                placement,
                density,
                rejection_reason: rejection_reason.clone(),
                remaining_budget_after: remaining,
            };
            if admitted {
                packed_selected.push(decision);
            } else {
                packed_rejected.push(decision);
            }
        }
        lens_reports[idx] = Some(LensReport {
            name: measurement.name.clone(),
            redundant: measurement.redundant,
            bits_about: bits,
            anchor_leaks_into_input: anchor_audit.anchor_leaks_into_input,
            trivial_anchor: anchor_audit.trivial_anchor,
            grounded_gate_eligible: anchor_audit.grounded_gate_eligible,
            ci: [measurement.estimate.ci_low, measurement.estimate.ci_high],
            estimate_bound: estimate_bound_name(measurement.estimate.bound).to_string(),
            power_calibration_status: calibration_status(&measurement.estimate),
            power_recovery_ratio: measurement
                .estimate
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.recovery_ratio),
            power_recovered_bits: measurement
                .estimate
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.recovered_bits),
            power_planted_bits: measurement
                .estimate
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.planted_bits),
            seed_sigma_bits: measurement
                .estimate
                .reliability
                .as_ref()
                .map(|reliability| reliability.seed_sigma),
            seed_count: measurement
                .estimate
                .reliability
                .as_ref()
                .map(|reliability| reliability.seed_count),
            unresolved: measurement
                .estimate
                .reliability
                .as_ref()
                .is_some_and(|reliability| reliability.unresolved),
            estimator: format!("{:?}", measurement.estimate.estimator),
            max_pairwise_corr: max_corr,
            max_pairwise_corr_ci: [corr.ci_low, corr.ci_high],
            admitted,
            rejection_reason,
            density: resource.map(|(_, _, density)| density),
            usage: resource.map(|(usage, _, _)| usage),
            placement: resource.map(|(_, placement, _)| placement),
        });
    }
    let mut lenses: Vec<LensReport> = lens_reports
        .into_iter()
        .map(|report| report.expect("every lens measured"))
        .collect();

    // Fail-closed checks.
    for (lens, measurement) in lenses.iter().zip(&measurements) {
        if !measurement.redundant && measurement.estimate.ci_low < request.min_bits {
            return Err(format!(
                "CALYX_FSV_ASSAY_BITS_CI_BELOW_THRESHOLD: lens={} ci_low={:.6} bits={:.6}",
                lens.name, measurement.estimate.ci_low, measurement.estimate.bits
            ));
        }
        if measurement.redundant && lens.admitted {
            return Err(format!(
                "CALYX_FSV_ASSAY_REDUNDANT_LENS_NOT_REJECTED: lens={} corr={:.6}",
                lens.name, lens.max_pairwise_corr
            ));
        }
    }

    // Signal density: join measured bits with measured cost (#717). Only when
    // a `--cost-json` was supplied; fail-closed if any lens lacks a cost entry.
    let signal_density = match (cost, density_budget) {
        (Some(cost_map), Some(budget)) => {
            Some(compute_signal_density(&mut lenses, cost_map, budget)?)
        }
        _ => None,
    };
    let packed_panel = panel_budget
        .map(|budget| packed_panel_report(budget, packed_selected, packed_rejected, used));
    let panel_comparison = match (cost, panel_budget, &packed_panel) {
        (Some(cost_map), Some(budget), Some(panel)) => Some(compare_density_panel(
            corpus,
            &measurements,
            cost_map,
            budget,
            panel,
            request.min_bits,
            request.max_corr,
        )?),
        _ => None,
    };

    // Panel MI: concatenate admitted lens vectors per sample.
    let admitted_order: Vec<usize> = order
        .iter()
        .copied()
        .filter(|idx| admitted_indices.contains(idx))
        .collect();
    let admitted_lens_names: Vec<String> = admitted_order
        .iter()
        .map(|&idx| measurements[idx].name.clone())
        .collect();
    let panel = panel_mi(corpus, &admitted_order, &measurements, &anchor)?;

    // Per-stratum bits: stratify lens-0 by class label.
    let strata = stratify(corpus, &anchor)?;
    let strata_reports: Vec<StratumReport> = strata
        .strata
        .iter()
        .map(|stratum| StratumReport {
            name: stratum.name.clone(),
            bits: stratum.bits,
            frequency: stratum.frequency,
        })
        .collect();

    // Persist per-lens estimates to the Assay CF as the source-of-truth.
    let (persisted, readback) = persist_estimates(corpus, request, &measurements)?;

    Ok(AssayBitsReport {
        dataset: corpus.dataset.clone(),
        embedding_model_id: corpus.embedding_model_id.clone(),
        domain: request.domain.clone(),
        n_samples: corpus.n_samples(),
        target_class: request.target_class,
        anchor_leaks_into_input: anchor_audit.anchor_leaks_into_input,
        trivial_anchor: anchor_audit.trivial_anchor,
        grounded_gate_eligible: anchor_audit.grounded_gate_eligible,
        anchor_audit,
        anchor_entropy_bits,
        min_informative_target_entropy_bits: calyx_assay::MIN_INFORMATIVE_TARGET_ENTROPY_BITS,
        min_bits: request.min_bits,
        max_corr: request.max_corr,
        lenses,
        panel: PanelReport {
            admitted_lenses: admitted_lens_names,
            i_panel_anchor: panel.bits,
            ci_95: [panel.ci_low, panel.ci_high],
            estimate_bound: estimate_bound_name(panel.bound).to_string(),
            sufficiency_basis_bits: panel.ci_low,
            power_calibration_status: calibration_status(&panel),
            power_recovery_ratio: panel
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.recovery_ratio),
            power_recovered_bits: panel
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.recovered_bits),
            power_planted_bits: panel
                .power_calibration
                .as_ref()
                .map(|calibration| calibration.planted_bits),
        },
        strata: strata_reports,
        signal_density,
        packed_panel,
        panel_comparison,
        cf_root: request.cf_root.display().to_string(),
        assay_cf_rows_persisted: persisted,
        assay_cf_rows_readback: readback,
    })
}

fn panel_mi(
    corpus: &AssayCorpus,
    admitted_order: &[usize],
    measurements: &[LensMeasurement],
    anchor: &[bool],
) -> Result<MiEstimate, String> {
    if admitted_order.is_empty() {
        return Err("CALYX_FSV_ASSAY_EMPTY_PANEL: no admitted lenses".to_string());
    }
    let n = corpus.n_samples();
    let mut joint: Vec<Vec<f32>> = vec![Vec::new(); n];
    for &idx in admitted_order {
        let rows = &corpus.lens_vectors[measurements[idx].index];
        for (sample, row) in rows.iter().enumerate() {
            joint[sample].extend_from_slice(row);
        }
    }
    let report =
        logistic_probe_mi_multiseed_calibrated(&joint, anchor, Some(&corpus.anchor_groups))
            .map_err(calyx_error_detail)?;
    Ok(report.estimate)
}

fn max_corr_evidence(
    corpus: &AssayCorpus,
    measurements: &[LensMeasurement],
    admitted_indices: &[usize],
    measurement: &LensMeasurement,
) -> Result<CorrelationEvidence, String> {
    let mut max_corr = CorrelationEvidence::point(0.0);
    for &other in admitted_indices {
        let evidence = lens_pair_correlation_evidence(
            &corpus.lens_vectors[measurement.index],
            &corpus.lens_vectors[measurements[other].index],
        )?;
        if evidence.corr > max_corr.corr {
            max_corr = evidence;
        }
    }
    Ok(max_corr)
}

fn stratify(corpus: &AssayCorpus, anchor: &[bool]) -> Result<calyx_assay::StratifiedBits, String> {
    let global = logistic_probe_mi_multiseed_calibrated(
        &corpus.lens_vectors[0],
        anchor,
        Some(&corpus.anchor_groups),
    )
    .map_err(calyx_error_detail)?
    .estimate
    .bits;
    let mut classes: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for &label in &corpus.labels {
        classes.insert(label);
    }
    let total = corpus.n_samples() as f32;
    let mut strata = Vec::new();
    for class in classes {
        // One-vs-rest anchor restricted to this stratum membership.
        let member: Vec<bool> = corpus.labels.iter().map(|&l| l == class).collect();
        let frequency = member.iter().filter(|&&v| v).count() as f32 / total.max(1.0);
        // Stratum bits: lens-0 signal about "is this sample in this class".
        let bits = logistic_probe_mi_multiseed_calibrated(
            &corpus.lens_vectors[0],
            &member,
            Some(&corpus.anchor_groups),
        )
        .map(|report| report.estimate.bits)
        .map_err(|error| format!("{}: stratum=class_{class}", calyx_error_detail(error)))?;
        strata.push(StratumBits {
            name: format!("class_{class}"),
            bits,
            frequency,
            sole_carrier: false,
        });
    }
    Ok(stratified_bits(global, strata))
}

fn persist_estimates(
    corpus: &AssayCorpus,
    request: &AssayBitsRequest,
    measurements: &[LensMeasurement],
) -> Result<(usize, usize), String> {
    let vault_id = deterministic_vault_id(&request.domain);
    let mut store = AssayStore::default();
    for measurement in measurements {
        let key = AssayCacheKey::scoped(
            PANEL_VERSION,
            request.domain.clone(),
            vault_id,
            AnchorKind::Label(format!("target_class_{}", request.target_class)),
        );
        let slot = SlotId::new(u16::try_from(measurement.index).unwrap_or(u16::MAX));
        store.put(
            key,
            AssaySubject::Lens { slot },
            measurement.estimate.clone(),
            format!(
                "assay bits-validate {} lens={}",
                corpus.dataset, measurement.name
            ),
            measurement.index as u64,
        );
    }
    let mut router =
        CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let persisted = store
        .persist_to_aster(&mut router)
        .map_err(calyx_error_detail)?;
    drop(router);
    let reopened = CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let loaded = AssayStore::load_from_aster(&reopened).map_err(calyx_error_detail)?;
    Ok((persisted, loaded.len()))
}

fn deterministic_vault_id(domain: &str) -> VaultId {
    let digest = blake3::hash(domain.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    VaultId::from_ulid(Ulid::from_bytes(bytes))
}
