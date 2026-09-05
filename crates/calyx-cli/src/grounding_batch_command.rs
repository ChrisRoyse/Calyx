mod input;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{
    AsterVault, GroundingBatchDisposition, GroundingBatchReport, VaultOptions,
};
use calyx_core::CalyxError;
use serde_json::json;

use crate::error::{CliError, CliResult};

use input::{encode_hex, hex_sha256, read_plan};

struct Args {
    vault: String,
    input: PathBuf,
    verify_only: bool,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    if args.first().map(String::as_str) != Some("grounding-batch") {
        return None;
    }
    Some(parse_args(&args[1..]).and_then(run))
}

fn run(args: Args) -> CliResult {
    let (events, input_sha256, input_bytes) =
        read_plan(&args.input).map_err(grounding_input_error)?;
    let resolved = resolve_vault(&args.vault)?;
    let wal_before = wal_state(&resolved.path)?;
    let before = open_vault(&resolved)?;
    let before_seq = before.latest_seq();
    let report = before.ground_existing_batch(events, args.verify_only)?;
    let after_seq = before.latest_seq();
    drop(before);

    let (readback_events, readback_input_sha256, readback_input_bytes) =
        read_plan(&args.input).map_err(grounding_input_error)?;
    if readback_input_sha256 != input_sha256 || readback_input_bytes != input_bytes {
        return Err(CliError::runtime(format!(
            "plan identity changed before independent vault readback: initial sha256={input_sha256} bytes={input_bytes}, readback sha256={readback_input_sha256} bytes={readback_input_bytes}"
        )));
    }
    let readback_vault = open_vault(&resolved)?;
    let readback = readback_vault
        .ground_existing_batch(readback_events, true)
        .map_err(|error| {
            CliError::Calyx(calyx_core::CalyxError {
                code: error.code,
                message: format!("independent readback: {}", error.message),
                remediation: error.remediation,
            })
        })?;
    let readback_seq = readback_vault.latest_seq();
    if readback.committed != 0
        || readback.existing_exact != report.events.len()
        || readback.physical_anchor_rows != report.physical_anchor_rows
        || readback_seq != after_seq
    {
        return Err(CliError::runtime(format!(
            "post-commit readback drift: applied committed={} existing={} rows={} seq={} readback committed={} existing={} rows={} seq={}",
            report.committed,
            report.existing_exact,
            report.physical_anchor_rows,
            after_seq,
            readback.committed,
            readback.existing_exact,
            readback.physical_anchor_rows,
            readback_seq
        )));
    }
    drop(readback_vault);
    let wal_after = wal_state(&resolved.path)?;
    if report.committed == 0 && after_seq != before_seq {
        return Err(CliError::runtime(format!(
            "zero-write replay advanced vault sequence from {before_seq} to {after_seq}"
        )));
    }
    if report.committed == 0 && wal_after != wal_before {
        return Err(CliError::runtime(format!(
            "zero-write replay changed WAL state from files={} bytes={} to files={} bytes={}",
            wal_before.files, wal_before.bytes, wal_after.files, wal_after.bytes
        )));
    }
    if report.committed != 0 && wal_after.bytes <= wal_before.bytes {
        return Err(CliError::runtime(format!(
            "committed grounding events without positive WAL growth: before files={} bytes={} after files={} bytes={}",
            wal_before.files, wal_before.bytes, wal_after.files, wal_after.bytes
        )));
    }
    print_report(
        &args,
        &resolved.path,
        &input_sha256,
        input_bytes,
        before_seq,
        after_seq,
        wal_before,
        wal_after,
        &report,
    )
    .map_err(CliError::runtime)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WalState {
    files: u64,
    bytes: u64,
}

fn wal_state(vault: &Path) -> CliResult<WalState> {
    let wal = vault.join("wal");
    let mut state = WalState { files: 0, bytes: 0 };
    for entry in fs::read_dir(&wal).map_err(|error| {
        CliError::runtime(format!("read WAL directory {}: {error}", wal.display()))
    })? {
        let entry = entry.map_err(|error| {
            CliError::runtime(format!(
                "read WAL directory entry {}: {error}",
                wal.display()
            ))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("wal") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            CliError::runtime(format!("stat WAL segment {}: {error}", path.display()))
        })?;
        if !metadata.file_type().is_file() {
            return Err(CliError::runtime(format!(
                "WAL segment candidate is not a regular file: {}",
                path.display()
            )));
        }
        state.files = state
            .files
            .checked_add(1)
            .ok_or_else(|| CliError::runtime("WAL file count overflow"))?;
        state.bytes = state
            .bytes
            .checked_add(metadata.len())
            .ok_or_else(|| CliError::runtime("WAL byte count overflow"))?;
    }
    Ok(state)
}

fn grounding_input_error(message: String) -> CliError {
    CliError::Calyx(CalyxError {
        code: "CALYX_GROUNDING_INPUT_INVALID",
        message,
        remediation: "fix the exact grounding plan identity or schema error; parsing completed before any vault write",
    })
}

fn parse_args(raw: &[String]) -> CliResult<Args> {
    let vault = raw.first().ok_or_else(|| CliError::usage(usage()))?.clone();
    let mut input = None;
    let mut verify_only = false;
    let mut index = 1usize;
    while index < raw.len() {
        let arg = &raw[index];
        match arg.as_str() {
            "--input" => {
                index += 1;
                input = Some(PathBuf::from(
                    raw.get(index).ok_or_else(|| CliError::usage(usage()))?,
                ));
            }
            "--verify-only" => verify_only = true,
            _ => {
                return Err(CliError::usage(format!(
                    "unknown argument {arg:?}; {}",
                    usage()
                )));
            }
        }
        index += 1;
    }
    Ok(Args {
        vault,
        input: input.ok_or_else(|| CliError::usage(usage()))?,
        verify_only,
    })
}

fn usage() -> String {
    "usage: calyx grounding-batch <vault> --input <canonical.json> [--verify-only]".to_string()
}

struct Resolved {
    path: PathBuf,
    name: String,
    vault_id: calyx_core::VaultId,
}

fn resolve_vault(reference: &str) -> CliResult<Resolved> {
    let home = crate::cmd::vault::home_dir()?;
    let resolved = crate::cmd::vault::resolve_vault_info(&home, reference)?;
    Ok(Resolved {
        path: resolved.path,
        name: resolved.name,
        vault_id: resolved.vault_id,
    })
}

fn open_vault(resolved: &Resolved) -> CliResult<AsterVault> {
    AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        crate::cmd::vault::vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )
    .map_err(CliError::from)
}

