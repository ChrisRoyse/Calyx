use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::vault::{
    IngestPrecondition, IngestPreconditionClaim, IngestPreconditionContext, IngestVaultState,
};
use calyx_core::CalyxError;
use calyx_ledger::{MAX_UNCLASSIFIED_TOKEN_LEN, RedactionPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::vault::{ResolvedVault, now_ms};
use super::batch::BatchValidation;
use super::store::resolve_cli_vault;
use super::types::BatchIngestSummary;
use crate::durable_write::{DurableWriteLockGuard, write_bytes_atomic, write_bytes_atomic_locked};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod process_identity;
mod signals;

use process_identity::{
    IngestProcessIdentity, OwnerInspection, current_process_identity, inspect_legacy_pid,
    inspect_owner,
};

const LEGACY_SESSION_SCHEMA_VERSION: u16 = 1;
const COUNTER_SESSION_SCHEMA_VERSION: u16 = 2;
const SESSION_SCHEMA_VERSION: u16 = 3;
const SESSION_ROOT: &str = "idx/ingest/runs";
const STATUS_FILE: &str = "status.json";
const HISTORY_DIR: &str = "history";
const CALYX_INGEST_SESSION_CLOCK_FAILED: &str = "CALYX_INGEST_SESSION_CLOCK_FAILED";
const CALYX_INGEST_SESSION_EXISTS: &str = "CALYX_INGEST_SESSION_EXISTS";
const CALYX_INGEST_SESSION_INVALID: &str = "CALYX_INGEST_SESSION_INVALID";
const CALYX_INGEST_SESSION_NOT_FOUND: &str = "CALYX_INGEST_SESSION_NOT_FOUND";
const CALYX_INGEST_SESSION_WRITE_FAILED: &str = "CALYX_INGEST_SESSION_WRITE_FAILED";
const CALYX_INGEST_SESSION_READ_FAILED: &str = "CALYX_INGEST_SESSION_READ_FAILED";
const CALYX_INGEST_SESSION_INCOMPLETE: &str = "CALYX_INGEST_SESSION_INCOMPLETE";
const CALYX_INGEST_SESSION_FAILED: &str = "CALYX_INGEST_SESSION_FAILED";
const CALYX_INGEST_SESSION_ABANDONED: &str = "CALYX_INGEST_SESSION_ABANDONED";
const CALYX_INGEST_SESSION_INTERRUPTED: &str = "CALYX_INGEST_SESSION_INTERRUPTED";
const CALYX_INGEST_SESSION_OWNER_UNKNOWN: &str = "CALYX_INGEST_SESSION_OWNER_UNKNOWN";
const CALYX_INGEST_SESSION_IDENTITY_FAILED: &str = "CALYX_INGEST_SESSION_IDENTITY_FAILED";
const CALYX_INGEST_SESSION_SIGNAL_FAILED: &str = "CALYX_INGEST_SESSION_SIGNAL_FAILED";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IngestStatusArgs {
    pub(crate) vault: String,
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct IngestSessionError {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct IngestSessionTerminal {
    pub(super) kind: String,
    pub(super) signal: Option<String>,
    pub(super) observed_at_unix_ms: u64,
    pub(super) last_durable_phase: String,
    pub(super) previous_status_sha256: String,
    pub(super) observer: IngestProcessIdentity,
    pub(super) detail: String,
    pub(super) remediation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct IngestSessionStatus {
    pub(super) schema_version: u16,
    pub(super) session_id: String,
    pub(super) status: String,
    pub(super) phase: String,
    pub(super) process_id: u32,
    #[serde(default)]
    pub(super) process_identity: Option<IngestProcessIdentity>,
    pub(super) vault_name: String,
    pub(super) vault_id: String,
    pub(super) vault_path: String,
    pub(super) batch_path: String,
    pub(super) batch_sha256: String,
    pub(super) batch_bytes: u64,
    pub(super) batch_line_count: usize,
    pub(super) planned_row_count: usize,
    #[serde(default)]
    pub(super) expected_vault_state: Option<IngestPrecondition>,
    #[serde(default)]
    pub(super) observed_vault_state_before_claim: Option<IngestVaultState>,
    #[serde(default)]
    pub(super) vault_state_claim: Option<IngestPreconditionClaim>,
    pub(super) rows_started: usize,
    pub(super) rows_committed: usize,
    #[serde(default)]
    pub(super) pending_rows: usize,
    #[serde(default)]
    pub(super) uncommitted_started_rows: usize,
    pub(super) committed_new_rows: usize,
    pub(super) already_idempotent_rows: usize,
    pub(super) failed_rows: usize,
    pub(super) distinct_cx_count: usize,
    pub(super) first_cx_id: Option<String>,
    pub(super) last_cx_id: Option<String>,
    pub(super) first_ledger_seq: Option<u64>,
    pub(super) last_ledger_seq: Option<u64>,
    pub(super) final_chain_seq: Option<u64>,
    pub(super) index_rebuild_phase: String,
    pub(super) started_at_unix_ms: u64,
    pub(super) updated_at_unix_ms: u64,
    pub(super) completed_at_unix_ms: Option<u64>,
    pub(super) status_path: String,
    pub(super) error: Option<IngestSessionError>,
    #[serde(default)]
    pub(super) terminal: Option<IngestSessionTerminal>,
}

#[derive(Debug)]
pub(super) struct BatchIngestSession {
    status: IngestSessionStatus,
    status_path: PathBuf,
}

impl BatchIngestSession {
    pub(super) fn start(
        resolved: &ResolvedVault,
        batch_path: &Path,
        validation: &BatchValidation,
        identity: &super::command::batch_identity::BatchSnapshotIdentity,
        requested_session_id: Option<&str>,
    ) -> CliResult<Self> {
        signals::install().map_err(|error| {
            CliError::from(session_error(
                CALYX_INGEST_SESSION_SIGNAL_FAILED,
                error,
                "fix OS signal-handler resources before starting batch ingest; the command did not create a session",
            ))
        })?;
        let process_identity = current_process_identity().map_err(|error| {
            CliError::from(session_error(
                CALYX_INGEST_SESSION_IDENTITY_FAILED,
                format!("capture exact batch-ingest process identity: {error}"),
                "restore access to the host, boot, process-start, and executable identity sources before starting batch ingest",
            ))
        })?;
        let session_id = match requested_session_id {
            Some(value) => {
                validate_session_id(value)?;
                value.to_string()
            }
            None => generated_session_id()?,
        };
        let root = session_dir(&resolved.path, &session_id);
        let parent = root
            .parent()
            .ok_or_else(|| session_invalid("missing session parent"))?;
        // Ingest session directory holds owner-only runtime evidence
        // (status.json); create the parent and root owner-only (0700) so no
        // world/group access is inherited from the operator umask (issue #1931).
        crate::private_fs::create_private_dir_all(parent)?;
        match fs::create_dir(&root) {
            Ok(()) => {
                crate::private_fs::harden_private_dir(&root)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(session_error(
                    CALYX_INGEST_SESSION_EXISTS,
                    format!(
                        "ingest session {session_id} already exists at {}",
                        root.display()
                    ),
                    "use a new --session-id or read the existing session with calyx ingest-status",
                )
                .into());
            }
            Err(error) => {
                return Err(session_write_error(format!(
                    "create ingest session directory {}: {error}",
                    root.display()
                )));
            }
        }
        let canonical_batch = batch_path.canonicalize().map_err(|error| {
            CliError::io(format!(
                "canonicalize batch {}: {error}",
                batch_path.display()
            ))
        })?;
        let batch_bytes = fs::metadata(&canonical_batch)
            .map_err(|error| {
                CliError::io(format!("stat batch {}: {error}", canonical_batch.display()))
            })?
            .len();
        // Re-hash the on-disk bytes and prove they still match the immutable
        // snapshot captured before any authoritative work (issue #1923). This makes
        // the recorded `batch_sha256` an actively verified identity rather than a
        // decorative field, and fails the session before it claims any precondition.
        let batch_sha256 = file_sha256(&canonical_batch)?;
        identity.verify_observed(
            &canonical_batch,
            &super::command::batch_identity::ObservedBatchBytes {
                sha256: batch_sha256.clone(),
                byte_len: batch_bytes,
            },
            "ingest session start",
        )?;
        let status_path = root.join(STATUS_FILE);
        let now = now_ms();
        let status = IngestSessionStatus {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id,
            status: "running".to_string(),
            phase: "session_created".to_string(),
            process_id: std::process::id(),
            process_identity: Some(process_identity),
            vault_name: resolved.name.clone(),
            vault_id: resolved.vault_id.to_string(),
            vault_path: resolved.path.display().to_string(),
            batch_path: canonical_batch.display().to_string(),
            batch_sha256,
            batch_bytes,
            batch_line_count: validation.line_count,
            planned_row_count: validation.row_count,
            expected_vault_state: None,
            observed_vault_state_before_claim: None,
            vault_state_claim: None,
            rows_started: 0,
            rows_committed: 0,
            pending_rows: validation.row_count,
            uncommitted_started_rows: 0,
            committed_new_rows: 0,
            already_idempotent_rows: 0,
            failed_rows: 0,
            distinct_cx_count: 0,
            first_cx_id: None,
            last_cx_id: None,
            first_ledger_seq: None,
            last_ledger_seq: None,
            final_chain_seq: None,
            index_rebuild_phase: "not_started".to_string(),
            started_at_unix_ms: now,
            updated_at_unix_ms: now,
            completed_at_unix_ms: None,
            status_path: status_path.display().to_string(),
            error: None,
            terminal: None,
        };
        let session = Self {
            status,
            status_path,
        };
        session.write()?;
        Ok(session)
    }

    pub(super) fn session_id(&self) -> &str {
        &self.status.session_id
    }

    pub(super) fn status_path(&self) -> &Path {
        &self.status_path
    }

    pub(super) fn declare_precondition(&mut self, expected: &IngestPrecondition) -> CliResult<()> {
        if expected.is_empty() {
            return Err(session_invalid(
                "cannot declare an empty ingest vault-state precondition",
            ));
        }
        self.status.expected_vault_state = Some(expected.clone());
        self.status.phase = "vault_state_precondition_declared".to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn precondition_context(&self) -> IngestPreconditionContext {
        IngestPreconditionContext {
            session_id: self.status.session_id.clone(),
            batch_sha256: self.status.batch_sha256.clone(),
            planned_row_count: self.status.planned_row_count,
        }
    }

    pub(super) fn record_precondition_claim(
        &mut self,
        claim: IngestPreconditionClaim,
    ) -> CliResult<()> {
        if self.status.expected_vault_state.as_ref() != Some(&claim.expected) {
            return Err(session_invalid(format!(
                "ingest session {} claim differs from its declared vault-state precondition",
                self.status.session_id
            )));
        }
        self.status.observed_vault_state_before_claim = Some(claim.observed_before_claim.clone());
        self.status.vault_state_claim = Some(claim);
        self.status.phase = "vault_state_precondition_claimed".to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_precondition_verification(
        &mut self,
        observed: IngestVaultState,
    ) -> CliResult<()> {
        self.status.observed_vault_state_before_claim = Some(observed);
        self.status.phase = "vault_state_precondition_verified_noop".to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_phase(&mut self, phase: &str) -> CliResult<()> {
        self.check_interrupted()?;
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_rows_started(
        &mut self,
        rows_started: usize,
        phase: &str,
    ) -> CliResult<()> {
        self.check_interrupted()?;
        if rows_started > self.status.planned_row_count {
            return Err(session_invalid(format!(
                "ingest session {} cannot record {rows_started} started rows for a {}-row plan",
                self.status.session_id, self.status.planned_row_count
            )));
        }
        self.status.rows_started = self.status.rows_started.max(rows_started);
        self.refresh_row_accounting()?;
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_summary_progress(
        &mut self,
        summary: &BatchIngestSummary,
        phase: &str,
    ) -> CliResult<()> {
        self.check_interrupted()?;
        self.copy_summary(summary)?;
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_index_phase(&mut self, phase: &str) -> CliResult<()> {
        self.check_interrupted()?;
        self.status.index_rebuild_phase = phase.to_string();
        self.status.phase = format!("index_rebuild_{phase}");
        self.touch()?;
        self.write()
    }

    pub(super) fn complete(
        &mut self,
        summary: &BatchIngestSummary,
        final_chain_seq: u64,
    ) -> CliResult<()> {
        self.check_interrupted()?;
        self.copy_summary(summary)?;
        self.status.status = "complete".to_string();
        self.status.phase = "complete".to_string();
        self.status.final_chain_seq = Some(final_chain_seq);
        let now = now_ms();
        self.status.updated_at_unix_ms = now;
        self.status.completed_at_unix_ms = Some(now);
        self.status.error = None;
        self.status.terminal = None;
        validate_status(&self.status)?;
        self.write()
    }

    pub(super) fn fail_with_error(&mut self, error: &CliError) -> CliResult<()> {
        if error.code() == CALYX_INGEST_SESSION_INTERRUPTED && self.status.status == "interrupted" {
            return Ok(());
        }
        self.status.status = "failed".to_string();
        self.status.phase = "failed".to_string();
        self.refresh_row_accounting()?;
        let now = now_ms();
        self.status.updated_at_unix_ms = now;
        self.status.completed_at_unix_ms = Some(now);
        self.status.error = Some(IngestSessionError {
            code: error.code().to_string(),
            message: error.message().to_string(),
        });
        self.status.terminal = None;
        self.write()
    }

    pub(super) fn check_interrupted(&mut self) -> CliResult<()> {
        let Some(signal) = signals::pending() else {
            return Ok(());
        };
        let signal_name = signals::name(signal);
        let detail = format!(
            "batch ingest process {} received {signal_name} after durable phase {}; counters remain at planned={} started={} committed={} pending={} uncommitted_started={}",
            self.status.process_id,
            self.status.phase,
            self.status.planned_row_count,
            self.status.rows_started,
            self.status.rows_committed,
            self.status.pending_rows,
            self.status.uncommitted_started_rows
        );
        self.record_terminal(
            "interrupted",
            Some(signal_name.clone()),
            CALYX_INGEST_SESSION_INTERRUPTED,
            detail.clone(),
            "inspect Base CF, the ledger chain, rebuild-required marker, and search manifest independently; retry with a new session id only after the vault is healthy",
        )?;
        Err(session_error(
            CALYX_INGEST_SESSION_INTERRUPTED,
            detail,
            "read the terminal session JSON and independently verify Base CF, ledger, rebuild-required marker, and search manifest before retrying",
        )
        .into())
    }

    fn record_terminal(
        &mut self,
        kind: &str,
        signal: Option<String>,
        code: &'static str,
        detail: String,
        remediation: &'static str,
    ) -> CliResult<()> {
        let _guard = DurableWriteLockGuard::acquire_for_target(
            &self.status_path,
            "ingest session terminal transition",
        )?;
        let previous = read_status_bytes(&self.status_path)?;
        let physical = decode_status(&self.status_path, &previous)?;
        if physical != self.status {
            return Err(session_invalid(format!(
                "ingest session {} changed on disk before terminal transition; refusing to overwrite concurrent state",
                self.status.session_id
            )));
        }
        let previous_status_sha256 = archive_status(&self.status_path, &previous)?;
        let observed_at_unix_ms = now_ms();
        let observer = current_process_identity().map_err(|error| {
            CliError::from(session_error(
                CALYX_INGEST_SESSION_IDENTITY_FAILED,
                format!("capture terminal-transition observer identity: {error}"),
                "restore process identity inspection before changing ingest session state",
            ))
        })?;
        self.status.status = kind.to_string();
        self.status.updated_at_unix_ms = observed_at_unix_ms;
        self.status.completed_at_unix_ms = Some(observed_at_unix_ms);
        self.status.error = Some(IngestSessionError {
            code: code.to_string(),
            message: detail.clone(),
        });
        self.status.terminal = Some(IngestSessionTerminal {
            kind: kind.to_string(),
            signal,
            observed_at_unix_ms,
            last_durable_phase: self.status.phase.clone(),
            previous_status_sha256,
            observer,
            detail,
            remediation: remediation.to_string(),
        });
        validate_status(&self.status)?;
        write_status_file_locked(&self.status_path, &self.status)
    }

    fn copy_summary(&mut self, summary: &BatchIngestSummary) -> CliResult<()> {
        if summary.row_count > self.status.rows_started
            || summary.verified_base_rows > summary.row_count
        {
            return Err(session_invalid(format!(
                "ingest session {} received inconsistent summary counts: planned={} started={} processed={} verified={}",
                self.status.session_id,
                self.status.planned_row_count,
                self.status.rows_started,
                summary.row_count,
                summary.verified_base_rows
            )));
        }
        self.status.rows_committed = summary.verified_base_rows;
        // Runtime counts are populated only after the flush and its physical
        // Base CF readback succeed. The final physical reconciliation fields
        // remain zero until the entire input stream finishes, so copying them
        // at a per-window checkpoint made failed partial sessions under-report
        // rows already committed to Aster. Once reconciliation runs, retain the
        // independent physical readback as the authoritative final result.
        if summary.physical_reconciled {
            self.status.committed_new_rows = summary.new_count;
            self.status.already_idempotent_rows = summary.already_count;
            self.status.distinct_cx_count = summary.distinct_cx_count;
        } else {
            self.status.committed_new_rows = summary.runtime_new_count;
            self.status.already_idempotent_rows = summary.runtime_already_count;
            self.status.distinct_cx_count = summary.batch_cx_ids.len();
        }
        self.status.first_cx_id = summary.first_cx_id.clone();
        self.status.last_cx_id = summary.last_cx_id.clone();
        self.status.first_ledger_seq = summary.first_ledger_seq;
        self.status.last_ledger_seq = summary.last_ledger_seq;
        self.refresh_row_accounting()
    }

    fn refresh_row_accounting(&mut self) -> CliResult<()> {
        if self.status.rows_committed > self.status.rows_started
            || self.status.rows_started > self.status.planned_row_count
        {
            return Err(session_invalid(format!(
                "ingest session {} row accounting is inconsistent: planned={} started={} committed={}",
                self.status.session_id,
                self.status.planned_row_count,
                self.status.rows_started,
                self.status.rows_committed
            )));
        }
        self.status.pending_rows = self.status.planned_row_count - self.status.rows_started;
        self.status.uncommitted_started_rows =
            self.status.rows_started - self.status.rows_committed;
        // Batch ingest is fail-fast and never skips an individual row. A
        // terminal process error is recorded in `error`; it does not relabel
        // pending or uncommitted work as failed input.
        self.status.failed_rows = 0;
        Ok(())
    }

    fn touch(&mut self) -> CliResult<()> {
        self.status.updated_at_unix_ms = now_ms();
        Ok(())
    }

    fn write(&self) -> CliResult<()> {
        validate_status(&self.status)?;
        write_status_file(&self.status_path, &self.status)
    }
}

pub(super) fn run_status(args: IngestStatusArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let (status, owner_unknown) = reconcile_session_status(&resolved.path, &args.session_id)?;
    print_json(&status)?;
    match status.status.as_str() {
        "complete" => Ok(()),
        "failed" => Err(session_error(
            CALYX_INGEST_SESSION_FAILED,
            format!(
                "ingest session {} failed during {}",
                status.session_id, status.phase
            ),
            "read the status JSON error field and fix the named phase before retrying",
        )
        .into()),
        "interrupted" => Err(session_error(
            CALYX_INGEST_SESSION_INTERRUPTED,
            format!(
                "ingest session {} was interrupted by {} during its last durable phase {}",
                status.session_id,
                status
                    .terminal
                    .as_ref()
                    .and_then(|terminal| terminal.signal.as_deref())
                    .unwrap_or("an unrecorded signal"),
                status.phase
            ),
            "inspect the terminal record and independently verify Base CF, ledger, rebuild-required marker, and search manifest before retrying",
        )
        .into()),
        "abandoned" => Err(session_error(
            CALYX_INGEST_SESSION_ABANDONED,
            format!(
                "ingest session {} has no live exact owner after its last durable phase {}; committed work was not inferred",
                status.session_id, status.phase
            ),
            "inspect the archived prior status and independently verify Base CF, ledger, rebuild-required marker, and search manifest before retrying",
        )
        .into()),
        "running" if owner_unknown.is_some() => Err(session_error(
            CALYX_INGEST_SESSION_OWNER_UNKNOWN,
            format!(
                "ingest session {} ownership cannot be established safely: {}",
                status.session_id,
                owner_unknown.as_deref().expect("guarded above")
            ),
            "inspect the recorded host/process identity on the owning host; do not reconcile or trust completion while ownership is unknown",
        )
        .into()),
        _ => Err(session_error(
            CALYX_INGEST_SESSION_INCOMPLETE,
            format!(
                "ingest session {} is not complete: status={} phase={}",
                status.session_id, status.status, status.phase
            ),
            "wait for a terminal status or inspect the recorded process/session state before trusting ingest completion",
        )
        .into()),
    }
}

pub(super) fn reconcile_session_status(
    vault_path: &Path,
    session_id: &str,
) -> CliResult<(IngestSessionStatus, Option<String>)> {
    validate_session_id(session_id)?;
    let path = session_dir(vault_path, session_id).join(STATUS_FILE);
    ensure_status_exists(&path)?;
    let _guard = DurableWriteLockGuard::acquire_for_target(&path, "ingest session reconciliation")?;
    let previous = read_status_bytes(&path)?;
    let mut status = decode_status(&path, &previous)?;
    if status.status != "running" {
        return Ok((status, None));
    }

    let owner = match status.process_identity.as_ref() {
        Some(identity) => inspect_owner(identity),
        None => inspect_legacy_pid(status.process_id),
    };
    match owner {
        OwnerInspection::Alive => Ok((status, None)),
        OwnerInspection::Unknown(detail) => Ok((status, Some(detail))),
        OwnerInspection::Dead(detail) => {
            let previous_status_sha256 = archive_status(&path, &previous)?;
            let observed_at_unix_ms = now_ms();
            let observer = current_process_identity().map_err(|error| {
                CliError::from(session_error(
                    CALYX_INGEST_SESSION_IDENTITY_FAILED,
                    format!("capture abandoned-session observer identity: {error}"),
                    "restore process identity inspection before reconciling ingest session state",
                ))
            })?;
            let message = format!(
                "exact batch-ingest owner is gone after durable phase {}; planned={} started={} committed={} pending={} uncommitted_started={}; no commit success was inferred; owner inspection: {detail}",
                status.phase,
                status.planned_row_count,
                status.rows_started,
                status.rows_committed,
                status.pending_rows,
                status.uncommitted_started_rows
            );
            status.status = "abandoned".to_string();
            status.updated_at_unix_ms = observed_at_unix_ms;
            status.completed_at_unix_ms = Some(observed_at_unix_ms);
            status.error = Some(IngestSessionError {
                code: CALYX_INGEST_SESSION_ABANDONED.to_string(),
                message: message.clone(),
            });
            status.terminal = Some(IngestSessionTerminal {
                kind: "abandoned".to_string(),
                signal: None,
                observed_at_unix_ms,
                last_durable_phase: status.phase.clone(),
                previous_status_sha256,
                observer,
                detail: message,
                remediation: "independently inspect Base CF, ledger chain, rebuild-required marker, and search manifest; preserve this history and retry with a new session id only after vault health is proven".to_string(),
            });
            validate_status(&status)?;
            write_status_file_locked(&path, &status)?;
            Ok((status, None))
        }
    }
}

#[cfg(test)]
pub(super) fn read_session_status(
    vault_path: &Path,
    session_id: &str,
) -> CliResult<IngestSessionStatus> {
    validate_session_id(session_id)?;
    let path = session_dir(vault_path, session_id).join(STATUS_FILE);
    ensure_status_exists(&path)?;
    let bytes = read_status_bytes(&path)?;
    decode_status(&path, &bytes)
}

fn ensure_status_exists(path: &Path) -> CliResult<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(session_error(
        CALYX_INGEST_SESSION_NOT_FOUND,
        format!("ingest session status does not exist at {}", path.display()),
        "check the CALYX_INGEST_SESSION line from ingest stderr or pass the exact --session-id used by ingest",
    )
    .into())
}

fn read_status_bytes(path: &Path) -> CliResult<Vec<u8>> {
    fs::read(path).map_err(|error| {
        session_error(
            CALYX_INGEST_SESSION_READ_FAILED,
            format!("read ingest session status {}: {error}", path.display()),
            "inspect file permissions and vault ingest session directory health",
        )
        .into()
    })
}

fn decode_status(path: &Path, bytes: &[u8]) -> CliResult<IngestSessionStatus> {
    let mut status: IngestSessionStatus = serde_json::from_slice(bytes).map_err(|error| {
        CliError::from(session_error(
            CALYX_INGEST_SESSION_READ_FAILED,
            format!("parse ingest session status {}: {error}", path.display()),
            "repair or quarantine the corrupt ingest session status before trusting this run",
        ))
    })?;
    validate_status(&status)?;
    // Schema v1 predates these explicit accounting fields. Derive only those
    // absent values from its preserved planned/started/committed source
    // counters before a terminal reconciliation publishes the upgraded JSON.
    // The original bytes are archived first, so both states remain auditable.
    if status.schema_version == LEGACY_SESSION_SCHEMA_VERSION {
        status.pending_rows = status.planned_row_count - status.rows_started;
        status.uncommitted_started_rows = status.rows_started - status.rows_committed;
    }
    Ok(status)
}

fn validate_status(status: &IngestSessionStatus) -> CliResult<()> {
    if !matches!(
        status.schema_version,
        LEGACY_SESSION_SCHEMA_VERSION | COUNTER_SESSION_SCHEMA_VERSION | SESSION_SCHEMA_VERSION
    ) {
        return Err(session_invalid(format!(
            "ingest session {} has unsupported schema version {}",
            status.session_id, status.schema_version
        )));
    }
    if status.rows_committed > status.rows_started || status.rows_started > status.planned_row_count
    {
        return Err(session_invalid(format!(
            "ingest session {} violates row ordering: planned={} started={} committed={}",
            status.session_id, status.planned_row_count, status.rows_started, status.rows_committed
        )));
    }
    if matches!(
        status.schema_version,
        COUNTER_SESSION_SCHEMA_VERSION | SESSION_SCHEMA_VERSION
    ) {
        let expected_pending = status.planned_row_count - status.rows_started;
        let expected_uncommitted = status.rows_started - status.rows_committed;
        if status.pending_rows != expected_pending
            || status.uncommitted_started_rows != expected_uncommitted
            || status.failed_rows != 0
        {
            return Err(session_invalid(format!(
                "ingest session {} violates v2+ counter invariants: pending={} expected_pending={} uncommitted_started={} expected_uncommitted={} failed_rows={}",
                status.session_id,
                status.pending_rows,
                expected_pending,
                status.uncommitted_started_rows,
                expected_uncommitted,
                status.failed_rows
            )));
        }
    }
    if status.schema_version == SESSION_SCHEMA_VERSION {
        let identity = status.process_identity.as_ref().ok_or_else(|| {
            session_invalid(format!(
                "v3 ingest session {} lacks exact process identity",
                status.session_id
            ))
        })?;
        validate_process_identity(identity, "session owner")?;
        if identity.process_id != status.process_id {
            return Err(session_invalid(format!(
                "v3 ingest session {} process id {} differs from identity process id {}",
                status.session_id, status.process_id, identity.process_id
            )));
        }
    } else if status.process_identity.is_some() {
        return Err(session_invalid(format!(
            "legacy ingest session {} unexpectedly carries v3 process identity",
            status.session_id
        )));
    }
    match status.status.as_str() {
        "running" => {
            if status.completed_at_unix_ms.is_some()
                || status.error.is_some()
                || status.terminal.is_some()
            {
                return Err(session_invalid(format!(
                    "running ingest session {} has terminal completion/error state",
                    status.session_id
                )));
            }
        }
        "complete" => {
            if status.rows_committed != status.planned_row_count
                || status.completed_at_unix_ms.is_none()
                || status.error.is_some()
                || status.terminal.is_some()
            {
                return Err(session_invalid(format!(
                    "complete ingest session {} does not have all planned rows physically committed",
                    status.session_id
                )));
            }
        }
        "failed" => {
            if status.completed_at_unix_ms.is_none()
                || status.error.is_none()
                || status.terminal.is_some()
            {
                return Err(session_invalid(format!(
                    "failed ingest session {} lacks terminal time or structured error",
                    status.session_id
                )));
            }
        }
        "interrupted" | "abandoned" => {
            let terminal = status.terminal.as_ref().ok_or_else(|| {
                session_invalid(format!(
                    "terminal ingest session {} lacks an audit record",
                    status.session_id
                ))
            })?;
            if status.completed_at_unix_ms != Some(terminal.observed_at_unix_ms)
                || status.error.is_none()
                || terminal.kind != status.status
                || terminal.last_durable_phase != status.phase
                || !is_sha256(&terminal.previous_status_sha256)
                || terminal.detail.is_empty()
                || terminal.remediation.is_empty()
            {
                return Err(session_invalid(format!(
                    "terminal ingest session {} has inconsistent audit state",
                    status.session_id
                )));
            }
            validate_process_identity(&terminal.observer, "terminal observer")?;
            match status.status.as_str() {
                "interrupted" if terminal.signal.is_none() => {
                    return Err(session_invalid(format!(
                        "interrupted ingest session {} lacks a signal",
                        status.session_id
                    )));
                }
                "abandoned" if terminal.signal.is_some() => {
                    return Err(session_invalid(format!(
                        "abandoned ingest session {} unexpectedly records a signal",
                        status.session_id
                    )));
                }
                _ => {}
            }
        }
        other => {
            return Err(session_invalid(format!(
                "ingest session {} has invalid status {other:?}",
                status.session_id
            )));
        }
    }
    Ok(())
}

fn validate_process_identity(identity: &IngestProcessIdentity, label: &str) -> CliResult<()> {
    if identity.host_name.is_empty()
        || identity.process_id == 0
        || identity.process_start == 0
        || identity.process_start_kind.is_empty()
        || identity.executable.is_empty()
    {
        return Err(session_invalid(format!(
            "{label} has incomplete host/process-start/executable identity"
        )));
    }
    if identity
        .boot_id
        .as_ref()
        .is_some_and(|value| value.is_empty())
    {
        return Err(session_invalid(format!("{label} has an empty boot id")));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn validate_session_id(value: &str) -> CliResult<()> {
    let path_safe = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !path_safe {
        return Err(session_invalid(format!(
            "invalid ingest session id {value}; use only ASCII letters, digits, '.', '-', or '_'"
        )));
    }
    RedactionPolicy::check_public_identifier("session_id", value).map_err(|error| {
        session_invalid(format!(
            "invalid ingest session id for the durable Ledger claim: {}; use at most {} characters for a generic path-safe id",
            error.message,
            MAX_UNCLASSIFIED_TOKEN_LEN,
        ))
    })?;
    Ok(())
}

fn session_dir(vault_path: &Path, session_id: &str) -> PathBuf {
    vault_path.join(SESSION_ROOT).join(session_id)
}

fn generated_session_id() -> CliResult<String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            session_error(
                CALYX_INGEST_SESSION_CLOCK_FAILED,
                format!("system clock before UNIX epoch while creating ingest session: {error}"),
                "fix host clock monotonicity before running ingest",
            )
        })?
        .as_millis();
    Ok(format!("{millis}-{}", std::process::id()))
}

fn file_sha256(path: &Path) -> CliResult<String> {
    let mut file = File::open(path).map_err(|error| {
        CliError::io(format!("open batch {} for sha256: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).map_err(|error| {
            CliError::io(format!("read batch {} for sha256: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_status_file(path: &Path, status: &IngestSessionStatus) -> CliResult<()> {
    let bytes = encode_status(path, status)?;
    write_bytes_atomic(path, &bytes, "ingest session status").map_err(|error| {
        session_write_error(format!(
            "durably publish ingest session status {}: {error}",
            path.display()
        ))
    })?;
    verify_status_readback(path, status)
}

fn write_status_file_locked(path: &Path, status: &IngestSessionStatus) -> CliResult<()> {
    let bytes = encode_status(path, status)?;
    write_bytes_atomic_locked(path, &bytes, "ingest session status").map_err(|error| {
        session_write_error(format!(
            "durably publish locked ingest session status {}: {error}",
            path.display(),
        ))
    })?;
    verify_status_readback(path, status)
}

fn encode_status(path: &Path, status: &IngestSessionStatus) -> CliResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(status).map_err(|error| {
        session_write_error(format!(
            "encode ingest session status {}: {error}",
            path.display()
        ))
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn verify_status_readback(path: &Path, expected: &IngestSessionStatus) -> CliResult<()> {
    let physical = decode_status(path, &read_status_bytes(path)?)?;
    if physical != *expected {
        return Err(session_write_error(format!(
            "physical ingest session readback {} differs from the state just published",
            path.display()
        )));
    }
    Ok(())
}

fn archive_status(status_path: &Path, previous: &[u8]) -> CliResult<String> {
    let previous_status_sha256 = format!("{:x}", Sha256::digest(previous));
    let session_root = status_path
        .parent()
        .ok_or_else(|| session_invalid("missing session root for status archive"))?;
    let archive_path = session_root
        .join(HISTORY_DIR)
        .join(format!("status-{previous_status_sha256}.json"));
    if archive_path.exists() {
        let archived = fs::read(&archive_path).map_err(|error| {
            session_write_error(format!(
                "read existing ingest status archive {}: {error}",
                archive_path.display()
            ))
        })?;
        if archived != previous {
            return Err(session_write_error(format!(
                "ingest status archive {} has the expected hash name but different bytes",
                archive_path.display()
            )));
        }
        return Ok(previous_status_sha256);
    }
    write_bytes_atomic(&archive_path, previous, "ingest session status archive").map_err(
        |error| {
            session_write_error(format!(
                "durably archive prior ingest status {}: {error}",
                archive_path.display()
            ))
        },
    )?;
    let archived = fs::read(&archive_path).map_err(|error| {
        session_write_error(format!(
            "read back ingest status archive {}: {error}",
            archive_path.display()
        ))
    })?;
    if archived != previous {
        return Err(session_write_error(format!(
            "physical ingest status archive readback {} differs from the prior status bytes",
            archive_path.display()
        )));
    }
    Ok(previous_status_sha256)
}

fn session_invalid(message: impl Into<String>) -> CliError {
    session_error(
        CALYX_INGEST_SESSION_INVALID,
        message.into(),
        "use a path-safe session id and retry the ingest command",
    )
    .into()
}

fn session_write_error(message: impl Into<String>) -> CliError {
    session_error(
        CALYX_INGEST_SESSION_WRITE_FAILED,
        message.into(),
        "inspect the vault idx/ingest/runs directory and filesystem health before retrying",
    )
    .into()
}

fn session_error(code: &'static str, message: String, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message,
        remediation,
    }
}
