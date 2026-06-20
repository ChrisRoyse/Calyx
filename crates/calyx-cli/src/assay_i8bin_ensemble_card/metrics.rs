use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EnsembleCard, EstimatorKind, MiEstimate, TrustTag,
};
use calyx_aster::cf::CfRouter;
use calyx_core::{AnchorKind, VaultId};
use serde::Serialize;
use ulid::Ulid;

use crate::assay_bits_validation::calyx_error_detail;

use super::engine::I8binEnsembleReport;
use super::request::I8binEnsembleRequest;

const PANEL_VERSION: u32 = 803;
const CF_MEMTABLE_CAP: usize = 1_048_576;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct I8binEnsembleEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) a37_report_path: String,
    pub(crate) ensemble_card_path: String,
    pub(crate) lens_values_path: String,
    pub(crate) pair_values_path: String,
    pub(crate) matrix_path: String,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
    pub(crate) assay_cf_subject_counts: BTreeMap<String, usize>,
    pub(crate) ensemble_card_row_present: bool,
    pub(crate) ensemble_card_payload_readback: bool,
    pub(crate) report: I8binEnsembleReport,
}

pub(crate) fn write_outputs(
    request: &I8binEnsembleRequest,
    report: &I8binEnsembleReport,
) -> Result<I8binEnsembleEvidence, String> {
    check_finite(report)?;
    request.ensure_fresh_outputs()?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let persistence = persist_and_readback(request, &report.card)?;

    let a37_report_path = request.metrics_dir.join("a37_i8bin_ensemble_report.json");
    fs::write(
        &a37_report_path,
        serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let ensemble_card_path = request.metrics_dir.join("ensemble_card.json");
    fs::write(
        &ensemble_card_path,
        serde_json::to_vec_pretty(&report.card).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let lens_values_path = request.metrics_dir.join("ensemble_lens_values.txt");
    fs::write(&lens_values_path, lens_values(report)).map_err(|error| error.to_string())?;

    let pair_values_path = request.metrics_dir.join("ensemble_pair_values.txt");
    fs::write(&pair_values_path, pair_values(report)).map_err(|error| error.to_string())?;

    let matrix_path = request.metrics_dir.join("correlation_nmi_matrix.json");
    fs::write(
        &matrix_path,
        serde_json::to_vec_pretty(&report.matrix).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    Ok(I8binEnsembleEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        a37_report_path: display(&a37_report_path),
        ensemble_card_path: display(&ensemble_card_path),
        lens_values_path: display(&lens_values_path),
        pair_values_path: display(&pair_values_path),
        matrix_path: display(&matrix_path),
        cf_root: request.cf_root.display().to_string(),
        assay_cf_rows_persisted: persistence.persisted,
        assay_cf_rows_readback: persistence.readback,
        assay_cf_subject_counts: persistence.subject_counts,
        ensemble_card_row_present: persistence.card_row_present,
        ensemble_card_payload_readback: persistence.card_payload_readback,
        report: report.clone(),
    })
}

fn lens_values(report: &I8binEnsembleReport) -> String {
    let mut out = String::new();
    for lens in &report.card.lenses {
        out.push_str(&format!(
            "lens={} slot={} solo={:.6} marginal={:.6} pid_unique={:.6} pid_redundant={:.6} pid_synergy={:.6} corr={:.6} nmi={:.6} decision={:?}\n",
            lens.name,
            lens.slot,
            lens.solo_bits,
            lens.marginal_bits,
            lens.pid.unique_bits,
            lens.pid.redundant_bits,
            lens.pid.synergistic_bits,
            lens.max_pairwise_corr,
            lens.max_pairwise_nmi,
            lens.decision
        ));
    }
    out
}

fn pair_values(report: &I8binEnsembleReport) -> String {
    let mut out = String::new();
    for pair in &report.card.pairs {
        out.push_str(&format!(
            "pair={}+{} slots={}+{} corr={:.6} nmi={:.6} pair_bits={:.6} synergy_gain={:.6}\n",
            pair.a,
            pair.b,
            pair.slot_a,
            pair.slot_b,
            pair.corr,
            pair.nmi,
            pair.pair_bits,
            pair.synergy_gain_bits
        ));
    }
    out
}

struct PersistenceReadback {
    persisted: usize,
    readback: usize,
    subject_counts: BTreeMap<String, usize>,
    card_row_present: bool,
    card_payload_readback: bool,
}

fn persist_and_readback(
    request: &I8binEnsembleRequest,
    card: &EnsembleCard,
) -> Result<PersistenceReadback, String> {
    let key = cache_key(request);
    let mut store = AssayStore::default();
    for (idx, lens) in card.lenses.iter().enumerate() {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: lens.slot },
            estimate(
                lens.solo_bits,
                lens.solo_ci,
                card.n_samples,
                EstimatorKind::LogisticProbe,
            ),
            format!("assay i8bin-ensemble-card lens={}", lens.name),
            idx as u64,
        );
    }
    for (idx, pair) in card.pairs.iter().enumerate() {
        store.put(
            key.clone(),
            AssaySubject::Pair {
                a: pair.slot_a,
                b: pair.slot_b,
            },
            estimate(
                pair.synergy_gain_bits,
                pair.pair_ci,
                card.n_samples,
                EstimatorKind::PairGain,
            ),
            format!("assay i8bin-ensemble-card pair={}+{}", pair.a, pair.b),
            1_000 + idx as u64,
        );
    }
    store.put(
        key.clone(),
        AssaySubject::Panel,
        panel_estimate(card, EstimatorKind::LogisticProbe),
        "assay i8bin-ensemble-card panel",
        2_000,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        MiEstimate::point(
            card.anchor_entropy_bits,
            card.n_samples,
            EstimatorKind::OutcomeEntropy,
            TrustTag::Provisional,
        ),
        "assay i8bin-ensemble-card anchor entropy",
        2_001,
    );
    let payload = serde_json::to_value(card).map_err(|error| error.to_string())?;
    store.put_with_payload(
        key.clone(),
        AssaySubject::EnsembleCard,
        panel_estimate(card, EstimatorKind::PanelSufficiency),
        "assay i8bin-ensemble-card payload",
        2_002,
        payload,
    );

    let mut router =
        CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let persisted = store
        .persist_to_aster(&mut router)
        .map_err(calyx_error_detail)?;
    drop(router);
    let reopened = CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let loaded = AssayStore::load_from_aster(&reopened).map_err(calyx_error_detail)?;
    let row = loaded.get(&key, &AssaySubject::EnsembleCard);
    let card_row_present = row.is_some();
    let card_payload_readback = match row.and_then(|row| row.payload.clone()) {
        Some(payload) => {
            let readback: EnsembleCard = serde_json::from_value(payload)
                .map_err(|error| format!("CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH: {error}"))?;
            if readback != *card {
                return Err(
                    "CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH: EnsembleCard payload changed after Assay CF readback"
                        .to_string(),
                );
            }
            true
        }
        None => {
            return Err(
                "CALYX_FSV_ASSAY_CARD_READBACK_MISSING: EnsembleCard payload absent from Assay CF"
                    .to_string(),
            );
        }
    };
    let rows = loaded.rows();
    Ok(PersistenceReadback {
        persisted,
        readback: rows.len(),
        subject_counts: subject_counts(&rows),
        card_row_present,
        card_payload_readback,
    })
}

