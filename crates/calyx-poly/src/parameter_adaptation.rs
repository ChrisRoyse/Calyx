//! Scheduled parameter auto-adaptation from local resolved observations (issue #105).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::exact_knn::ExactKnnExecution;
use crate::parameter_adaptation_math::{
    adaptation_metrics, knn_brier_table, proposed_parameters, validate_edges,
};
pub use crate::parameter_adaptation_types::*;

pub fn run_parameter_adaptation_report(
    request: &ParameterAdaptationRequest,
    output_root: &Path,
) -> Result<ParameterAdaptationRun> {
    run_parameter_adaptation_report_with_execution(request, output_root).map(|result| result.0)
}

pub fn run_parameter_adaptation_report_with_execution(
    request: &ParameterAdaptationRequest,
    output_root: &Path,
) -> Result<(ParameterAdaptationRun, ExactKnnExecution)> {
    let ledger_path = Path::new(&request.ledger_dir).join(PARAMETER_ADAPTATION_LEDGER_FILE);
    let before = read_parameter_adaptation_ledger_entries(&ledger_path)?;
    let (report, execution) =
        compute_parameter_adaptation_report_with_execution(request, before.len() as u64)?;
    let report_path = write_parameter_adaptation_report(output_root, &report)?;
    if let Some(entry) = &report.ledger_entry {
        append_ledger_entry(&ledger_path, entry)?;
    }
    let readback = read_parameter_adaptation_report(&report_path)?;
    let ledger_entries = read_parameter_adaptation_ledger_entries(&ledger_path)?;
    if !reports_equivalent(&readback, &report) {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_READBACK_MISMATCH,
            format!(
                "parameter adaptation report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    if let Some(entry) = &report.ledger_entry
        && (ledger_entries.last() != Some(entry) || ledger_entries.len() != before.len() + 1)
    {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_READBACK_MISMATCH,
            format!(
                "parameter adaptation ledger {} did not read back appended entry",
                ledger_path.display()
            ),
        ));
    }
    Ok((
        ParameterAdaptationRun {
            report_path,
            ledger_path,
            report: readback,
            ledger_entries,
        },
        execution,
    ))
}

pub fn compute_parameter_adaptation_report(
    request: &ParameterAdaptationRequest,
    next_ledger_sequence: u64,
) -> Result<ParameterAdaptationReport> {
    compute_parameter_adaptation_report_with_execution(request, next_ledger_sequence)
        .map(|result| result.0)
}

pub fn compute_parameter_adaptation_report_with_execution(
    request: &ParameterAdaptationRequest,
    next_ledger_sequence: u64,
) -> Result<(ParameterAdaptationReport, ExactKnnExecution)> {
    validate_request(request)?;
    let required_k = request
        .schedule
        .candidate_knn_k
        .iter()
        .copied()
        .chain(std::iter::once(request.current.knn_k))
        .collect::<Vec<_>>();
    let knn_briers = knn_brier_table(&request.observations, &required_k)?;
    let proposed = proposed_parameters(request, &knn_briers)?;
    let metrics = adaptation_metrics(request, &proposed, &knn_briers)?;
    let changed_parameters = changed_parameters(&request.current, &proposed);
    let promote = !changed_parameters.is_empty()
        && metrics.brier_improvement >= request.schedule.min_brier_improvement;
    let status = if promote {
        ParameterAdaptationStatus::Promoted
    } else {
        ParameterAdaptationStatus::NoChange
    };
    let report_hash = report_hash(request, &proposed, &metrics, &changed_parameters, status);
    let ledger_entry = promote.then(|| ParameterAdaptationLedgerEntry {
        schema_version: PARAMETER_ADAPTATION_SCHEMA_VERSION.to_string(),
        sequence: next_ledger_sequence,
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        previous_version: request.current.version.clone(),
        new_version: proposed.version.clone(),
        changed_parameters: changed_parameters.clone(),
        observations_hash: request.observations_artifact.blake3.clone(),
        rollback_hash: request.rollback_artifact.blake3.clone(),
        report_hash: report_hash.clone(),
        scheduled_at_ts: request.schedule.scheduled_at_ts,
    });
    let report = ParameterAdaptationReport {
        schema_version: PARAMETER_ADAPTATION_SCHEMA_VERSION.to_string(),
        artifact_kind: PARAMETER_ADAPTATION_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        status,
        reason: if promote {
            "scheduled_parameter_update_promoted".to_string()
        } else {
            "no_measured_improvement".to_string()
        },
        observation_count: request.observations.len(),
        new_observation_count: new_observation_count(request),
        previous: request.current.clone(),
        proposed,
        metrics,
        changed_parameters,
        observations_artifact: request.observations_artifact.clone(),
        rollback_artifact: request.rollback_artifact.clone(),
        ledger_entry,
        report_hash,
    };
    Ok((report, knn_briers.execution))
}

pub fn write_parameter_adaptation_report(
    dir: &Path,
    report: &ParameterAdaptationReport,
) -> Result<PathBuf> {
    write_json(dir, PARAMETER_ADAPTATION_REPORT_FILE, report)
}

