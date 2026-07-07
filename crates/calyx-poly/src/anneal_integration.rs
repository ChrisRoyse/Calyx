//! Reversible Calyx Anneal integration for index/fusion/tau tuning (issue #104).

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealAction, BudgetHandle, HeldOutReplay, ReplayQuery, ShadowExecutor,
    ShadowVerdict, TripwireMetric, TripwireRegistry, read_tripwire_config_from_vault,
    validate_index_config, validate_mat_plan_config,
};
use calyx_core::FixedClock;

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub use crate::anneal_integration_types::*;

pub fn run_anneal_integration_report(
    request: &AnnealIntegrationRequest,
    output_root: &Path,
) -> Result<AnnealIntegrationRun> {
    let ledger_path = Path::new(&request.ledger_dir).join(ANNEAL_INTEGRATION_LEDGER_FILE);
    let before = read_anneal_integration_ledger_entries(&ledger_path)?;
    let report = compute_anneal_integration_report(request, before.len() as u64)?;
    let report_path = write_anneal_integration_report(output_root, &report)?;
    append_ledger_entry(&ledger_path, &report.ledger_entry)?;
    let readback = read_anneal_integration_report(&report_path)?;
    let ledger_entries = read_anneal_integration_ledger_entries(&ledger_path)?;
    if readback.status != report.status
        || readback.reason != report.reason
        || readback.report_hash != report.report_hash
        || readback.ledger_entry != report.ledger_entry
    {
        return Err(PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_READBACK_MISMATCH,
            format!(
                "anneal integration report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    if ledger_entries.last() != Some(&report.ledger_entry)
        || ledger_entries.len() != before.len() + 1
    {
        return Err(PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_READBACK_MISMATCH,
            format!(
                "anneal integration ledger {} did not read back appended entry",
                ledger_path.display()
            ),
        ));
    }
    Ok(AnnealIntegrationRun {
        report_path,
        ledger_path,
        report: readback,
        ledger_entries,
    })
}

pub fn compute_anneal_integration_report(
    request: &AnnealIntegrationRequest,
    next_ledger_sequence: u64,
) -> Result<AnnealIntegrationReport> {
    validate_request(request)?;
    let tripwire = load_tripwires(request)?;
    let incumbent = StaticMetricAction::from_rows(&request.incumbent_metrics);
    let candidate = StaticMetricAction::from_rows(&request.candidate_metrics);
    let replay = HeldOutReplay {
        queries: request.replay_queries.clone(),
        seed: request.replay_seed,
    };
    let clock = FixedClock::new(request.generated_at_ts);
    let mut executor = ShadowExecutor::new(
        tripwire.registry,
        replay,
        BudgetHandle::new(request.budget_ticks),
        &clock,
    );
    let verdict = executor.run_shadow(&candidate, &incumbent);
    let metrics = verdict_metrics(&verdict);
    let promoted = matches!(verdict, ShadowVerdict::Promote { .. });
    let status = if promoted {
        AnnealIntegrationStatus::Promoted
    } else {
        AnnealIntegrationStatus::Reverted
    };
    let changed_parameters = changed_parameters(&request.current, &request.candidate);
    let active_after = if promoted {
        request.candidate.clone()
    } else {
        request.current.clone()
    };
    let report_hash = report_hash(request, status, &verdict, &changed_parameters);
    let ledger_entry = AnnealIntegrationLedgerEntry {
        schema_version: ANNEAL_INTEGRATION_SCHEMA_VERSION.to_string(),
        sequence: next_ledger_sequence,
        domain: request.domain.clone(),
        scope_id: request.scope_id.clone(),
        previous_version: request.current.version.clone(),
        candidate_version: request.candidate.version.clone(),
        active_version: active_after.version.clone(),
        status,
        changed_parameters: changed_parameters.clone(),
        replay_hash: request.replay_artifact.blake3.clone(),
        rollback_hash: request.rollback_artifact.blake3.clone(),
        report_hash: report_hash.clone(),
        generated_at_ts: request.generated_at_ts,
    };
    Ok(AnnealIntegrationReport {
        schema_version: ANNEAL_INTEGRATION_SCHEMA_VERSION.to_string(),
        artifact_kind: ANNEAL_INTEGRATION_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        scope_id: request.scope_id.clone(),
        status,
        reason: verdict_reason(&verdict),
        replay_query_count: request.replay_queries.len(),
        budget_ticks: request.budget_ticks,
        previous: request.current.clone(),
        candidate: request.candidate.clone(),
        active_after,
        changed_parameters,
        shadow_verdict: verdict,
        metrics,
        replay_artifact: request.replay_artifact.clone(),
        rollback_artifact: request.rollback_artifact.clone(),
        tripwire_config_path: tripwire.config_path,
        tripwire_thresholds: tripwire.thresholds,
        ledger_entry,
        report_hash,
    })
}

pub fn write_anneal_integration_report(
    dir: &Path,
    report: &AnnealIntegrationReport,
) -> Result<PathBuf> {
    write_json(dir, ANNEAL_INTEGRATION_REPORT_FILE, report)
}

pub fn read_anneal_integration_report(path: &Path) -> Result<AnnealIntegrationReport> {
    read_json(path)
}

pub fn read_anneal_integration_ledger_entries(
    path: &Path,
) -> Result<Vec<AnnealIntegrationLedgerEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_LEDGER_IO,
            format!("read anneal integration ledger {}: {err}", path.display()),
        )
    })?;
    let mut entries = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(PolyError::diagnostics(
                ERR_ANNEAL_INTEGRATION_LEDGER_DECODE,
                format!("ledger {} line {} is empty", path.display(), idx + 1),
            ));
        }
        entries.push(serde_json::from_str(line).map_err(|err| {
            PolyError::diagnostics(
                ERR_ANNEAL_INTEGRATION_LEDGER_DECODE,
                format!("decode ledger {} line {}: {err}", path.display(), idx + 1),
            )
        })?);
    }
    Ok(entries)
}

