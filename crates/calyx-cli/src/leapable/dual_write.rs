use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, VaultStore};
use serde::{Deserialize, Serialize};

use super::ShadowVault;
use super::shadow_harness::{
    ShadowManifestReadback, read_shadow_manifest, update_shadow_chunk_count,
};
use crate::error::{CliError, CliResult};
use crate::migrate;
use crate::migrate::adapter::{default_base_lens_id, default_panel_version};
use crate::migrate::manifest::{MigrationManifest, hex_encode};
use crate::migrate::reader::{ChunkRow, open_sqlite, row_count, stream_rows};
use crate::migrate::verifier::{
    StatusReport, VerifyReport, row_exists_and_matches, status, verify_migration,
};
use crate::output::print_json;

pub(crate) const CALYX_SHADOW_WRITE_FAILED: &str = "CALYX_SHADOW_WRITE_FAILED";
pub(crate) const DUAL_WRITE_RECEIPTS: &str = "DUAL_WRITE_RECEIPTS.jsonl";
pub(crate) const ASTER_DIR: &str = "aster";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct IngestReceipt {
    pub(crate) chunk_id: String,
    pub(crate) database_name: String,
    pub(crate) sqlite_rowid: u64,
    pub(crate) cx_id: String,
    pub(crate) text_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FailedIngest {
    pub(crate) chunk_id: String,
    pub(crate) database_name: String,
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct DualWriteReport {
    pub(crate) sqlite_path: PathBuf,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) aster_dir: PathBuf,
    pub(crate) shadow_manifest: ShadowManifestReadback,
    pub(crate) source_rows: usize,
    pub(crate) written_rows: usize,
    pub(crate) skipped_rows: usize,
    pub(crate) failures: Vec<FailedIngest>,
    pub(crate) receipts: Vec<IngestReceipt>,
    pub(crate) verify: Option<VerifyReport>,
    pub(crate) status: Option<StatusReport>,
    pub(crate) receipts_log: PathBuf,
    pub(crate) gate: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ReadbackVerifyReport {
    pub(crate) sqlite_path: PathBuf,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) aster_dir: PathBuf,
    pub(crate) shadow_manifest: ShadowManifestReadback,
    pub(crate) source_rows: usize,
    pub(crate) verify: VerifyReport,
    pub(crate) status: StatusReport,
    pub(crate) gate: String,
}

pub(crate) fn run_dual_write(args: &[String]) -> CliResult {
    let args = parse_dual_write_args(args)?;
    let report = if args.inject {
        replay_existing_sqlite_with_options(&args.sqlite, &args.calyx, true)?
    } else {
        replay_existing_sqlite(&args.sqlite, &args.calyx)?
    };
    let failed = !report.failures.is_empty() || report.gate != "PASS";
    print_json(&report)?;
    if failed {
        return Err(shadow_write_failed("dual-write replay did not pass the shadow gate").into());
    }
    Ok(())
}

pub(crate) fn run_readback_verify(vault: &Path, sqlite: &Path) -> CliResult {
    let report = verify_against(sqlite, vault)?;
    print_json(&report)?;
    if report.gate == "PASS" {
        Ok(())
    } else {
        Err(shadow_write_failed("readback verify-against gate failed").into())
    }
}

pub(crate) fn replay_existing_sqlite(
    sqlite: &Path,
    calyx_dir: &Path,
) -> CliResult<DualWriteReport> {
    replay_existing_sqlite_with_options(sqlite, calyx_dir, false)
}

pub(crate) fn verify_against(sqlite: &Path, calyx_dir: &Path) -> CliResult<ReadbackVerifyReport> {
    let shadow_manifest = read_shadow_manifest(calyx_dir)?;
    let conn = open_sqlite(sqlite)?;
    let rows = stream_rows(&conn)?;
    let aster = aster_dir(calyx_dir);
    let manifest = MigrationManifest::load(&aster)?;
    let vault = migrate::open_vault(&aster, &manifest)?;
    let adapter = migrate::adapter(&manifest)?;
    let verify = verify_migration(&vault, &rows, &adapter, false)?;
    let status = status(&vault, &aster)?;
    Ok(ReadbackVerifyReport {
        sqlite_path: sqlite.to_path_buf(),
        calyx_dir: calyx_dir.to_path_buf(),
        aster_dir: aster,
        shadow_manifest,
        source_rows: rows.len(),
        gate: verify.gate.clone(),
        verify,
        status,
    })
}

fn replay_existing_sqlite_with_options(
    sqlite: &Path,
    calyx_dir: &Path,
    inject_failure: bool,
) -> CliResult<DualWriteReport> {
    let shadow = ShadowVault::open(sqlite, calyx_dir)?;
    shadow.close()?;
    let conn = open_sqlite(sqlite)?;
    let source_rows = usize::try_from(row_count(&conn)?)
        .map_err(|_| CliError::usage("sqlite row count exceeds usize"))?;
    let rows = stream_rows(&conn)?;
    let aster = aster_dir(calyx_dir);
    let mut manifest = MigrationManifest::load_or_create(
        &aster,
        sqlite,
        &rows,
        default_base_lens_id(),
        default_panel_version(),
    )?;
    manifest.write(&aster)?;
    let vault = migrate::open_vault(&aster, &manifest)?;
    let adapter = migrate::adapter(&manifest)?;
    migrate::ensure_unique_cx_ids(&adapter, &rows)?;
    let mut receipts = Vec::new();
    let mut failures = Vec::new();
    let mut written_rows = 0;
    let mut skipped_rows = 0;
    for row in &rows {
        if inject_failure {
            failures.push(failed(
                row,
                shadow_write_failed("injected shadow write failure"),
            ));
            continue;
        }
        match row_exists_and_matches(&vault, row, &adapter) {
            Ok(true) => {
                skipped_rows += 1;
                receipts.push(receipt_for(&adapter, row, hex_encode(&row.content_hash())));
            }
            Ok(false) => match adapter
                .constellation(row)
                .and_then(|cx| vault.put(cx).map_err(Into::into))
            {
                Ok(_) => {
                    written_rows += 1;
                    receipts.push(receipt_for(&adapter, row, hex_encode(&row.content_hash())));
                }
                Err(error) => failures.push(failed(row, shadow_write_failed(error.message()))),
            },
            Err(error) => failures.push(failed(row, shadow_write_failed(error.message))),
        }
    }
    vault.flush()?;
    manifest.source_rows = source_rows;
    manifest.migrated_rows = written_rows + skipped_rows;
    manifest.write(&aster)?;
    let receipts_log = write_receipts(calyx_dir, &receipts)?;
    let (verify, current_status, gate) = if failures.is_empty() {
        let verify = verify_migration(&vault, &rows, &adapter, false)?;
        let current_status = status(&vault, &aster)?;
        let gate = verify.gate.clone();
        (Some(verify), current_status, gate)
    } else {
        (None, status(&vault, &aster)?, "FAIL".to_string())
    };
    let shadow_manifest = update_shadow_chunk_count(calyx_dir, current_status.base_rows as u64)?;
    Ok(DualWriteReport {
        sqlite_path: sqlite.to_path_buf(),
        calyx_dir: calyx_dir.to_path_buf(),
        aster_dir: aster,
        shadow_manifest,
        source_rows,
        written_rows,
        skipped_rows,
        failures,
        receipts,
        verify,
        status: Some(current_status),
        receipts_log,
        gate,
    })
}

pub(crate) fn aster_dir(calyx_dir: &Path) -> PathBuf {
    calyx_dir.join(ASTER_DIR)
}

fn receipt_for(
    adapter: &crate::migrate::adapter::VaultSqliteAdapter,
    row: &ChunkRow,
    text_hash: String,
) -> IngestReceipt {
    IngestReceipt {
        chunk_id: row.chunk_id.clone(),
        database_name: row.database_name.clone(),
        sqlite_rowid: row.row_num,
        cx_id: adapter.cx_id(row).to_string(),
        text_hash,
    }
}

fn write_receipts(calyx_dir: &Path, receipts: &[IngestReceipt]) -> CliResult<PathBuf> {
    fs::create_dir_all(calyx_dir)
        .map_err(|err| CliError::io(format!("create {}: {err}", calyx_dir.display())))?;
    let path = calyx_dir.join(DUAL_WRITE_RECEIPTS);
    let mut file = File::create(&path)
        .map_err(|err| CliError::io(format!("create {}: {err}", path.display())))?;
    for receipt in receipts {
        serde_json::to_writer(&mut file, receipt)
            .map_err(|err| CliError::io(format!("write receipt: {err}")))?;
        file.write_all(b"\n")
            .map_err(|err| CliError::io(format!("write receipt newline: {err}")))?;
    }
    file.sync_all()
        .map_err(|err| CliError::io(format!("sync {}: {err}", path.display())))?;
    Ok(path)
}

fn failed(row: &ChunkRow, error: CalyxError) -> FailedIngest {
    FailedIngest {
        chunk_id: row.chunk_id.clone(),
        database_name: row.database_name.clone(),
        code: error.code.to_string(),
        message: error.message,
    }
}

pub(crate) fn shadow_write_failed(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_SHADOW_WRITE_FAILED,
        message,
        "keep sqlite readable, inspect the Calyx shadow vault WAL, then retry dual-write",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DualWriteArgs {
    sqlite: PathBuf,
    calyx: PathBuf,
    inject: bool,
}

fn parse_dual_write_args(args: &[String]) -> CliResult<DualWriteArgs> {
    match args {
        [sqlite_flag, sqlite, calyx_flag, calyx]
            if sqlite_flag == "--sqlite" && calyx_flag == "--calyx" =>
        {
            Ok(DualWriteArgs {
                sqlite: PathBuf::from(sqlite),
                calyx: PathBuf::from(calyx),
                inject: false,
            })
        }
        [sqlite_flag, sqlite, calyx_flag, calyx, inject]
            if sqlite_flag == "--sqlite"
                && calyx_flag == "--calyx"
                && inject == "--inject-shadow-failure" =>
        {
            Ok(DualWriteArgs {
                sqlite: PathBuf::from(sqlite),
                calyx: PathBuf::from(calyx),
                inject: true,
            })
        }
        _ => Err(CliError::usage(
            "usage: calyx leapable dual-write --sqlite <db> --calyx <dir> [--inject-shadow-failure]",
        )),
    }
}
