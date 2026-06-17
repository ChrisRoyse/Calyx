use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, MiEstimate, StratumBits, admit_lens, entropy_bits,
    logistic_probe_mi, stratified_bits,
};
use calyx_aster::cf::CfRouter;
use calyx_core::{AnchorKind, SlotId, VaultId};
use serde::Serialize;
use ulid::Ulid;

use super::cost::{LensCostMap, LensDensity};
use super::data::AssayCorpus;
use super::request::AssayBitsRequest;

const PANEL_VERSION: u32 = 70;
const CF_MEMTABLE_CAP: usize = 1_048_576;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AssayBitsReport {
    pub(crate) dataset: String,
    pub(crate) embedding_model_id: String,
    pub(crate) domain: String,
    pub(crate) n_samples: usize,
    pub(crate) target_class: usize,
    pub(crate) anchor_entropy_bits: f32,
    pub(crate) min_bits: f32,
    pub(crate) max_corr: f32,
    pub(crate) lenses: Vec<LensReport>,
    pub(crate) panel: PanelReport,
    pub(crate) strata: Vec<StratumReport>,
    /// Present only when `--cost-json` was supplied: per-lens signal density
    /// (bits per resource) ranked for panel selection (#717 signal-density).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signal_density: Option<SignalDensityReport>,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensReport {
    pub(crate) name: String,
    pub(crate) redundant: bool,
    pub(crate) bits_about: f32,
    pub(crate) ci: [f32; 2],
    pub(crate) estimator: String,
    pub(crate) max_pairwise_corr: f32,
    pub(crate) admitted: bool,
    pub(crate) rejection_reason: Option<String>,
    /// Per-lens signal density, present only when `--cost-json` was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) density: Option<LensDensity>,
}

