use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use calyx_sextant::index::{BwPostcutoffConfig, BwPostcutoffTuner, TunerConfig, TunerObservation};
use serde::Serialize;
use serde_json::Value;

use crate::error::{CliError, CliResult};

const STATUS_REL: &str = "bench/bw_postcutoff_status.json";
const DEFAULT_LATENCY_SLO_US: u64 = 25_000;
const DEFAULT_RECALL_FLOOR: f32 = 0.85;

pub(super) struct StatusRequest<'a> {
    pub vault: &'a Path,
    pub latencies_us: &'a [u64],
    pub per_query_recall: &'a [f32],
    pub region_beam: usize,
    pub n_probe: usize,
    pub tuner_slo_us: Option<u64>,
    pub recall_floor: Option<f32>,
    pub report_path: Option<&'a Path>,
    pub report_latency_us: &'a Value,
    pub fused_recall: Option<f32>,
    pub lens_count: usize,
    pub queries: usize,
    pub k: usize,
}

pub(super) fn write_status(req: StatusRequest<'_>) -> CliResult<PathBuf> {
    if req.per_query_recall.is_empty() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_RRF_TUNER_GROUND_TRUTH_REQUIRED",
            "--anneal-vault requires --ground-truth > 0 so tuner observations carry recall",
            "rerun fused RRF with --ground-truth N and --recall-floor before claiming #550",
        ));
    }
    if req.latencies_us.len() < req.per_query_recall.len() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_RRF_TUNER_OBSERVATION_MISMATCH",
            format!(
                "latencies={} recall_rows={}",
                req.latencies_us.len(),
                req.per_query_recall.len()
            ),
            "record one latency and one recall value for every tuner observation",
        ));
    }

    let mut tuner = BwPostcutoffTuner::with_config(
        BwPostcutoffConfig {
            beamwidth: req.region_beam,
            posting_cutoff: req.n_probe,
        },
        TunerConfig {
            latency_slo_us: req.tuner_slo_us.unwrap_or(DEFAULT_LATENCY_SLO_US),
            recall_floor: req.recall_floor.unwrap_or(DEFAULT_RECALL_FLOOR),
            ..TunerConfig::default()
        },
    );
    for (&latency, &recall) in req.latencies_us.iter().zip(req.per_query_recall.iter()) {
        tuner.observe(TunerObservation {
            query_latency_us: latency.max(1),
            recall_at_10: recall,
            beamwidth: req.region_beam,
            posting_cutoff: req.n_probe,
        });
        let _ = tuner.maybe_adjust();
    }

    let status = Status {
        tuner: "bw_postcutoff",
        mode: "real_multi_slot_rrf",
        trigger: "calyx bench partitioned-rrf",
        current: tuner.current_config(),
        adjustments: tuner.adjustment_history(),
        ledger_entries: tuner.ledger_entries(),
        warnings: tuner.warnings(),
        observations: req.per_query_recall.len(),
        lens_count: req.lens_count,
        queries: req.queries,
        k: req.k,
        fused_ground_truth_recall_at_k: req.fused_recall,
        latency_us: req.report_latency_us,
        report_path: req
            .report_path
            .map(|path| path.to_string_lossy().into_owned()),
    };
    let path = req.vault.join(STATUS_REL);
    write_bytes_atomic(&path, &serde_json::to_vec_pretty(&status)?)?;
    Ok(path)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_RRF_TUNER_STATUS_EXISTS",
            format!("{} already exists", path.display()),
            "use a fresh --anneal-vault for each FSV trigger to avoid stale status readback",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn tuner_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

#[derive(Serialize)]
struct Status<'a> {
    tuner: &'static str,
    mode: &'static str,
    trigger: &'static str,
    current: BwPostcutoffConfig,
    adjustments: &'a [calyx_sextant::TunerAdjustment],
    ledger_entries: &'a [calyx_sextant::TunerLedgerEntry],
    warnings: &'a [calyx_sextant::TunerWarning],
    observations: usize,
    lens_count: usize,
    queries: usize,
    k: usize,
    fused_ground_truth_recall_at_k: Option<f32>,
    latency_us: &'a Value,
    report_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn writes_adjustment_status_for_real_rrf_observations() {
        let root = temp_root("rrf-tuner-status");
        let latencies = vec![100_u64; 512];
        let recalls = vec![1.0_f32; 512];

        let path = write_status(StatusRequest {
            vault: &root,
            latencies_us: &latencies,
            per_query_recall: &recalls,
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            report_path: None,
            report_latency_us: &json!({"p99": 100}),
            fused_recall: Some(1.0),
            lens_count: 4,
            queries: 512,
            k: 10,
        })
        .unwrap();

        let row: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(row["tuner"], "bw_postcutoff");
        assert_eq!(row["mode"], "real_multi_slot_rrf");
        assert_eq!(row["ledger_entries"][0]["event"], "diskann_tuner_adjust");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_status_without_ground_truth_observations() {
        let root = temp_root("rrf-tuner-no-gt");
        let err = write_status(StatusRequest {
            vault: &root,
            latencies_us: &[],
            per_query_recall: &[],
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: None,
            recall_floor: Some(0.85),
            report_path: None,
            report_latency_us: &json!({}),
            fused_recall: None,
            lens_count: 4,
            queries: 0,
            k: 10,
        })
        .unwrap_err();

        assert_eq!(
            err.code(),
            "CALYX_FSV_PARTITIONED_RRF_TUNER_GROUND_TRUTH_REQUIRED"
        );
        assert!(!root.join(STATUS_REL).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "calyx-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
