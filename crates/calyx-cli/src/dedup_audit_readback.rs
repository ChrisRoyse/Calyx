use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::base_page_index::{DEFAULT_BASE_PAGE_INDEX_PAGE_SIZE, read_indexed_base_rows};
use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::CxId;
use serde_json::json;

use crate::bounded_progress::{Deadline, ProgressSink, parse_nonzero_u64, parse_nonzero_usize};
use crate::cf_read::{hex_bytes, latest_cf_row, latest_cf_rows, vault_id_from_base};
use crate::error::{CliError, CliResult};
use crate::output::print_line;

const CX_LIST_UNBOUNDED_ROW_LIMIT: usize = 100;

mod index_progress;
mod physical_slots;
#[cfg(test)]
mod tests;

use index_progress::rebuild_cx_list_base_page_index;
use physical_slots::{physical_slot_states, slot_row_json, slot_summary, tombstone_row};

#[derive(Debug)]
struct CxListArgs {
    vault: PathBuf,
    cx_id: Option<CxId>,
    limit: Option<usize>,
    include_slots: bool,
    allow_unbounded: bool,
    progress_jsonl: Option<String>,
    time_budget_ms: Option<u64>,
    rebuild_base_page_index: bool,
    base_page_index_page_size: usize,
}

pub fn readback_dedup_audit(vault: &Path, cx_id: &str) -> crate::error::CliResult {
    let cx_id = CxId::from_str(cx_id)
        .map_err(|error| CliError::usage(format!("invalid --cx-id: {error}")))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )?;
    let report = dedup_audit(&store, cx_id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::runtime(format!("serialize dedup audit report: {error}")))?
    );
    Ok(())
}

pub fn readback_dedup_undo(vault: &Path, token: &str) -> crate::error::CliResult {
    let token: ReversalToken = serde_json::from_str(token)
        .map_err(|error| CliError::usage(format!("invalid --token: {error}")))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )?;
    let before = latest_cf_rows(vault, ColumnFamily::Base)?;
    let restored = dedup_undo(&store, &token)?;
    store.flush()?;
    let after = latest_cf_rows(vault, ColumnFamily::Base)?;
    let value = json!({
        "vault": vault.display().to_string(),
        "restored": restored,
        "base_rows_before": before.len(),
        "base_rows_after": after.len(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value)
            .map_err(|error| CliError::runtime(format!("serialize dedup undo report: {error}")))?
    );
    Ok(())
}

