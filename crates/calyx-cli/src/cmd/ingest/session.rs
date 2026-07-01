use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::vault::{ResolvedVault, now_ms};
use super::batch::BatchValidation;
use super::store::resolve_cli_vault;
use super::types::BatchIngestSummary;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const SESSION_SCHEMA_VERSION: u16 = 1;
const SESSION_ROOT: &str = "idx/ingest/runs";
const STATUS_FILE: &str = "status.json";
const CALYX_INGEST_SESSION_CLOCK_FAILED: &str = "CALYX_INGEST_SESSION_CLOCK_FAILED";
const CALYX_INGEST_SESSION_EXISTS: &str = "CALYX_INGEST_SESSION_EXISTS";
const CALYX_INGEST_SESSION_INVALID: &str = "CALYX_INGEST_SESSION_INVALID";
const CALYX_INGEST_SESSION_NOT_FOUND: &str = "CALYX_INGEST_SESSION_NOT_FOUND";
const CALYX_INGEST_SESSION_WRITE_FAILED: &str = "CALYX_INGEST_SESSION_WRITE_FAILED";
const CALYX_INGEST_SESSION_READ_FAILED: &str = "CALYX_INGEST_SESSION_READ_FAILED";
const CALYX_INGEST_SESSION_INCOMPLETE: &str = "CALYX_INGEST_SESSION_INCOMPLETE";
const CALYX_INGEST_SESSION_FAILED: &str = "CALYX_INGEST_SESSION_FAILED";

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
pub(super) struct IngestSessionStatus {
    pub(super) schema_version: u16,
    pub(super) session_id: String,
    pub(super) status: String,
    pub(super) phase: String,
    pub(super) process_id: u32,
    pub(super) vault_name: String,
    pub(super) vault_id: String,
    pub(super) vault_path: String,
    pub(super) batch_path: String,
    pub(super) batch_sha256: String,
    pub(super) batch_bytes: u64,
    pub(super) batch_line_count: usize,
    pub(super) planned_row_count: usize,
    pub(super) rows_started: usize,
    pub(super) rows_committed: usize,
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
        requested_session_id: Option<&str>,
    ) -> CliResult<Self> {
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
        fs::create_dir_all(parent).map_err(|error| {
            session_write_error(format!(
                "create ingest session parent {}: {error}",
                parent.display()
            ))
        })?;
        match fs::create_dir(&root) {
            Ok(()) => {}
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
        let batch_sha256 = file_sha256(&canonical_batch)?;
        let status_path = root.join(STATUS_FILE);
        let now = now_ms();
        let status = IngestSessionStatus {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id,
            status: "running".to_string(),
            phase: "session_created".to_string(),
            process_id: std::process::id(),
            vault_name: resolved.name.clone(),
            vault_id: resolved.vault_id.to_string(),
            vault_path: resolved.path.display().to_string(),
            batch_path: canonical_batch.display().to_string(),
            batch_sha256,
            batch_bytes,
            batch_line_count: validation.line_count,
            planned_row_count: validation.row_count,
            rows_started: 0,
            rows_committed: 0,
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

    pub(super) fn record_phase(&mut self, phase: &str) -> CliResult<()> {
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_rows_started(
        &mut self,
        rows_started: usize,
        phase: &str,
    ) -> CliResult<()> {
        self.status.rows_started = self.status.rows_started.max(rows_started);
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_summary_progress(
        &mut self,
        summary: &BatchIngestSummary,
        phase: &str,
    ) -> CliResult<()> {
        self.copy_summary(summary);
        self.status.phase = phase.to_string();
        self.touch()?;
        self.write()
    }

    pub(super) fn record_index_phase(&mut self, phase: &str) -> CliResult<()> {
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
        self.copy_summary(summary);
        self.status.status = "complete".to_string();
        self.status.phase = "complete".to_string();
        self.status.final_chain_seq = Some(final_chain_seq);
        let now = now_ms();
        self.status.updated_at_unix_ms = now;
        self.status.completed_at_unix_ms = Some(now);
        self.status.error = None;
        self.write()
    }

    pub(super) fn fail_with_error(&mut self, error: &CliError) -> CliResult<()> {
        self.status.status = "failed".to_string();
        self.status.phase = "failed".to_string();
        self.status.failed_rows = self
            .status
            .planned_row_count
            .saturating_sub(self.status.rows_committed);
        let now = now_ms();
        self.status.updated_at_unix_ms = now;
        self.status.completed_at_unix_ms = Some(now);
        self.status.error = Some(IngestSessionError {
            code: error.code().to_string(),
            message: error.message().to_string(),
        });
        self.write()
    }

    fn copy_summary(&mut self, summary: &BatchIngestSummary) {
        self.status.rows_committed = summary.verified_base_rows;
        self.status.committed_new_rows = summary.new_count;
        self.status.already_idempotent_rows = summary.already_count;
        self.status.distinct_cx_count = summary.distinct_cx_count;
        self.status.first_cx_id = summary.first_cx_id.clone();
        self.status.last_cx_id = summary.last_cx_id.clone();
        self.status.first_ledger_seq = summary.first_ledger_seq;
        self.status.last_ledger_seq = summary.last_ledger_seq;
        self.status.failed_rows = self
            .status
            .planned_row_count
            .saturating_sub(summary.row_count);
    }

    fn touch(&mut self) -> CliResult<()> {
        self.status.updated_at_unix_ms = now_ms();
        Ok(())
    }

    fn write(&self) -> CliResult<()> {
        write_status_file(&self.status_path, &self.status)
    }
}

pub(super) fn run_status(args: IngestStatusArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let status = read_session_status(&resolved.path, &args.session_id)?;
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

pub(super) fn read_session_status(
    vault_path: &Path,
    session_id: &str,
) -> CliResult<IngestSessionStatus> {
    validate_session_id(session_id)?;
    let path = session_dir(vault_path, session_id).join(STATUS_FILE);
    if !path.is_file() {
        return Err(session_error(
            CALYX_INGEST_SESSION_NOT_FOUND,
            format!("ingest session status does not exist at {}", path.display()),
            "check the CALYX_INGEST_SESSION line from ingest stderr or pass the exact --session-id used by ingest",
        )
        .into());
    }
    let bytes = fs::read(&path).map_err(|error| {
        session_error(
            CALYX_INGEST_SESSION_READ_FAILED,
            format!("read ingest session status {}: {error}", path.display()),
            "inspect file permissions and vault ingest session directory health",
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        session_error(
            CALYX_INGEST_SESSION_READ_FAILED,
            format!("parse ingest session status {}: {error}", path.display()),
            "repair or quarantine the corrupt ingest session status before trusting this run",
        )
        .into()
    })
}

pub(super) fn validate_session_id(value: &str) -> CliResult<()> {
    let valid = !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(session_invalid(format!(
            "invalid ingest session id {value}; use only ASCII letters, digits, '.', '-', or '_'"
        )))
    }
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
    let parent = path
        .parent()
        .ok_or_else(|| session_invalid("missing status parent"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            session_error(
                CALYX_INGEST_SESSION_CLOCK_FAILED,
                format!("system clock before UNIX epoch while writing ingest session: {error}"),
                "fix host clock monotonicity before running ingest",
            )
        })?
        .as_nanos();
    let tmp = parent.join(format!(".status.{}.{}.tmp", std::process::id(), nonce));
    let bytes = serde_json::to_vec_pretty(status).map_err(|error| {
        session_write_error(format!(
            "encode ingest session status {}: {error}",
            path.display()
        ))
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|error| {
            session_write_error(format!(
                "create ingest session temp {}: {error}",
                tmp.display()
            ))
        })?;
    file.write_all(&bytes).map_err(|error| {
        session_write_error(format!(
            "write ingest session temp {}: {error}",
            tmp.display()
        ))
    })?;
    file.write_all(b"\n").map_err(|error| {
        session_write_error(format!(
            "write ingest session temp newline {}: {error}",
            tmp.display()
        ))
    })?;
    file.sync_all().map_err(|error| {
        session_write_error(format!(
            "sync ingest session temp {}: {error}",
            tmp.display()
        ))
    })?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        session_write_error(format!(
            "commit ingest session status {} from temp {}: {error}",
            path.display(),
            tmp.display()
        ))
    })?;
    Ok(())
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