struct TripwireReadback {
    registry: TripwireRegistry,
    config_path: String,
    thresholds: Vec<calyx_anneal::TripwireThresholdEntry>,
}

struct StaticMetricAction {
    by_query: HashMap<u64, ActionMetricSnapshot>,
}

impl StaticMetricAction {
    fn from_rows(rows: &[AnnealIntegrationMetricRow]) -> Self {
        let by_query = rows
            .iter()
            .map(|row| {
                (
                    row.query_id,
                    ActionMetricSnapshot::from_values(row.metrics.metric_values()),
                )
            })
            .collect();
        Self { by_query }
    }
}

impl AnnealAction for StaticMetricAction {
    fn apply_shadow(&self, query: &ReplayQuery) -> ActionMetricSnapshot {
        self.by_query
            .get(&query.query_id)
            .cloned()
            .unwrap_or_else(|| {
                ActionMetricSnapshot::from_values(std::iter::empty::<(TripwireMetric, f64)>())
            })
    }
}

fn validate_request(request: &AnnealIntegrationRequest) -> Result<()> {
    for (field, value) in [
        ("domain", &request.domain),
        ("scope_id", &request.scope_id),
        ("tripwire_vault", &request.tripwire_vault),
        ("ledger_dir", &request.ledger_dir),
        ("current.version", &request.current.version),
        ("candidate.version", &request.candidate.version),
    ] {
        if value.trim().is_empty() {
            return invalid(format!("{field} must not be empty"));
        }
    }
    validate_param_set("current", &request.current)?;
    validate_param_set("candidate", &request.candidate)?;
    validate_replay_artifact(request)?;
    validate_rollback_artifact(request)?;
    validate_replay_queries(&request.replay_queries)?;
    validate_metric_rows(
        "incumbent_metrics",
        &request.replay_queries,
        &request.incumbent_metrics,
    )?;
    validate_metric_rows(
        "candidate_metrics",
        &request.replay_queries,
        &request.candidate_metrics,
    )?;
    validate_tripwire_bounds(request.tripwire_bounds)?;
    Ok(())
}

