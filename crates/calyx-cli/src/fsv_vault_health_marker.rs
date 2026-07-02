//! `fsv vault-health` check for the durable search-index rebuild-required
//! marker (issue #1089).

use std::path::Path;

use serde_json::json;

use crate::error::CliError;
use crate::fsv_vault_health::{VaultHealthCheck, failed, failed_from_cli, ok};

pub(crate) const REBUILD_REQUIRED_CODE: &str = "CALYX_SEARCH_REBUILD_REQUIRED";

/// Fail-closed readback of the durable rebuild-required marker. A present
/// marker means a mutation committed Base rows but no search-index rebuild has
/// completed since — even if the seq comparison happens to look clean, the
/// marker survives until a rebuild proves the derived state and clears it, so
/// the check reports the recorded commit context verbatim.
pub(crate) fn check_search_rebuild_marker(vault_dir: &Path) -> VaultHealthCheck {
    let marker_path = calyx_search::rebuild_required_marker_path(vault_dir);
    match calyx_search::read_rebuild_required_marker(vault_dir) {
        Ok(None) => ok(
            "search_rebuild_marker",
            "no rebuild-required marker present; no interrupted ingest or index rebuild recorded",
            json!({"marker_path": marker_path.display().to_string()}),
        ),
        Ok(Some(marker)) => failed(
            "search_rebuild_marker",
            REBUILD_REQUIRED_CODE,
            format!(
                "rebuild-required marker present: source={} required_base_seq={} written_at_unix_ms={} process_id={} session_id={} detail={}",
                marker.source,
                marker
                    .required_base_seq
                    .map(|seq| seq.to_string())
                    .unwrap_or_else(|| "in-flight".to_string()),
                marker.written_at_unix_ms,
                marker.process_id,
                marker.session_id.as_deref().unwrap_or("<none>"),
                marker.detail
            ),
            "run `calyx rebuild-search-index <vault>` (resumes staged slot artifacts from the interrupted run) and rerun vault-health",
            json!({
                "marker_path": marker_path.display().to_string(),
                "source": marker.source,
                "required_base_seq": marker.required_base_seq,
                "manifest_base_seq_at_write": marker.manifest_base_seq_at_write,
                "session_id": marker.session_id,
                "batch_path": marker.batch_path,
                "process_id": marker.process_id,
                "written_at_unix_ms": marker.written_at_unix_ms,
            }),
        ),
        Err(error) => {
            let error: CliError = error.into();
            failed_from_cli(
                "search_rebuild_marker",
                &error,
                json!({"marker_path": marker_path.display().to_string()}),
            )
        }
    }
}
