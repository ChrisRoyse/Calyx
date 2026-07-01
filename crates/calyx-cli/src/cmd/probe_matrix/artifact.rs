use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{CalyxError, CxId, SlotId};
use calyx_lodestar::{
    PROBE_MATRIX_SCHEMA_VERSION, ProbeMatrixLog, ProbeMatrixSpec, ProbeProductivity, ProbeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ProbeMatrixArgs;
use super::diagnostics::{
    ProbeMatrixArtifactStatus, ProbeMatrixDiagnostics, ProbeMatrixVariantDiagnostic,
    QueryVectorCache,
};
use super::persist::{self, persist_probe_matrix_at_path};
use crate::cmd::vault::ResolvedVault;
use crate::error::{CliError, CliResult};
use crate::fsv_grounding::GroundingAudit;

const PROBE_MATRIX_ARTIFACT_SCHEMA_VERSION: u32 = 6;
const PROBE_MATRIX_INCOMPLETE: &str = "CALYX_PROBE_MATRIX_INCOMPLETE";
const PROBE_MATRIX_INCOMPLETE_REMEDIATION: &str = "inspect the persisted matrix/progress artifacts, then increase the budget or narrow explicit axes";
const PROBE_MATRIX_TIMEOUT_REMEDIATION: &str = "inspect the persisted matrix/progress artifacts, then increase --time-budget-ms or narrow explicit axes";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProbeMatrixRun {
    pub(super) total_variant_count: usize,
    pub(super) completed_variant_count: usize,
    pub(super) next_variant_index: Option<usize>,
    pub(super) resume_token: Option<String>,
    pub(super) max_variants: Option<usize>,
    pub(super) time_budget_ms: Option<u64>,
    pub(super) elapsed_ms: u128,
    pub(super) complete: bool,
    pub(super) stop_reason: Option<String>,
    pub(super) progress_artifact: String,
    pub(super) partial_matrix_artifact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProbeMatrixArtifact {
    pub(super) schema_version: u32,
    pub(super) status: ProbeMatrixArtifactStatus,
    pub(super) vault: String,
    pub(super) vault_id: String,
    pub(super) vault_dir: String,
    pub(super) active_slots: Vec<SlotId>,
    pub(super) diagnostics: ProbeMatrixDiagnostics,
    pub(super) run: ProbeMatrixRun,
    pub(super) log: ProbeMatrixLog,
}
pub(super) struct MatrixArtifactWriter<'a> {
    matrix_path: std::path::PathBuf,
    progress_path: std::path::PathBuf,
    resolved: &'a ResolvedVault,
    spec: &'a ProbeMatrixSpec,
    args: &'a ProbeMatrixArgs,
    total_variant_count: usize,
}

impl<'a> MatrixArtifactWriter<'a> {
    pub(super) fn new(
        matrix_path: &Path,
        resolved: &'a ResolvedVault,
        spec: &'a ProbeMatrixSpec,
        args: &'a ProbeMatrixArgs,
        total_variant_count: usize,
        progress_path: &Path,
    ) -> Self {
        Self {
            matrix_path: matrix_path.to_path_buf(),
            progress_path: progress_path.to_path_buf(),
            resolved,
            spec,
            args,
            total_variant_count,
        }
    }

    pub(super) fn persist_incomplete(
        &self,
        records: &[ProbeRecord],
        query_cache: &QueryVectorCache,
        search_cache: &calyx_search::SearchSlotCache,
        guard_diagnostics: &[ProbeMatrixVariantDiagnostic],
        elapsed_ms: u128,
        reason: &str,
    ) -> CliResult<persist::PersistedProbeMatrix> {
        let run = self.run_state(records.len(), elapsed_ms, false, Some(reason));
        self.persist_run(
            records,
            query_cache,
            search_cache,
            guard_diagnostics,
            ProbeMatrixArtifactStatus::Incomplete,
            run,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn persist_incomplete_with_grounding(
        &self,
        records: &[ProbeRecord],
        query_cache: &QueryVectorCache,
        search_cache: &calyx_search::SearchSlotCache,
        guard_diagnostics: &[ProbeMatrixVariantDiagnostic],
        grounding_preflight: &GroundingAudit,
        elapsed_ms: u128,
        reason: &str,
    ) -> CliResult<persist::PersistedProbeMatrix> {
        let run = self.run_state(records.len(), elapsed_ms, false, Some(reason));
        let log = matrix_log(self.spec, records);
        let mut artifact = self.artifact_for(
            query_cache,
            search_cache,
            guard_diagnostics,
            ProbeMatrixArtifactStatus::Incomplete,
            run,
            log,
        );
        artifact.diagnostics.grounding_preflight = Some(grounding_preflight.clone());
        persist_probe_matrix_at_path(&self.matrix_path, &artifact, true)
    }

    pub(super) fn persist_run(
        &self,
        records: &[ProbeRecord],
        query_cache: &QueryVectorCache,
        search_cache: &calyx_search::SearchSlotCache,
        guard_diagnostics: &[ProbeMatrixVariantDiagnostic],
        status: ProbeMatrixArtifactStatus,
        run: ProbeMatrixRun,
    ) -> CliResult<persist::PersistedProbeMatrix> {
        let log = matrix_log(self.spec, records);
        let artifact = self.artifact_for(
            query_cache,
            search_cache,
            guard_diagnostics,
            status,
            run,
            log,
        );
        persist_probe_matrix_at_path(&self.matrix_path, &artifact, true)
    }

    pub(super) fn artifact_for(
        &self,
        query_cache: &QueryVectorCache,
        search_cache: &calyx_search::SearchSlotCache,
        guard_diagnostics: &[ProbeMatrixVariantDiagnostic],
        status: ProbeMatrixArtifactStatus,
        run: ProbeMatrixRun,
        log: ProbeMatrixLog,
    ) -> ProbeMatrixArtifact {
        ProbeMatrixArtifact {
            schema_version: PROBE_MATRIX_ARTIFACT_SCHEMA_VERSION,
            status,
            vault: self.resolved.name.clone(),
            vault_id: self.resolved.vault_id.to_string(),
            vault_dir: self.resolved.path.display().to_string(),
            active_slots: self.spec.active_slots.clone(),
            diagnostics: ProbeMatrixDiagnostics {
                query_measurements: query_cache.diagnostics(),
                search_result_cache: search_cache.diagnostics(),
                variant_guard_counts: guard_diagnostics.to_vec(),
                grounding_preflight: None,
            },
            run,
            log,
        }
    }

    pub(super) fn run_state(
        &self,
        completed_variant_count: usize,
        elapsed_ms: u128,
        complete: bool,
        stop_reason: Option<&str>,
    ) -> ProbeMatrixRun {
        let next_variant_index =
            (completed_variant_count < self.total_variant_count).then_some(completed_variant_count);
        ProbeMatrixRun {
            total_variant_count: self.total_variant_count,
            completed_variant_count,
            next_variant_index,
            resume_token: next_variant_index.map(|idx| format!("variant:{idx}")),
            max_variants: self.args.max_variants,
            time_budget_ms: self.args.time_budget_ms,
            elapsed_ms,
            complete,
            stop_reason: stop_reason.map(str::to_string),
            progress_artifact: self.progress_path.display().to_string(),
            partial_matrix_artifact: self.matrix_path.display().to_string(),
        }
    }
}

pub(super) fn matrix_log(spec: &ProbeMatrixSpec, records: &[ProbeRecord]) -> ProbeMatrixLog {
    let mut records = records.to_vec();
    attach_unique_hits(&mut records);
    let productive = productive_rows(&records);
    ProbeMatrixLog {
        schema_version: PROBE_MATRIX_SCHEMA_VERSION,
        spec: spec.clone(),
        records,
        productive,
    }
}

pub(super) fn incomplete_error(reason: &str, matrix_path: &Path, progress_path: &Path) -> CliError {
    CalyxError {
        code: PROBE_MATRIX_INCOMPLETE,
        message: format!(
            "probe-matrix stopped before full matrix reason={reason}; matrix artifact persisted at {}; progress artifact persisted at {}",
            matrix_path.display(),
            progress_path.display()
        ),
        remediation: PROBE_MATRIX_INCOMPLETE_REMEDIATION,
    }
    .into()
}

pub(super) fn timeout_with_artifacts(
    error: &CliError,
    matrix_path: &Path,
    progress_path: &Path,
) -> CliError {
    CalyxError {
        code: "CALYX_CLI_TIMEOUT",
        message: format!(
            "{}; matrix artifact persisted at {}; progress artifact persisted at {}",
            error.message(),
            matrix_path.display(),
            progress_path.display()
        ),
        remediation: PROBE_MATRIX_TIMEOUT_REMEDIATION,
    }
    .into()
}

pub(super) fn error_details(error: &CliError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.message(),
    })
}

fn attach_unique_hits(records: &mut [ProbeRecord]) {
    let mut counts = BTreeMap::<CxId, usize>::new();
    for record in records.iter() {
        for hit in record.hits.iter().filter(|hit| hit.grounded) {
            *counts.entry(hit.cx_id).or_default() += 1;
        }
    }
    for record in records {
        record.unique_grounded_hits = record
            .hits
            .iter()
            .filter(|hit| hit.grounded && counts.get(&hit.cx_id) == Some(&1))
            .map(|hit| hit.cx_id)
            .collect();
    }
}

fn productive_rows(records: &[ProbeRecord]) -> Vec<ProbeProductivity> {
    let mut rows: Vec<_> = records
        .iter()
        .filter(|record| record.accepted_hit_count > 0)
        .map(|record| ProbeProductivity {
            variant_id: record.variant.id,
            fusion: record.variant.fusion.clone(),
            phrasing: record.variant.phrasing,
            length: record.variant.length,
            lens_emphasis: record.variant.lens_emphasis.clone(),
            unique_hit_count: record.unique_grounded_hits.len(),
            accepted_hit_count: record.accepted_hit_count,
            refusal_count: record.refusals.len(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .unique_hit_count
            .cmp(&left.unique_hit_count)
            .then_with(|| right.accepted_hit_count.cmp(&left.accepted_hit_count))
            .then_with(|| left.variant_id.cmp(&right.variant_id))
    });
    rows
}