pub fn read_parameter_adaptation_report(path: &Path) -> Result<ParameterAdaptationReport> {
    read_json(path)
}

pub fn read_parameter_adaptation_ledger_entries(
    path: &Path,
) -> Result<Vec<ParameterAdaptationLedgerEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_LEDGER_IO,
            format!("read parameter adaptation ledger {}: {err}", path.display()),
        )
    })?;
    let mut entries = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(PolyError::diagnostics(
                ERR_PARAMETER_ADAPTATION_LEDGER_DECODE,
                format!("ledger {} line {} is empty", path.display(), idx + 1),
            ));
        }
        entries.push(serde_json::from_str(line).map_err(|err| {
            PolyError::diagnostics(
                ERR_PARAMETER_ADAPTATION_LEDGER_DECODE,
                format!("decode ledger {} line {}: {err}", path.display(), idx + 1),
            )
        })?);
    }
    Ok(entries)
}

fn append_ledger_entry(path: &Path, entry: &ParameterAdaptationLedgerEntry) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::diagnostics(
                ERR_PARAMETER_ADAPTATION_LEDGER_IO,
                format!("create ledger dir {}: {err}", parent.display()),
            )
        })?;
    }
    let line = serde_json::to_string(entry).map_err(|err| {
        PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_LEDGER_DECODE,
            format!("encode parameter adaptation ledger entry: {err}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| {
            PolyError::diagnostics(
                ERR_PARAMETER_ADAPTATION_LEDGER_IO,
                format!("open parameter adaptation ledger {}: {err}", path.display()),
            )
        })?;
    writeln!(file, "{line}").map_err(|err| {
        PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_LEDGER_IO,
            format!(
                "append parameter adaptation ledger {}: {err}",
                path.display()
            ),
        )
    })
}

fn validate_request(request: &ParameterAdaptationRequest) -> Result<()> {
    for (field, value) in [
        ("domain", &request.domain),
        ("horizon_bucket", &request.horizon_bucket),
        ("ledger_dir", &request.ledger_dir),
        ("current.version", &request.current.version),
    ] {
        if value.trim().is_empty() {
            return invalid(format!("{field} must not be empty"));
        }
    }
    validate_artifact("observations_artifact", &request.observations_artifact)?;
    validate_artifact("rollback_artifact", &request.rollback_artifact)?;
    validate_schedule(&request.schedule)?;
    validate_current(
        &request.current,
        &request.schedule,
        request.observations.len(),
    )?;
    if request.observations.len() < request.schedule.min_rows {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA,
            format!(
                "parameter adaptation needs >= {} observations, got {}",
                request.schedule.min_rows,
                request.observations.len()
            ),
        ));
    }
    if new_observation_count(request) < request.schedule.min_new_rows {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA,
            "scheduled adaptation did not receive enough new observations",
        ));
    }
    for idx in 0..request.observations.len() {
        validate_row(request, idx)?;
    }
    Ok(())
}

fn validate_schedule(schedule: &ParameterAdaptationSchedule) -> Result<()> {
    if schedule.scheduled_at_ts <= schedule.previous_run_ts {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_LOOKAHEAD,
            "scheduled_at_ts must be after previous_run_ts",
        ));
    }
    if schedule.min_rows < PARAMETER_ADAPTATION_MIN_ROWS
        || schedule.min_new_rows == 0
        || schedule.max_te_lag == 0
        || schedule.candidate_knn_k.is_empty()
        || !schedule.min_brier_improvement.is_finite()
        || schedule.min_brier_improvement <= 0.0
    {
        return invalid("schedule has invalid sample floors, lags, k candidates, or improvement");
    }
    Ok(())
}

fn validate_current(
    current: &ParameterSetSnapshot,
    schedule: &ParameterAdaptationSchedule,
    n: usize,
) -> Result<()> {
    if !current.encoder_sigma.is_finite() || current.encoder_sigma <= 0.0 {
        return invalid("current encoder_sigma must be finite and positive");
    }
    validate_edges(&current.quantile_edges)?;
    if current.te_lag == 0 || current.te_lag > schedule.max_te_lag {
        return invalid("current te_lag must be within the scheduled lag search range");
    }
    if current.knn_k == 0 || current.knn_k >= n {
        return invalid("current knn_k must be in 1..observation_count");
    }
    Ok(())
}

fn validate_row(request: &ParameterAdaptationRequest, idx: usize) -> Result<()> {
    let row = &request.observations[idx];
    if row.ts > request.schedule.scheduled_at_ts {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_LOOKAHEAD,
            format!("observation {idx} is after the scheduled adaptation timestamp"),
        ));
    }
    if idx > 0 && request.observations[idx - 1].ts >= row.ts {
        return malformed(format!(
            "observation {idx} timestamps must be strictly increasing"
        ));
    }
    for (name, value) in [
        ("scalar_value", row.scalar_value),
        ("heavy_tail_value", row.heavy_tail_value),
        ("lag_signal", row.lag_signal),
    ] {
        if !value.is_finite() {
            return malformed(format!("observation {idx} {name} must be finite"));
        }
    }
    let dim = request.observations[0].knn_vector.len();
    if row.knn_vector.len() != dim || dim == 0 || row.knn_vector.iter().any(|v| !v.is_finite()) {
        return malformed(format!("observation {idx} kNN vector is malformed"));
    }
    Ok(())
}