fn print_report(
    args: &Args,
    vault: &Path,
    input_sha256: &str,
    input_bytes: u64,
    before_seq: u64,
    after_seq: u64,
    wal_before: WalState,
    wal_after: WalState,
    report: &GroundingBatchReport,
) -> Result<(), String> {
    let event_rows = report
        .events
        .iter()
        .map(|event| {
            json!({
                "event_id": encode_hex(&event.event_id),
                "disposition": match event.disposition {
                    GroundingBatchDisposition::Committed => "committed",
                    GroundingBatchDisposition::ExistingExact => "existing_exact",
                },
                "ledger_seq": event.ledger_ref.seq,
                "ledger_hash": encode_hex(&event.ledger_ref.hash),
                "targets": event.targets,
                "anchors_per_target": event.anchors_per_target,
            })
        })
        .collect::<Vec<_>>();
    let events_sha256 = hex_sha256(
        &serde_json::to_vec(&event_rows)
            .map_err(|error| format!("serialize report event rows: {error}"))?,
    );
    println!(
        "{}",
        json!({
            "event": "grounding_batch.readback_passed",
            "mode": if args.verify_only { "verify_only" } else { "apply" },
            "vault": vault,
            "input": args.input,
            "input_bytes": input_bytes,
            "input_sha256": input_sha256,
            "before_seq": before_seq,
            "after_seq": after_seq,
            "wal_files_before": wal_before.files,
            "wal_files_after": wal_after.files,
            "wal_bytes_before": wal_before.bytes,
            "wal_bytes_after": wal_after.bytes,
            "wal_bytes_delta": i128::from(wal_after.bytes) - i128::from(wal_before.bytes),
            "events": report.events.len(),
            "committed": report.committed,
            "existing_exact": report.existing_exact,
            "physical_anchor_rows": report.physical_anchor_rows,
            "checkpoint_event_interval": report.checkpoint_event_interval,
            "checkpoint_flushes": report.checkpoint_flushes,
            "mvcc_versions_reclaimed": report.mvcc_versions_reclaimed,
            "mvcc_value_bytes_reclaimed": report.mvcc_value_bytes_reclaimed,
            "events_sha256": events_sha256,
            "readback": "separately_reopened_base_anchors_ledger_exact",
        })
    );
    Ok(())
}
