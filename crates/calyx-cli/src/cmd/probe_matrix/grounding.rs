use calyx_aster::vault::AsterVault;
use calyx_lodestar::{ProbeMatrixSpec, ProbeRecord};
use serde_json::json;

use super::artifact::{MatrixArtifactWriter, error_details};
use super::diagnostics::{ProbeMatrixVariantDiagnostic, QueryVectorCache};
use super::progress::ProbeMatrixProgressWriter;
use super::support::with_persisted_artifact_error;
use crate::error::{CliError, CliResult};

pub(super) struct GroundingPreflight<'a> {
    pub(super) vault: &'a AsterVault,
    pub(super) spec: &'a ProbeMatrixSpec,
    pub(super) artifacts: &'a MatrixArtifactWriter<'a>,
    pub(super) records: &'a [ProbeRecord],
    pub(super) query_cache: &'a QueryVectorCache,
    pub(super) guard_diagnostics: &'a [ProbeMatrixVariantDiagnostic],
    pub(super) elapsed_ms: u128,
}

impl GroundingPreflight<'_> {
    pub(super) fn run(self, progress: &mut ProbeMatrixProgressWriter) -> CliResult {
        progress.write(
            "running",
            "grounding_preflight_start",
            json!({ "active_slots": &self.spec.active_slots }),
        )?;
        let grounding_preflight =
            match crate::fsv_grounding::audit_grounding(self.vault, &self.spec.active_slots) {
                Ok(audit) => audit,
                Err(error) => {
                    let persisted = self.artifacts.persist_incomplete(
                        self.records,
                        self.query_cache,
                        self.guard_diagnostics,
                        self.elapsed_ms,
                        "grounding_preflight_error",
                    )?;
                    progress.write(
                        "failed",
                        "grounding_preflight_error",
                        json!({
                            "error": error_details(&error),
                            "matrix_artifact": persisted.path.display().to_string(),
                            "matrix_json_sha256": persisted.sha256,
                        }),
                    )?;
                    return Err(with_persisted_artifact_error(error, &persisted));
                }
            };
        progress.write(
            "running",
            "grounding_preflight_complete",
            json!({ "grounding": &grounding_preflight }),
        )?;
        if let Some(error) = crate::fsv_grounding::grounding_failure_for_probe(&grounding_preflight)
        {
            let error = CliError::from(error);
            let persisted = self.artifacts.persist_incomplete_with_grounding(
                self.records,
                self.query_cache,
                self.guard_diagnostics,
                &grounding_preflight,
                self.elapsed_ms,
                "grounding_preflight_failed",
            )?;
            progress.write(
                "failed",
                "grounding_preflight_failed",
                json!({
                    "error": error_details(&error),
                    "grounding": &grounding_preflight,
                    "matrix_artifact": persisted.path.display().to_string(),
                    "matrix_json_sha256": persisted.sha256,
                }),
            )?;
            return Err(with_persisted_artifact_error(error, &persisted));
        }
        Ok(())
    }
}