fn validate_artifact(label: &str, artifact: &ParameterAdaptationArtifactRef) -> Result<()> {
    if artifact.path.trim().is_empty()
        || artifact.blake3.len() != 64
        || !artifact.blake3.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return invalid(format!("{label} requires a path and 64-character BLAKE3"));
    }
    let bytes = fs::read(&artifact.path).map_err(|err| {
        PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT,
            format!("read {label} {}: {err}", artifact.path),
        )
    })?;
    let observed = blake3::hash(&bytes).to_hex().to_string();
    if observed != artifact.blake3 {
        return Err(PolyError::diagnostics(
            ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT,
            format!("{label} {} hash mismatch", artifact.path),
        ));
    }
    Ok(())
}

fn changed_parameters(
    current: &ParameterSetSnapshot,
    proposed: &ParameterSetSnapshot,
) -> Vec<String> {
    let mut changed = Vec::new();
    if (current.encoder_sigma - proposed.encoder_sigma).abs() > 1.0e-9 {
        changed.push("encoder_sigma".to_string());
    }
    if current.quantile_edges.len() != proposed.quantile_edges.len()
        || current
            .quantile_edges
            .iter()
            .zip(&proposed.quantile_edges)
            .any(|(a, b)| (*a - *b).abs() > 1.0e-9)
    {
        changed.push("quantile_edges".to_string());
    }
    if current.te_lag != proposed.te_lag {
        changed.push("te_lag".to_string());
    }
    if current.knn_k != proposed.knn_k {
        changed.push("knn_k".to_string());
    }
    changed
}

fn reports_equivalent(
    readback: &ParameterAdaptationReport,
    expected: &ParameterAdaptationReport,
) -> bool {
    readback.schema_version == expected.schema_version
        && readback.artifact_kind == expected.artifact_kind
        && readback.domain == expected.domain
        && readback.horizon_bucket == expected.horizon_bucket
        && readback.status == expected.status
        && readback.reason == expected.reason
        && readback.observation_count == expected.observation_count
        && readback.new_observation_count == expected.new_observation_count
        && parameter_set_equivalent(&readback.previous, &expected.previous)
        && parameter_set_equivalent(&readback.proposed, &expected.proposed)
        && metrics_equivalent(&readback.metrics, &expected.metrics)
        && readback.changed_parameters == expected.changed_parameters
        && readback.observations_artifact == expected.observations_artifact
        && readback.rollback_artifact == expected.rollback_artifact
        && readback.ledger_entry == expected.ledger_entry
        && readback.report_hash == expected.report_hash
}

fn parameter_set_equivalent(a: &ParameterSetSnapshot, b: &ParameterSetSnapshot) -> bool {
    a.version == b.version
        && close(a.encoder_sigma, b.encoder_sigma)
        && a.te_lag == b.te_lag
        && a.knn_k == b.knn_k
        && a.quantile_edges.len() == b.quantile_edges.len()
        && a.quantile_edges
            .iter()
            .zip(&b.quantile_edges)
            .all(|(x, y)| close(*x, *y))
}

fn metrics_equivalent(a: &ParameterAdaptationMetrics, b: &ParameterAdaptationMetrics) -> bool {
    close(a.current_knn_brier, b.current_knn_brier)
        && close(a.selected_knn_brier, b.selected_knn_brier)
        && close(a.brier_improvement, b.brier_improvement)
        && close(a.selected_te_score, b.selected_te_score)
        && close(a.selected_sigma, b.selected_sigma)
        && a.selected_knn_k == b.selected_knn_k
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0e-12
}

fn report_hash(
    request: &ParameterAdaptationRequest,
    proposed: &ParameterSetSnapshot,
    metrics: &ParameterAdaptationMetrics,
    changed: &[String],
    status: ParameterAdaptationStatus,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("{:?}", status).as_bytes());
    hasher.update(request.observations_artifact.blake3.as_bytes());
    hasher.update(request.rollback_artifact.blake3.as_bytes());
    hasher.update(request.current.version.as_bytes());
    hasher.update(proposed.version.as_bytes());
    hasher.update(&metrics.brier_improvement.to_le_bytes());
    for name in changed {
        hasher.update(name.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn new_observation_count(request: &ParameterAdaptationRequest) -> usize {
    request
        .observations
        .iter()
        .filter(|row| row.ts > request.schedule.previous_run_ts)
        .count()
}

fn malformed<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PARAMETER_ADAPTATION_MALFORMED_ROW,
        message.into(),
    ))
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PARAMETER_ADAPTATION_INVALID_REQUEST,
        message.into(),
    ))
}
