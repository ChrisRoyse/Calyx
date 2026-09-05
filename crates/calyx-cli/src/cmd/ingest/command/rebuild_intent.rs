//! Search rebuild intent bound to Aster's durable commit sequence.

use std::path::{Path, PathBuf};

use calyx_aster::vault::DurableCommitIntent;
use calyx_core::{CalyxError, Result, Seq};

use super::ingest_runtime_log;

const CALYX_SEARCH_REBUILD_MARKER_INTENT_INVALID: &str =
    "CALYX_SEARCH_REBUILD_MARKER_INTENT_INVALID";

struct PreparedMarker {
    previous: Option<calyx_search::RebuildRequiredMarker>,
    prepared: calyx_search::RebuildRequiredMarker,
}

pub(super) struct SearchRebuildCommitIntent {
    vault_dir: PathBuf,
    source: &'static str,
    detail: String,
    session_id: Option<String>,
    batch_path: Option<String>,
    prepared: Option<PreparedMarker>,
    highest_committed_seq: Option<Seq>,
}

impl SearchRebuildCommitIntent {
    pub(super) fn new(
        vault_dir: &Path,
        source: &'static str,
        detail: String,
        session_id: Option<&str>,
        batch_path: Option<&Path>,
    ) -> Self {
        Self {
            vault_dir: vault_dir.to_path_buf(),
            source,
            detail,
            session_id: session_id.map(str::to_string),
            batch_path: batch_path.map(|path| path.display().to_string()),
            prepared: None,
            highest_committed_seq: None,
        }
    }

    pub(super) const fn highest_committed_seq(&self) -> Option<Seq> {
        self.highest_committed_seq
    }

    fn invalid(&self, message: impl Into<String>) -> CalyxError {
        CalyxError {
            code: CALYX_SEARCH_REBUILD_MARKER_INTENT_INVALID,
            message: message.into(),
            remediation: "preserve the rebuild-required marker, inspect the authoritative Base/WAL sequence and exact marker bytes, then reconcile before retrying",
        }
    }
}

impl DurableCommitIntent for SearchRebuildCommitIntent {
    fn prepare(&mut self, predicted_seq: Seq) -> Result<()> {
        if self.prepared.is_some() {
            return Err(self.invalid(
                "Aster requested a second rebuild-marker prepare before resolving the first",
            ));
        }
        let previous = calyx_search::read_rebuild_required_marker(&self.vault_dir)
            .map_err(CalyxError::from)?;
        let mut marker = match previous.as_ref() {
            Some(existing) => existing.clone(),
            None => {
                let mut marker =
                    calyx_search::RebuildRequiredMarker::new(self.source, self.detail.clone())
                        .map_err(CalyxError::from)?;
                marker.session_id.clone_from(&self.session_id);
                marker.batch_path.clone_from(&self.batch_path);
                marker
            }
        };
        marker.required_base_seq = Some(marker.required_base_seq.unwrap_or(0).max(predicted_seq));
        calyx_search::write_rebuild_required_marker(&self.vault_dir, &marker)
            .map_err(CalyxError::from)?;
        ingest_runtime_log(format_args!(
            "phase=derived_rebuild_marker_prepared source={} predicted_base_seq={} previous_marker={}",
            self.source,
            predicted_seq,
            previous.is_some()
        ));
        self.prepared = Some(PreparedMarker {
            previous,
            prepared: marker,
        });
        Ok(())
    }

    fn committed(&mut self, committed_seq: Seq) -> Result<()> {
        let prepared = self.prepared.take().ok_or_else(|| {
            self.invalid("Aster reported a committed sequence without a prepared rebuild marker")
        })?;
        let mut committed = prepared.prepared.clone();
        committed.required_base_seq =
            Some(committed.required_base_seq.unwrap_or(0).max(committed_seq));
        calyx_search::replace_rebuild_required_marker_if_exact(
            &self.vault_dir,
            &prepared.prepared,
            Some(&committed),
        )
        .map_err(CalyxError::from)?;
        self.highest_committed_seq =
            Some(self.highest_committed_seq.unwrap_or(0).max(committed_seq));
        ingest_runtime_log(format_args!(
            "phase=derived_rebuild_marker_committed source={} required_base_seq={committed_seq}",
            self.source
        ));
        Ok(())
    }

    fn aborted(&mut self) -> Result<()> {
        let prepared = self.prepared.take().ok_or_else(|| {
            self.invalid("Aster reported a no-commit abort without a prepared rebuild marker")
        })?;
        calyx_search::replace_rebuild_required_marker_if_exact(
            &self.vault_dir,
            &prepared.prepared,
            prepared.previous.as_ref(),
        )
        .map_err(CalyxError::from)?;
        ingest_runtime_log(format_args!(
            "phase=derived_rebuild_marker_aborted source={} restored_previous_marker={}",
            self.source,
            prepared.previous.is_some()
        ));
        Ok(())
    }
}