fn validate_param_set(label: &str, params: &AnnealIntegrationParamSet) -> Result<()> {
    validate_index_config(&params.index).map_err(PolyError::from)?;
    validate_mat_plan_config(&params.fusion).map_err(PolyError::from)?;
    if !params.tau.is_finite() || params.tau <= 0.0 {
        return invalid(format!("{label}.tau must be finite and positive"));
    }
    Ok(())
}

fn validate_replay_artifact(request: &AnnealIntegrationRequest) -> Result<()> {
    let bytes = read_hashed_artifact("replay_artifact", &request.replay_artifact)?;
    let decoded: Vec<ReplayQuery> = serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            format!(
                "decode replay_artifact {} as replay queries: {err}",
                request.replay_artifact.path
            ),
        )
    })?;
    if decoded != request.replay_queries {
        return Err(PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            "replay_artifact does not match request replay queries",
        ));
    }
    Ok(())
}

fn validate_rollback_artifact(request: &AnnealIntegrationRequest) -> Result<()> {
    let bytes = read_hashed_artifact("rollback_artifact", &request.rollback_artifact)?;
    let decoded: AnnealIntegrationParamSet = serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            format!(
                "decode rollback_artifact {} as parameter set: {err}",
                request.rollback_artifact.path
            ),
        )
    })?;
    if decoded != request.current {
        return Err(PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            "rollback_artifact does not match current parameter set",
        ));
    }
    Ok(())
}

fn read_hashed_artifact(label: &str, artifact: &AnnealIntegrationArtifactRef) -> Result<Vec<u8>> {
    if artifact.path.trim().is_empty()
        || artifact.blake3.len() != 64
        || !artifact.blake3.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return invalid(format!("{label} requires a path and 64-character BLAKE3"));
    }
    let bytes = fs::read(&artifact.path).map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            format!("read {label} {}: {err}", artifact.path),
        )
    })?;
    let observed = blake3::hash(&bytes).to_hex().to_string();
    if observed != artifact.blake3 {
        return Err(PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT,
            format!("{label} {} hash mismatch", artifact.path),
        ));
    }
    Ok(bytes)
}

fn validate_replay_queries(queries: &[ReplayQuery]) -> Result<()> {
    let mut ids = HashSet::new();
    for query in queries {
        if !ids.insert(query.query_id) {
            return invalid(format!("duplicate replay query id {}", query.query_id));
        }
        if query.query_vector.is_empty() || query.query_vector.iter().any(|v| !v.is_finite()) {
            return invalid(format!(
                "replay query {} vector is malformed",
                query.query_id
            ));
        }
        if query.expected_top_k.is_empty()
            || query
                .expected_top_k
                .iter()
                .any(|anchor| !anchor.similarity.is_finite())
        {
            return invalid(format!(
                "replay query {} expected_top_k is malformed",
                query.query_id
            ));
        }
    }
    Ok(())
}

fn validate_metric_rows(
    label: &str,
    queries: &[ReplayQuery],
    rows: &[AnnealIntegrationMetricRow],
) -> Result<()> {
    let expected: HashSet<u64> = queries.iter().map(|query| query.query_id).collect();
    let mut observed = HashSet::new();
    for row in rows {
        if !expected.contains(&row.query_id) {
            return invalid(format!(
                "{label} contains unknown query id {}",
                row.query_id
            ));
        }
        if !observed.insert(row.query_id) {
            return invalid(format!("{label} duplicates query id {}", row.query_id));
        }
    }
    if observed != expected {
        return invalid(format!(
            "{label} must contain exactly one row per replay query"
        ));
    }
    Ok(())
}