pub fn readback_cx_list_args(rest: &[String]) -> CliResult {
    let args = parse_cx_list_args(rest)?;
    let mut progress = ProgressSink::from_arg(args.progress_jsonl.as_deref())?;
    let deadline = Deadline::new(args.time_budget_ms);
    progress.emit(json!({
        "event": "cx_list.progress",
        "phase": "start",
        "vault": args.vault.display().to_string(),
        "limit": args.limit,
        "include_slots": args.include_slots,
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    if args.rebuild_base_page_index {
        rebuild_cx_list_base_page_index(&args, &deadline, &mut progress)?;
    }
    if let Some(cx_id) = args.cx_id {
        let key = base_key(cx_id);
        let value = latest_cf_row(&args.vault, ColumnFamily::Base, &key)?.ok_or_else(|| {
            CliError::usage(format!(
                "cx-list --cx-id {cx_id} was not found in {}",
                args.vault.display()
            ))
        })?;
        let rows = BTreeMap::from([(key, value)]);
        return render_cx_list(
            &args.vault,
            rows,
            args.include_slots,
            &deadline,
            &mut progress,
        );
    }

    if let Some(limit) = args.limit {
        check_deadline(&deadline, &mut progress, "base_page_index_read", 0)?;
        progress.emit(json!({
            "event": "cx_list.progress",
            "phase": "base_page_index_read",
            "limit": limit,
            "elapsed_ms": deadline.elapsed_ms(),
        }))?;
        let rows = read_indexed_base_rows(&args.vault, limit)?;
        progress.emit(json!({
            "event": "cx_list.progress",
            "phase": "base_page_index_rows_loaded",
            "base_rows": rows.len(),
            "elapsed_ms": deadline.elapsed_ms(),
        }))?;
        check_deadline(
            &deadline,
            &mut progress,
            "base_page_index_rows_loaded",
            rows.len() as u64,
        )?;
        return render_cx_list(
            &args.vault,
            rows,
            args.include_slots,
            &deadline,
            &mut progress,
        );
    }

    check_deadline(&deadline, &mut progress, "base_scan", 0)?;
    let rows = latest_cf_rows(&args.vault, ColumnFamily::Base)?;
    progress.emit(json!({
        "event": "cx_list.progress",
        "phase": "base_rows_loaded",
        "base_rows": rows.len(),
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    check_deadline(
        &deadline,
        &mut progress,
        "base_rows_loaded",
        rows.len() as u64,
    )?;
    if rows.len() > CX_LIST_UNBOUNDED_ROW_LIMIT && !args.allow_unbounded {
        return Err(CliError::usage(format!(
            "cx-list would print {} rows from {}; use --cx-id <cx>, --limit <n>, or --allow-unbounded",
            rows.len(),
            args.vault.display()
        )));
    }
    render_cx_list(
        &args.vault,
        rows,
        args.include_slots,
        &deadline,
        &mut progress,
    )
}

fn parse_cx_list_args(rest: &[String]) -> CliResult<CxListArgs> {
    let mut vault = None;
    let mut cx_id = None;
    let mut limit = None;
    let mut include_slots = false;
    let mut allow_unbounded = false;
    let mut progress_jsonl = None;
    let mut time_budget_ms = None;
    let mut rebuild_base_page_index = false;
    let mut base_page_index_page_size = DEFAULT_BASE_PAGE_INDEX_PAGE_SIZE;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value(rest, idx, "--vault")?));
            }
            "--cx-id" => {
                idx += 1;
                let raw = value(rest, idx, "--cx-id")?;
                cx_id = Some(
                    CxId::from_str(raw)
                        .map_err(|error| CliError::usage(format!("invalid --cx-id: {error}")))?,
                );
            }
            "--limit" => {
                idx += 1;
                let raw = value(rest, idx, "--limit")?;
                limit = Some(parse_nonzero_usize(raw, "--limit")?);
            }
            "--allow-unbounded" => allow_unbounded = true,
            "--include-slots" => include_slots = true,
            "--rebuild-base-page-index" => rebuild_base_page_index = true,
            "--base-page-index-page-size" => {
                idx += 1;
                base_page_index_page_size = parse_nonzero_usize(
                    value(rest, idx, "--base-page-index-page-size")?,
                    "--base-page-index-page-size",
                )?;
            }
            "--progress-jsonl" => {
                idx += 1;
                progress_jsonl = Some(value(rest, idx, "--progress-jsonl")?.to_string());
            }
            "--time-budget-ms" => {
                idx += 1;
                time_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--time-budget-ms")?,
                    "--time-budget-ms",
                )?);
            }
            other => return Err(CliError::usage(format!("unexpected cx-list flag {other}"))),
        }
        idx += 1;
    }
    Ok(CxListArgs {
        vault: vault.ok_or_else(|| CliError::usage("cx-list requires --vault <dir>"))?,
        cx_id,
        limit,
        include_slots,
        allow_unbounded,
        progress_jsonl,
        time_budget_ms,
        rebuild_base_page_index,
        base_page_index_page_size,
    })
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn render_cx_list(
    vault: &Path,
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    include_slots: bool,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> crate::error::CliResult {
    let values = cx_list_rows(vault, rows, include_slots, deadline, progress)?;
    let json = serde_json::to_string_pretty(&values)
        .map_err(|error| CliError::runtime(format!("serialize cx-list rows: {error}")))?;
    print_line(&json)?;
    progress.emit(json!({
        "event": "cx_list.progress",
        "phase": "complete",
        "rows_rendered": values.len(),
        "include_slots": include_slots,
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    Ok(())
}

fn cx_list_rows(
    vault: &Path,
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    include_slots: bool,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> CliResult<Vec<serde_json::Value>> {
    let mut decoded = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        check_deadline(deadline, progress, "render_row", decoded.len() as u64)?;
        if is_tombstone_value(&value) {
            decoded.push((key, value, None));
            continue;
        }
        let cx = decode_constellation_base(&value)?;
        decoded.push((key, value, Some(cx)));
    }
    let slot_states = if include_slots {
        let live = decoded
            .iter()
            .filter_map(|(_, _, cx)| cx.as_ref())
            .collect::<Vec<_>>();
        physical_slot_states(vault, &live, deadline, progress)?
    } else {
        BTreeMap::new()
    };
    let mut values = Vec::with_capacity(decoded.len());
    for (key, value, cx) in decoded {
        let Some(cx) = cx else {
            values.push(tombstone_row(&key));
            continue;
        };
        let mut row = json!({
            "key_hex": hex_bytes(&key),
            "cx_id": cx.cx_id,
            "modality": &cx.modality,
            "created_at": cx.created_at,
            "panel_version": cx.panel_version,
            "input_ref": {
                "hash": hex_bytes(&cx.input_ref.hash),
                "pointer": cx.input_ref.pointer.as_deref(),
                "redacted": cx.input_ref.redacted,
            },
            "metadata": &cx.metadata,
            "provenance": {
                "seq": cx.provenance.seq,
                "hash": hex_bytes(&cx.provenance.hash),
            },
            "flags": cx.flags,
            "base_slot_count": cx.slots.len(),
            "base_hex": hex_bytes(&value),
            "slot_payloads_decoded": include_slots,
            "slot_payload_decode_mode": if include_slots { "physical_slot_cf_readback" } else { "base_only" },
        });
        if include_slots {
            let states = cx
                .slots
                .keys()
                .map(|slot| {
                    let state = slot_states.get(&(cx.cx_id, *slot)).ok_or_else(|| {
                        CliError::io(format!(
                            "cx-list internal error: no physical slot state resolved for cx {} slot {}",
                            cx.cx_id,
                            slot.get()
                        ))
                    })?;
                    Ok((*slot, state))
                })
                .collect::<CliResult<Vec<_>>>()?;
            row["slot_summary"] = slot_summary(states.iter().map(|(_, state)| *state));
            row["slots"] = json!(
                states
                    .iter()
                    .map(|(slot, state)| slot_row_json(*slot, state))
                    .collect::<Vec<_>>()
            );
        }
        values.push(row);
    }
    Ok(values)
}

fn check_deadline(
    deadline: &Deadline,
    progress: &mut ProgressSink,
    phase: &str,
    processed: u64,
) -> CliResult {
    match deadline.check("readback cx-list", phase, processed) {
        Ok(()) => Ok(()),
        Err(error) => {
            progress.emit(json!({
                "event": "cx_list.progress",
                "phase": "timeout",
                "processed": processed,
                "elapsed_ms": deadline.elapsed_ms(),
                "error_code": error.code(),
                "error": error.message(),
            }))?;
            Err(error)
        }
    }
}
