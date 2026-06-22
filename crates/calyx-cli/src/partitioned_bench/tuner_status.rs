use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use calyx_sextant::index::{
    BwPostcutoffConfig, BwPostcutoffTuner, TunerConfig, TunerObservation, TunerRange,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};

const STATUS_REL: &str = "bench/bw_postcutoff_status.json";
const DEFAULT_LATENCY_SLO_US: u64 = 25_000;
const DEFAULT_RECALL_FLOOR: f32 = 0.85;
const POSTING_CUTOFF_MIN: usize = 1;
const POSTING_CUTOFF_MAX: usize = 65_536;
const POSTING_CUTOFF_STEP: usize = 1;

pub(super) struct Request<'a> {
    pub vault: &'a Path,
    pub latencies_us: &'a [u64],
    pub per_query_recall: &'a [f32],
    pub region_beam: usize,
    pub n_probe: usize,
    pub tuner_slo_us: Option<u64>,
    pub recall_floor: Option<f32>,
    pub aggregate_recall: f32,
    pub latency_us: &'a serde_json::Value,
    pub queries: usize,
    pub k: usize,
}

pub(super) fn write(req: Request<'_>) -> CliResult<PathBuf> {
    if req.per_query_recall.is_empty() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_RECALL_REQUIRED",
            "--anneal-vault requires per-query recall observations",
            "rerun with synthetic self-recall or use partitioned-rrf with ground truth",
        ));
    }
    if req.latencies_us.len() < req.per_query_recall.len() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_OBSERVATION_MISMATCH",
            format!(
                "latencies={} recall_rows={}",
                req.latencies_us.len(),
                req.per_query_recall.len()
            ),
            "record one latency and recall value per tuner observation",
        ));
    }
    let mut tuner = BwPostcutoffTuner::with_config(
        BwPostcutoffConfig {
            beamwidth: req.region_beam,
            posting_cutoff: req.n_probe,
        },
        TunerConfig {
            posting_cutoff: TunerRange::new(
                POSTING_CUTOFF_MIN,
                POSTING_CUTOFF_MAX.max(req.n_probe),
                POSTING_CUTOFF_STEP,
            ),
            latency_slo_us: req.tuner_slo_us.unwrap_or(DEFAULT_LATENCY_SLO_US),
            recall_floor: req.recall_floor.unwrap_or(DEFAULT_RECALL_FLOOR),
            ..TunerConfig::default()
        },
    );
    for (&latency, &recall) in req.latencies_us.iter().zip(req.per_query_recall) {
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
        mode: "synthetic_partitioned_search",
        trigger: "calyx bench partitioned-search",
        current: tuner.current_config(),
        adjustments: tuner.adjustment_history(),
        ledger_entries: tuner.ledger_entries(),
        warnings: tuner.warnings(),
        observations: req.per_query_recall.len(),
        recall_observation_mode: "synthetic_query_self_hit_per_query",
        posting_cutoff_semantic: "partitioned_search_n_probe",
        aggregate_recall_at_k: req.aggregate_recall,
        latency_us: req.latency_us,
        queries: req.queries,
        k: req.k,
    };
    let path = req.vault.join(STATUS_REL);
    write_bytes_atomic(&path, &serde_json::to_vec_pretty(&status)?)?;
    Ok(path)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        return Err(tuner_error(
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_STATUS_EXISTS",
            format!("{} already exists", path.display()),
            "use a fresh --anneal-vault for each FSV trigger",
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
    recall_observation_mode: &'static str,
    posting_cutoff_semantic: &'static str,
    aggregate_recall_at_k: f32,
    latency_us: &'a serde_json::Value,
    queries: usize,
    k: usize,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn writes_synthetic_partitioned_search_status_readback() {
        let root = temp_root("search-tuner-status");
        let latencies = vec![100_u64; 512];
        let recalls = vec![1.0_f32; 512];

        let path = write(Request {
            vault: &root,
            latencies_us: &latencies,
            per_query_recall: &recalls,
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            aggregate_recall: 1.0,
            latency_us: &json!({"p99": 100}),
            queries: 512,
            k: 10,
        })
        .unwrap();

        let row: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(row["tuner"], "bw_postcutoff");
        assert_eq!(row["mode"], "synthetic_partitioned_search");
        assert_eq!(row["trigger"], "calyx bench partitioned-search");
        assert_eq!(row["current"]["posting_cutoff"], 8);
        assert_eq!(row["ledger_entries"][0]["event"], "diskann_tuner_adjust");
        assert_eq!(
            row["recall_observation_mode"],
            "synthetic_query_self_hit_per_query"
        );
        assert_eq!(row["posting_cutoff_semantic"], "partitioned_search_n_probe");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_stale_synthetic_status_readback() {
        let root = temp_root("search-tuner-stale");
        let first = write(Request {
            vault: &root,
            latencies_us: &[100],
            per_query_recall: &[1.0],
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            aggregate_recall: 1.0,
            latency_us: &json!({"p99": 100}),
            queries: 1,
            k: 10,
        })
        .unwrap();
        assert!(first.is_file());

        let err = write(Request {
            vault: &root,
            latencies_us: &[100],
            per_query_recall: &[1.0],
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            aggregate_recall: 1.0,
            latency_us: &json!({"p99": 100}),
            queries: 1,
            k: 10,
        })
        .unwrap_err();

        assert_eq!(
            err.code(),
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_STATUS_EXISTS"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_mismatched_synthetic_tuner_observations() {
        let root = temp_root("search-tuner-mismatch");
        let err = write(Request {
            vault: &root,
            latencies_us: &[100],
            per_query_recall: &[1.0, 1.0],
            region_beam: 64,
            n_probe: 8,
            tuner_slo_us: Some(1),
            recall_floor: Some(0.85),
            aggregate_recall: 1.0,
            latency_us: &json!({"p99": 100}),
            queries: 2,
            k: 10,
        })
        .unwrap_err();

        assert_eq!(
            err.code(),
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_OBSERVATION_MISMATCH"
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