fn estimate(bits: f32, ci: [f32; 2], n: usize, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::new(bits, ci[0], ci[1], n, estimator, TrustTag::Provisional)
}

fn panel_estimate(card: &EnsembleCard, estimator: EstimatorKind) -> MiEstimate {
    let mut estimate = estimate(card.panel_bits, card.panel_ci, card.n_samples, estimator)
        .with_bound(card.sufficiency.estimate_bound);
    if let Some(calibration) = card.sufficiency.power_calibration.clone() {
        estimate = estimate.with_power_calibration(calibration);
    }
    estimate
}

fn subject_counts(rows: &[calyx_assay::AssayRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = match &row.subject {
            AssaySubject::Lens { .. } => "lens",
            AssaySubject::Pair { .. } => "pair",
            AssaySubject::Panel => "panel",
            AssaySubject::OutcomeEntropy => "outcome_entropy",
            AssaySubject::EnsembleCard => "ensemble_card",
        };
        *counts.entry(key.to_string()).or_insert(0) += 1;
    }
    counts
}

fn cache_key(request: &I8binEnsembleRequest) -> AssayCacheKey {
    AssayCacheKey::scoped(
        PANEL_VERSION,
        request.domain.clone(),
        deterministic_vault_id(&request.domain),
        AnchorKind::Label(format!("target_class_{}", request.target_class)),
    )
}

fn deterministic_vault_id(domain: &str) -> VaultId {
    let digest = blake3::hash(domain.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    VaultId::from_ulid(Ulid::from_bytes(bytes))
}

fn check_finite(report: &I8binEnsembleReport) -> Result<(), String> {
    let mut values = vec![
        ("anchor_entropy_bits", report.card.anchor_entropy_bits),
        ("panel_bits", report.card.panel_bits),
        ("n_eff", report.card.n_eff),
        ("matrix.n_eff", report.matrix.n_eff),
        (
            "matrix.mean_pairwise_corr",
            report.matrix.mean_pairwise_corr,
        ),
        ("matrix.mean_pairwise_nmi", report.matrix.mean_pairwise_nmi),
        (
            "diversity.sum_unique_pid_bits",
            report.diversity.sum_unique_pid_bits,
        ),
    ];
    for lens in &report.card.lenses {
        values.push(("lens.solo_bits", lens.solo_bits));
        values.push(("lens.marginal_bits", lens.marginal_bits));
        values.push(("lens.pid.unique_bits", lens.pid.unique_bits));
        values.push(("lens.pid.redundant_bits", lens.pid.redundant_bits));
        values.push(("lens.pid.synergistic_bits", lens.pid.synergistic_bits));
    }
    for pair in &report.matrix.pairs {
        values.push(("matrix.pair.corr", pair.corr));
        values.push(("matrix.pair.nmi", pair.nmi));
    }
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_ASSAY_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