fn validate_tripwire_bounds(bounds: AnnealIntegrationTripwireBounds) -> Result<()> {
    for (metric, value) in bounds.metric_bounds() {
        if !value.is_finite() || value < 0.0 {
            return invalid(format!(
                "{metric:?} tripwire bound must be finite and nonnegative"
            ));
        }
    }
    if !bounds.hysteresis.is_finite() || bounds.hysteresis < 0.0 {
        return invalid("tripwire hysteresis must be finite and nonnegative");
    }
    Ok(())
}

fn load_tripwires(request: &AnnealIntegrationRequest) -> Result<TripwireReadback> {
    let mut registry =
        TripwireRegistry::load_from_vault(&request.tripwire_vault).map_err(PolyError::from)?;
    for (metric, bound) in request.tripwire_bounds.metric_bounds() {
        registry
            .set_tripwire(metric, bound, request.tripwire_bounds.hysteresis)
            .map_err(PolyError::from)?;
    }
    let readback =
        read_tripwire_config_from_vault(&request.tripwire_vault).map_err(PolyError::from)?;
    Ok(TripwireReadback {
        registry,
        config_path: readback.config_path.display().to_string(),
        thresholds: readback.thresholds,
    })
}

fn append_ledger_entry(path: &Path, entry: &AnnealIntegrationLedgerEntry) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::diagnostics(
                ERR_ANNEAL_INTEGRATION_LEDGER_IO,
                format!("create ledger dir {}: {err}", parent.display()),
            )
        })?;
    }
    let line = serde_json::to_string(entry).map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_LEDGER_DECODE,
            format!("encode anneal integration ledger entry: {err}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| {
            PolyError::diagnostics(
                ERR_ANNEAL_INTEGRATION_LEDGER_IO,
                format!("open anneal integration ledger {}: {err}", path.display()),
            )
        })?;
    writeln!(file, "{line}").map_err(|err| {
        PolyError::diagnostics(
            ERR_ANNEAL_INTEGRATION_LEDGER_IO,
            format!("append anneal integration ledger {}: {err}", path.display()),
        )
    })
}

fn changed_parameters(
    current: &AnnealIntegrationParamSet,
    candidate: &AnnealIntegrationParamSet,
) -> Vec<String> {
    let mut changed = Vec::new();
    if current.index != candidate.index {
        changed.push("index".to_string());
    }
    if current.fusion != candidate.fusion {
        changed.push("fusion".to_string());
    }
    if (current.tau - candidate.tau).abs() > 1.0e-12 {
        changed.push("tau".to_string());
    }
    changed
}

fn verdict_metrics(verdict: &ShadowVerdict) -> calyx_anneal::MetricSnapshot {
    match verdict {
        ShadowVerdict::Promote { metrics } | ShadowVerdict::Revert { metrics, .. } => {
            metrics.clone()
        }
    }
}

fn verdict_reason(verdict: &ShadowVerdict) -> String {
    match verdict {
        ShadowVerdict::Promote { .. } => "shadow_promoted".to_string(),
        ShadowVerdict::Revert { reason, .. } => format!("shadow_reverted:{reason:?}"),
    }
}

fn report_hash(
    request: &AnnealIntegrationRequest,
    status: AnnealIntegrationStatus,
    verdict: &ShadowVerdict,
    changed: &[String],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("{status:?}").as_bytes());
    hasher.update(format!("{verdict:?}").as_bytes());
    hasher.update(request.replay_artifact.blake3.as_bytes());
    hasher.update(request.rollback_artifact.blake3.as_bytes());
    hasher.update(request.current.version.as_bytes());
    hasher.update(request.candidate.version.as_bytes());
    for name in changed {
        hasher.update(name.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_ANNEAL_INTEGRATION_INVALID_REQUEST,
        message.into(),
    ))
}