/// Lenses ranked by signal density for panel selection. CPU-only (zero-VRAM)
/// lenses are ranked first — they cost nothing on the scarce GPU resource —
/// ordered among themselves by `bits_per_ms`; GPU lenses follow, ordered by
/// `bits_per_vram_mb` descending. This is the descriptive ranking the
/// resource-aware knapsack (#721/#729) consumes; it does not itself drop lenses.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct SignalDensityReport {
    pub(crate) note: String,
    pub(crate) ranked: Vec<DensityRank>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DensityRank {
    pub(crate) name: String,
    pub(crate) bits_about: f32,
    pub(crate) zero_vram: bool,
    pub(crate) vram_mb: f32,
    pub(crate) ms_per_input: f32,
    pub(crate) bits_per_vram_mb: Option<f32>,
    pub(crate) bits_per_ms: f32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PanelReport {
    pub(crate) admitted_lenses: Vec<String>,
    pub(crate) i_panel_anchor: f32,
    pub(crate) ci_95: [f32; 2],
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct StratumReport {
    pub(crate) name: String,
    pub(crate) bits: f32,
    pub(crate) frequency: f32,
}

struct LensMeasurement {
    index: usize,
    name: String,
    redundant: bool,
    estimate: MiEstimate,
}

pub(crate) fn evaluate_corpus(
    corpus: &AssayCorpus,
    request: &AssayBitsRequest,
    cost: Option<&LensCostMap>,
) -> Result<AssayBitsReport, String> {
    let anchor = corpus.anchor_labels(request.target_class);
    let positives = anchor.iter().filter(|&&v| v).count();
    if positives == 0 || positives == anchor.len() {
        return Err(format!(
            "CALYX_FSV_ASSAY_SINGLE_CLASS_ANCHOR: target_class={} positives={positives} total={}",
            request.target_class,
            anchor.len()
        ));
    }
    let anchor_entropy_bits = entropy_bits(&anchor);

    // Per-lens bits_about about the grounded binary anchor.
    let mut measurements = Vec::with_capacity(corpus.lenses.len());
    for (index, lens) in corpus.lenses.iter().enumerate() {
        let report = logistic_probe_mi(&corpus.lens_vectors[index], &anchor)
            .map_err(|error| error.code.to_string())?;
        measurements.push(LensMeasurement {
            index,
            name: lens.name.clone(),
            redundant: lens.redundant,
            estimate: report.estimate,
        });
    }

    // Greedy admission ordered by bits desc.
    let mut order: Vec<usize> = (0..measurements.len()).collect();
    order.sort_by(|&a, &b| {
        measurements[b]
            .estimate
            .bits
            .total_cmp(&measurements[a].estimate.bits)
    });

    let mut lens_reports: Vec<Option<LensReport>> = vec![None; measurements.len()];
    let mut admitted_indices: Vec<usize> = Vec::new();
    for &idx in &order {
        let measurement = &measurements[idx];
        let bits = measurement.estimate.bits;
        let max_corr = admitted_indices
            .iter()
            .map(|&other| {
                lens_pair_correlation(
                    &corpus.lens_vectors[measurement.index],
                    &corpus.lens_vectors[measurements[other].index],
                )
            })
            .fold(0.0_f32, f32::max);
        let decision = admit_lens(bits, max_corr);
        let (admitted, rejection_reason) = match decision {
            Ok(_) => {
                admitted_indices.push(idx);
                (true, None)
            }
            Err(error) => (false, Some(error.code.to_string())),
        };
        lens_reports[idx] = Some(LensReport {
            name: measurement.name.clone(),
            redundant: measurement.redundant,
            bits_about: bits,
            ci: [measurement.estimate.ci_low, measurement.estimate.ci_high],
            estimator: format!("{:?}", measurement.estimate.estimator),
            max_pairwise_corr: max_corr,
            admitted,
            rejection_reason,
            density: None,
        });
    }
    let mut lenses: Vec<LensReport> = lens_reports
        .into_iter()
        .map(|report| report.expect("every lens measured"))
        .collect();

    // Fail-closed checks.
    for (lens, measurement) in lenses.iter().zip(&measurements) {
        if !measurement.redundant && measurement.estimate.bits < request.min_bits {
            return Err(format!(
                "CALYX_FSV_ASSAY_BITS_BELOW_THRESHOLD: lens={} bits={:.6}",
                lens.name, measurement.estimate.bits
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
    let signal_density = match cost {
        Some(cost_map) => Some(compute_signal_density(&mut lenses, cost_map)?),
        None => None,
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
        anchor_entropy_bits,
        min_bits: request.min_bits,
        max_corr: request.max_corr,
        lenses,
        panel: PanelReport {
            admitted_lenses: admitted_lens_names,
            i_panel_anchor: panel.bits,
            ci_95: [panel.ci_low, panel.ci_high],
        },
        strata: strata_reports,
        signal_density,
        cf_root: request.cf_root.display().to_string(),
        assay_cf_rows_persisted: persisted,
        assay_cf_rows_readback: readback,
    })
}

/// Attach per-lens [`LensDensity`] to each lens report and build the ranked
/// [`SignalDensityReport`]. Fail-closed: every lens must have a measured cost.
fn compute_signal_density(
    lenses: &mut [LensReport],
    cost: &LensCostMap,
) -> Result<SignalDensityReport, String> {
    for lens in lenses.iter_mut() {
        let lens_cost = cost.require(&lens.name)?;
        lens.density = Some(LensDensity::compute(lens.bits_about, lens_cost));
    }
    // Rank for selection: zero-VRAM (CPU-only) lenses first — they are free on
    // the scarce GPU resource — ordered by bits/ms; then GPU lenses by
    // bits/VRAM-MB descending. Ties broken by name for determinism.
    let mut ranked: Vec<DensityRank> = lenses
        .iter()
        .map(|lens| {
            let d = lens.density.expect("density set above");
            DensityRank {
                name: lens.name.clone(),
                bits_about: lens.bits_about,
                zero_vram: d.zero_vram,
                vram_mb: d.vram_mb,
                ms_per_input: d.ms_per_input,
                bits_per_vram_mb: d.bits_per_vram_mb,
                bits_per_ms: d.bits_per_ms,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        // zero-VRAM lenses sort before VRAM-bearing ones.
        b.zero_vram
            .cmp(&a.zero_vram)
            .then_with(|| match (a.zero_vram, b.zero_vram) {
                (true, true) => b.bits_per_ms.total_cmp(&a.bits_per_ms),
                _ => {
                    let av = a.bits_per_vram_mb.unwrap_or(f32::INFINITY);
                    let bv = b.bits_per_vram_mb.unwrap_or(f32::INFINITY);
                    bv.total_cmp(&av)
                }
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(SignalDensityReport {
        note: "ranked by signal density: CPU-only (zero-VRAM) lenses first by \
               bits/ms, then GPU lenses by bits/VRAM-MB descending"
            .to_string(),
        ranked,
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
    let report = logistic_probe_mi(&joint, anchor).map_err(|error| error.code.to_string())?;
    Ok(report.estimate)
}

fn stratify(corpus: &AssayCorpus, anchor: &[bool]) -> Result<calyx_assay::StratifiedBits, String> {
    let global = logistic_probe_mi(&corpus.lens_vectors[0], anchor)
        .map_err(|error| error.code.to_string())?
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
        let bits = logistic_probe_mi(&corpus.lens_vectors[0], &member)
            .map(|report| report.estimate.bits)
            .unwrap_or(0.0);
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
    let mut router = CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP)
        .map_err(|error| error.code.to_string())?;
    let persisted = store
        .persist_to_aster(&mut router)
        .map_err(|error| error.code.to_string())?;
    drop(router);
    let reopened = CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP)
        .map_err(|error| error.code.to_string())?;
    let loaded = AssayStore::load_from_aster(&reopened).map_err(|error| error.code.to_string())?;
    Ok((persisted, loaded.len()))
}

fn deterministic_vault_id(domain: &str) -> VaultId {
    let digest = blake3::hash(domain.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    VaultId::from_ulid(Ulid::from_bytes(bytes))
}

/// Representational correlation between two lenses:
/// `mean_i cosine(unit(A_i), unit(B_i))`.
fn lens_pair_correlation(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    // Representational correlation is only defined between same-shaped lenses;
    // differently dimensioned lenses cannot be representational near-duplicates.
    if a.first().map(Vec::len) != b.first().map(Vec::len) {
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
