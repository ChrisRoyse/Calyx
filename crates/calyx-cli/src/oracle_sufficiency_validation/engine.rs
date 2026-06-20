use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag,
    ensure_informative_binary_labels, logistic_probe_mi_calibrated,
};
use calyx_aster::cf::CfRouter;
use calyx_core::{AnchorKind, CalyxError, SlotId, VaultId};
use serde::Serialize;
use ulid::Ulid;

use crate::assay_verdict_metadata::{
    calibration_planted_bits, calibration_recovered_bits, calibration_recovery_ratio,
    calibration_status, estimate_bound_name,
};

use super::data::OracleCorpus;
use super::request::OracleSufficiencyRequest;

const PANEL_VERSION: u32 = 70;
const CF_MEMTABLE_CAP: usize = 1_048_576;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OracleSufficiencyReport {
    pub(crate) oracle_model: String,
    pub(crate) dataset: String,
    pub(crate) anchor: String,
    pub(crate) embedding_model_id: String,
    pub(crate) domain: String,
    pub(crate) n: usize,
    pub(crate) resolved: usize,
    pub(crate) h_y: f32,
    pub(crate) i_panel_oracle: f32,
    pub(crate) i_panel_ci: [f32; 2],
    pub(crate) estimate_bound: String,
    pub(crate) sufficiency_basis_bits: f32,
    pub(crate) power_calibration_status: Option<String>,
    pub(crate) power_recovery_ratio: Option<f32>,
    pub(crate) power_recovered_bits: Option<f32>,
    pub(crate) power_planted_bits: Option<f32>,
    pub(crate) deficit: f32,
    pub(crate) sufficient: bool,
    pub(crate) refused: bool,
    pub(crate) lenses: Vec<LensReport>,
    pub(crate) per_sensor_deficit: Vec<PerSensorDeficit>,
    pub(crate) cf_root: String,
    pub(crate) rows_persisted: usize,
    pub(crate) rows_readback: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensReport {
    pub(crate) name: String,
    pub(crate) bits: f32,
    pub(crate) ci: [f32; 2],
    pub(crate) estimate_bound: String,
    pub(crate) power_calibration_status: Option<String>,
    pub(crate) power_recovery_ratio: Option<f32>,
    pub(crate) power_recovered_bits: Option<f32>,
    pub(crate) power_planted_bits: Option<f32>,
    pub(crate) accuracy: f32,
    pub(crate) estimator: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PerSensorDeficit {
    pub(crate) name: String,
    pub(crate) deficit: f32,
}

struct LensMeasurement {
    index: usize,
    name: String,
    estimate: MiEstimate,
    accuracy: f32,
}

pub(crate) fn evaluate_corpus(
    corpus: &OracleCorpus,
    request: &OracleSufficiencyRequest,
) -> Result<OracleSufficiencyReport, String> {
    // Lens-agnostic binary oracle labels: true iff the instance was resolved.
    let labels: &[bool] = &corpus.labels;
    let h_y = ensure_informative_binary_labels(labels).map_err(calyx_error_detail)?;

    // Per-lens form-only bits about the oracle.
    let mut measurements = Vec::with_capacity(corpus.lenses.len());
    for (index, lens) in corpus.lenses.iter().enumerate() {
        let report = logistic_probe_mi_calibrated(&corpus.lens_vectors[index], labels)
            .map_err(calyx_error_detail)?;
        measurements.push(LensMeasurement {
            index,
            name: lens.name.clone(),
            estimate: report.estimate,
            accuracy: report.accuracy,
        });
    }

    // Panel I(panel;oracle): concatenate ALL lens vectors per instance.
    let panel = panel_mi(corpus, labels)?;
    let i_panel_oracle = panel.bits;
    let sufficiency_basis_bits = panel.ci_low;

    let sufficient = sufficiency_basis_bits >= h_y;
    let refused = !sufficient;
    let deficit = (h_y - sufficiency_basis_bits).max(0.0);

    // Fail-closed: the binding outcome is that the form-only panel is
    // insufficient and refusal fires. A sufficient form-only panel (or a gate
    // that fails to refuse) is a regression, not a pass.
    if sufficient {
        return Err(format!(
            "CALYX_FSV_ORACLE_PANEL_UNEXPECTEDLY_SUFFICIENT: i_panel_oracle={i_panel_oracle:.6} h_y={h_y:.6}"
        ));
    }
    if !refused {
        return Err(format!(
            "CALYX_FSV_ORACLE_REFUSAL_DID_NOT_FIRE: i_panel_oracle={i_panel_oracle:.6} h_y={h_y:.6}"
        ));
    }

    let lenses: Vec<LensReport> = measurements
        .iter()
        .map(|m| LensReport {
            name: m.name.clone(),
            bits: m.estimate.bits,
            ci: [m.estimate.ci_low, m.estimate.ci_high],
            estimate_bound: estimate_bound_name(m.estimate.bound).to_string(),
            power_calibration_status: calibration_status(&m.estimate),
            power_recovery_ratio: calibration_recovery_ratio(&m.estimate),
            power_recovered_bits: calibration_recovered_bits(&m.estimate),
            power_planted_bits: calibration_planted_bits(&m.estimate),
            accuracy: m.accuracy,
            estimator: format!("{:?}", m.estimate.estimator),
        })
        .collect();

    let per_sensor_deficit: Vec<PerSensorDeficit> = measurements
        .iter()
        .map(|m| PerSensorDeficit {
            name: m.name.clone(),
            deficit: (h_y - m.estimate.bits).max(0.0),
        })
        .collect();

    // Persist oracle-sufficiency rows to the Assay CF as the source-of-truth,
    // then reopen and load to prove durable readback.
    let (persisted, readback) = persist_estimates(corpus, request, &measurements, &panel, h_y)?;

    Ok(OracleSufficiencyReport {
        oracle_model: corpus.oracle_model.clone(),
        dataset: corpus.dataset.clone(),
        anchor: corpus.anchor.clone(),
        embedding_model_id: corpus.embedding_model_id.clone(),
        domain: request.domain.clone(),
        n: corpus.n_samples(),
        resolved: corpus.resolved(),
        h_y,
        i_panel_oracle,
        i_panel_ci: [panel.ci_low, panel.ci_high],
        estimate_bound: estimate_bound_name(panel.bound).to_string(),
        sufficiency_basis_bits,
        power_calibration_status: calibration_status(&panel),
        power_recovery_ratio: calibration_recovery_ratio(&panel),
        power_recovered_bits: calibration_recovered_bits(&panel),
        power_planted_bits: calibration_planted_bits(&panel),
        deficit,
        sufficient,
        refused,
        lenses,
        per_sensor_deficit,
        cf_root: request.cf_root.display().to_string(),
        rows_persisted: persisted,
        rows_readback: readback,
    })
}

fn panel_mi(corpus: &OracleCorpus, labels: &[bool]) -> Result<MiEstimate, String> {
    if corpus.lenses.is_empty() {
        return Err("CALYX_FSV_ORACLE_INVALID_CORPUS: empty panel".to_string());
    }
    let n = corpus.n_samples();
    let mut joint: Vec<Vec<f32>> = vec![Vec::new(); n];
    for lens_rows in &corpus.lens_vectors {
        for (sample, row) in lens_rows.iter().enumerate() {
            joint[sample].extend_from_slice(row);
        }
    }
    let report = logistic_probe_mi_calibrated(&joint, labels).map_err(calyx_error_detail)?;
    Ok(report.estimate)
}

fn persist_estimates(
    corpus: &OracleCorpus,
    request: &OracleSufficiencyRequest,
    measurements: &[LensMeasurement],
    panel: &MiEstimate,
    h_y: f32,
) -> Result<(usize, usize), String> {
    let vault_id = deterministic_vault_id(&request.domain);
    let key = AssayCacheKey::scoped(
        PANEL_VERSION,
        request.domain.clone(),
        vault_id,
        AnchorKind::Label(corpus.anchor.clone()),
    );
    let mut store = AssayStore::default();
    for measurement in measurements {
        let slot = SlotId::new(u16::try_from(measurement.index).unwrap_or(u16::MAX));
        store.put(
            key.clone(),
            AssaySubject::Lens { slot },
            measurement.estimate.clone(),
            format!(
                "oracle sufficiency-validate {} lens={}",
                corpus.dataset, measurement.name
            ),
            measurement.index as u64,
        );
    }
    store.put(
        key.clone(),
        AssaySubject::Panel,
        panel.clone(),
        format!("oracle sufficiency-validate {} panel", corpus.dataset),
        measurements.len() as u64,
    );
    store.put(
        key,
        AssaySubject::OutcomeEntropy,
        MiEstimate::point(
            h_y,
            corpus.n_samples(),
            EstimatorKind::OutcomeEntropy,
            TrustTag::Trusted,
        ),
        format!(
            "oracle sufficiency-validate {} outcome entropy",
            corpus.dataset
        ),
        (measurements.len() + 1) as u64,
    );

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

fn calyx_error_detail(error: CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}
