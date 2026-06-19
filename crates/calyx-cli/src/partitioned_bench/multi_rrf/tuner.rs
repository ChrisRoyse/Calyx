use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use calyx_sextant::index::{
    BwPostcutoffConfig, BwPostcutoffTuner, TunerConfig, TunerObservation, TunerRange,
};
use serde::Serialize;
use serde_json::Value;

use crate::error::{CliError, CliResult};

const STATUS_REL: &str = "bench/bw_postcutoff_status.json";
const DEFAULT_LATENCY_SLO_US: u64 = 25_000;
const DEFAULT_RECALL_FLOOR: f32 = 0.85;
const RRF_POSTING_CUTOFF_MIN: usize = 1;
const RRF_POSTING_CUTOFF_MAX: usize = 65_536;
const RRF_POSTING_CUTOFF_STEP: usize = 1;
const RECALL_OBSERVATION_MODE: &str = "fused_aggregate_recall_repeated";
const POSTING_CUTOFF_SEMANTIC: &str = "partitioned_rrf_n_probe";

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
    let fused_recall = req.fused_recall.ok_or_else(|| {
        tuner_error(
            "CALYX_FSV_PARTITIONED_RRF_TUNER_AGGREGATE_RECALL_REQUIRED",
            "--anneal-vault requires aggregate fused recall readback",
            "rerun fused RRF with --ground-truth N so aggregate recall can be read",
        )
    })?;
    let recall_summary = RecallObservationSummary::from_rows(req.per_query_recall, fused_recall);

    let mut tuner = BwPostcutoffTuner::with_config(
        BwPostcutoffConfig {
            beamwidth: req.region_beam,
            posting_cutoff: req.n_probe,
        },
        TunerConfig {
            posting_cutoff: TunerRange::new(
                RRF_POSTING_CUTOFF_MIN,
                RRF_POSTING_CUTOFF_MAX.max(req.n_probe),
                RRF_POSTING_CUTOFF_STEP,
            ),
            latency_slo_us: req.tuner_slo_us.unwrap_or(DEFAULT_LATENCY_SLO_US),
            recall_floor: req.recall_floor.unwrap_or(DEFAULT_RECALL_FLOOR),
            ..TunerConfig::default()
        },
    );
    for &latency in req.latencies_us.iter().take(req.per_query_recall.len()) {
        tuner.observe(TunerObservation {
            query_latency_us: latency.max(1),
            recall_at_10: fused_recall,
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
        recall_observation_mode: RECALL_OBSERVATION_MODE,
        posting_cutoff_semantic: POSTING_CUTOFF_SEMANTIC,
        per_query_recall_summary: recall_summary,
        lens_count: req.lens_count,
        queries: req.queries,
        k: req.k,
        fused_ground_truth_recall_at_k: Some(fused_recall),
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
struct RecallObservationSummary {
    count: usize,
    min: f32,
    mean: f32,
    max: f32,
    aggregate_used_for_tuner: f32,
}

impl RecallObservationSummary {
    fn from_rows(rows: &[f32], aggregate: f32) -> Self {
        let min = rows.iter().copied().fold(f32::INFINITY, f32::min);
        let max = rows.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mean = rows.iter().sum::<f32>() / rows.len() as f32;
        Self {
            count: rows.len(),
            min,
            mean,
            max,
            aggregate_used_for_tuner: aggregate,
        }
    }
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
    recall_observation_mode: &'static str,
    posting_cutoff_semantic: &'static str,
    per_query_recall_summary: RecallObservationSummary,
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
        assert_eq!(row["current"]["posting_cutoff"], 8);
        assert_eq!(row["posting_cutoff_semantic"], "partitioned_rrf_n_probe");
        assert_eq!(
            row["recall_observation_mode"],
            "fused_aggregate_recall_repeated"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aggregate_recall_drives_tuner_while_raw_query_min_is_preserved() {
        let root = temp_root("rrf-tuner-aggregate-recall");
        let latencies = vec![100_u64; 512];
        let mut recalls = vec![1.0_f32; 512];
        recalls[0] = 0.0;

        let path = write_status(StatusRequest {
            vault: &root,
            latencies_us: &latencies,
            per_query_recall: &recalls,
            region_beam: 64,
            n_probe: 12,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            report_path: None,
            report_latency_us: &json!({"p99": 100}),
            fused_recall: Some(0.95),
            lens_count: 4,
            queries: 512,
            k: 10,
        })
        .unwrap();

        let row: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(row["current"]["posting_cutoff"], 12);
        assert_eq!(row["per_query_recall_summary"]["min"], 0.0);
        assert_eq!(
            row["per_query_recall_summary"]["aggregate_used_for_tuner"],
            0.95
        );
        assert_eq!(row["ledger_entries"][0]["event"], "diskann_tuner_adjust");
        assert_eq!(row["ledger_entries"][0]["reason"], "latency_above_slo");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn aggregate_recall_below_floor_reverts() {
        let root = temp_root("rrf-tuner-aggregate-revert");
        let latencies = vec![100_u64; 512];
        let recalls = vec![0.95_f32; 512];

        let path = write_status(StatusRequest {
            vault: &root,
            latencies_us: &latencies,
            per_query_recall: &recalls,
            region_beam: 64,
            n_probe: 12,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            report_path: None,
            report_latency_us: &json!({"p99": 100}),
            fused_recall: Some(0.80),
            lens_count: 4,
            queries: 512,
            k: 10,
        })
        .unwrap();

        let row: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(row["current"]["posting_cutoff"], 12);
        assert_eq!(row["ledger_entries"][0]["event"], "diskann_tuner_revert");
        assert_eq!(row["ledger_entries"][0]["reason"], "recall_below_floor");
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
