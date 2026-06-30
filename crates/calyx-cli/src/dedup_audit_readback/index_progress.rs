use calyx_aster::base_page_index::{BasePageIndexBuildProgress, build_base_page_index};
use calyx_core::CalyxError;
use serde_json::json;

use super::{CliError, CliResult, CxListArgs, Deadline, ProgressSink, check_deadline};

pub(super) fn rebuild_cx_list_base_page_index(
    args: &CxListArgs,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> CliResult {
    check_deadline(deadline, progress, "base_page_index_build", 0)?;
    progress.emit(json!({
        "event": "cx_list.progress",
        "phase": "base_page_index_build_start",
        "page_size": args.base_page_index_page_size,
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    let manifest = build_base_page_index(&args.vault, args.base_page_index_page_size, |event| {
        emit_base_page_index_build_progress(event, deadline, progress)
    })?;
    progress.emit(json!({
        "event": "cx_list.progress",
        "phase": "base_page_index_build_complete",
        "total_entries": manifest.total_entries,
        "live_entries": manifest.live_entries,
        "tombstone_entries": manifest.tombstone_entries,
        "pages": manifest.pages.len(),
        "ledger_head_height": manifest.ledger_head_height,
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    check_deadline(
        deadline,
        progress,
        "base_page_index_build_complete",
        manifest.total_entries as u64,
    )
}

fn emit_base_page_index_build_progress(
    event: BasePageIndexBuildProgress,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> calyx_core::Result<()> {
    let processed = match event {
        BasePageIndexBuildProgress::ScanStarted {
            sst_files,
            ledger_head_height,
        } => {
            progress
                .emit(json!({
                    "event": "cx_list.progress",
                    "phase": "base_page_index_scan_start",
                    "sst_files": sst_files,
                    "ledger_head_height": ledger_head_height,
                    "elapsed_ms": deadline.elapsed_ms(),
                }))
                .map_err(cli_error_to_calyx)?;
            0
        }
        BasePageIndexBuildProgress::SstScanned {
            scanned_sst_files,
            total_sst_files,
            current_rows,
        } => {
            progress
                .emit(json!({
                    "event": "cx_list.progress",
                    "phase": "base_page_index_sst_scanned",
                    "scanned_sst_files": scanned_sst_files,
                    "total_sst_files": total_sst_files,
                    "current_rows": current_rows,
                    "elapsed_ms": deadline.elapsed_ms(),
                }))
                .map_err(cli_error_to_calyx)?;
            scanned_sst_files as u64
        }
        BasePageIndexBuildProgress::WalScanned {
            wal_records,
            current_rows,
        } => {
            progress
                .emit(json!({
                    "event": "cx_list.progress",
                    "phase": "base_page_index_wal_scanned",
                    "wal_records": wal_records,
                    "current_rows": current_rows,
                    "elapsed_ms": deadline.elapsed_ms(),
                }))
                .map_err(cli_error_to_calyx)?;
            current_rows as u64
        }
        BasePageIndexBuildProgress::PageWritten {
            page_index,
            entry_count,
            live_entry_count,
        } => {
            progress
                .emit(json!({
                    "event": "cx_list.progress",
                    "phase": "base_page_index_page_written",
                    "page_index": page_index,
                    "entry_count": entry_count,
                    "live_entry_count": live_entry_count,
                    "elapsed_ms": deadline.elapsed_ms(),
                }))
                .map_err(cli_error_to_calyx)?;
            entry_count as u64
        }
        BasePageIndexBuildProgress::Complete {
            total_entries,
            live_entries,
            pages,
        } => {
            progress
                .emit(json!({
                    "event": "cx_list.progress",
                    "phase": "base_page_index_complete",
                    "total_entries": total_entries,
                    "live_entries": live_entries,
                    "pages": pages,
                    "elapsed_ms": deadline.elapsed_ms(),
                }))
                .map_err(cli_error_to_calyx)?;
            total_entries as u64
        }
    };
    deadline
        .check("readback cx-list", "base_page_index_build", processed)
        .map_err(cli_error_to_calyx)
}

fn cli_error_to_calyx(error: CliError) -> CalyxError {
    match error {
        CliError::Calyx(error) => error,
        other => CalyxError {
            code: other.code(),
            message: other.message().to_string(),
            remediation: other.remediation(),
        },
    }
}
