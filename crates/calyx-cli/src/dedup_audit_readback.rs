use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::base_page_index::{DEFAULT_BASE_PAGE_INDEX_PAGE_SIZE, read_indexed_base_rows};
use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use serde_json::json;

use crate::bounded_progress::{Deadline, ProgressSink, parse_nonzero_u64, parse_nonzero_usize};
use crate::cf_read::{
    hex_bytes, latest_cf_row, latest_cf_rows, latest_cf_rows_for_keys, latest_cf_rows_near_seqs,
    vault_id_from_base,
};
use crate::error::{CliError, CliResult};
use crate::output::print_line;

const CX_LIST_UNBOUNDED_ROW_LIMIT: usize = 100;

mod index_progress;
#[cfg(test)]
mod tests;

use index_progress::rebuild_cx_list_base_page_index;

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
    let cx_id = CxId::from_str(cx_id).map_err(|error| format!("invalid --cx-id: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let report = dedup_audit(&store, cx_id).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_dedup_undo(vault: &Path, token: &str) -> crate::error::CliResult {
    let token: ReversalToken =
        serde_json::from_str(token).map_err(|error| format!("invalid --token: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let before = latest_cf_rows(vault, ColumnFamily::Base)?;
    let restored = dedup_undo(&store, &token).map_err(|error| error.to_string())?;
    store.flush().map_err(|error| error.to_string())?;
    let after = latest_cf_rows(vault, ColumnFamily::Base)?;
    let value = json!({
        "vault": vault.display().to_string(),
        "restored": restored,
        "base_rows_before": before.len(),
        "base_rows_after": after.len(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
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
    let json = serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?;
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
        let cx = decode_constellation_base(&value).map_err(|error| error.to_string())?;
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
            "created_at": cx.created_at,
            "panel_version": cx.panel_version,
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
                    .map(|(slot, state)| slot_row_json(*slot, *state))
                    .collect::<Vec<_>>()
            );
        }
        values.push(row);
    }
    Ok(values)
}

fn slot_row_json(slot: SlotId, state: &PhysicalSlotState) -> serde_json::Value {
    match state {
        PhysicalSlotState::Tombstoned { payload_source } => json!({
            "slot": slot.get(),
            "kind": "tombstoned",
            "payload_source": payload_source,
        }),
        PhysicalSlotState::Vector {
            vector,
            payload_source,
        } => match vector {
            SlotVector::Dense { dim, data } => json!({
                "slot": slot.get(),
                "kind": "dense",
                "payload_source": payload_source,
                "dim": dim,
                "values": data.len(),
            }),
            SlotVector::Sparse { dim, entries } => json!({
                "slot": slot.get(),
                "kind": "sparse",
                "payload_source": payload_source,
                "dim": dim,
                "entries": entries.len(),
            }),
            SlotVector::Multi { token_dim, tokens } => json!({
                "slot": slot.get(),
                "kind": "multi",
                "payload_source": payload_source,
                "token_dim": token_dim,
                "tokens": tokens.len(),
            }),
            SlotVector::Absent { reason } => json!({
                "slot": slot.get(),
                "kind": "absent",
                "payload_source": payload_source,
                "reason": reason,
            }),
        },
    }
}

fn tombstone_row(key: &[u8]) -> serde_json::Value {
    json!({
        "key_hex": hex_bytes(key),
        "cx_id": cx_id_from_base_key(key).map(|id| id.to_string()),
        "base_visible": false,
        "tombstoned": true,
        "slot_payloads_decoded": false,
        "slot_payload_decode_mode": "mvcc_tombstone",
    })
}

fn cx_id_from_base_key(key: &[u8]) -> Option<CxId> {
    let bytes: [u8; 16] = key.try_into().ok()?;
    Some(CxId::from_bytes(bytes))
}

/// Physical state of one slot resolved from the slot column families.
///
/// Base rows persist only slot ids plus payload hashes, so
/// [`decode_constellation_base`] always yields `Absent` placeholders that
/// carry no payload state. The slot CFs (and `slot_raw` for compressed slots)
/// are the only physical source of truth `--include-slots` may report from.
#[derive(Debug)]
enum PhysicalSlotState {
    Vector {
        vector: SlotVector,
        payload_source: &'static str,
    },
    Tombstoned {
        payload_source: &'static str,
    },
}

/// Reads the physical slot CF rows for every base-listed slot of every live
/// constellation, grouped per slot CF (the same grouped near-seq read path
/// `weave-loom` dense-slot coverage uses). Ingest stages one physical slot row
/// per base-listed slot in the same WAL batch as the base row, so a
/// base-listed slot with no physical `slot_XX`/`slot_raw_XX` row fails closed
/// as `CALYX_ASTER_CORRUPT_SHARD` instead of being reported as absent.
fn physical_slot_states(
    vault: &Path,
    constellations: &[&Constellation],
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> CliResult<BTreeMap<(CxId, SlotId), PhysicalSlotState>> {
    let mut per_slot: BTreeMap<SlotId, Vec<(CxId, Vec<u8>, u64)>> = BTreeMap::new();
    for cx in constellations {
        for slot in cx.slots.keys() {
            per_slot.entry(*slot).or_default().push((
                cx.cx_id,
                slot_key(cx.cx_id),
                cx.provenance.seq,
            ));
        }
    }
    let mut out = BTreeMap::new();
    for (slot, members) in per_slot {
        check_deadline(deadline, progress, "slot_lookup", out.len() as u64)?;
        progress.emit(json!({
            "event": "cx_list.progress",
            "phase": "slot_lookup",
            "slot": slot.get(),
            "rows": members.len(),
            "elapsed_ms": deadline.elapsed_ms(),
        }))?;
        let pairs = members
            .iter()
            .map(|(_, key, seq)| (key.clone(), *seq))
            .collect::<Vec<_>>();
        let near = latest_cf_rows_near_seqs(vault, ColumnFamily::slot(slot), &pairs)
            .map_err(|error| slot_readback_io_error(slot, "near-seq", &error))?;
        let missing = members
            .iter()
            .filter(|(_, key, _)| !matches!(near.get(key), Some(Some(_))))
            .map(|(_, key, _)| key.clone())
            .collect::<Vec<_>>();
        let full = if missing.is_empty() {
            BTreeMap::new()
        } else {
            latest_cf_rows_for_keys(vault, ColumnFamily::slot(slot), &missing)
                .map_err(|error| slot_readback_io_error(slot, "full-set", &error))?
        };
        for (cx_id, key, seq) in &members {
            let located = match near.get(key) {
                Some(Some(bytes)) => Some((bytes.clone(), "slot_cf")),
                _ => full
                    .get(key)
                    .and_then(Clone::clone)
                    .map(|bytes| (bytes, "slot_cf_full_set")),
            };
            let state = resolve_slot_state(vault, *cx_id, slot, key, *seq, located)?;
            out.insert((*cx_id, slot), state);
        }
    }
    Ok(out)
}

fn resolve_slot_state(
    vault: &Path,
    cx_id: CxId,
    slot: SlotId,
    key: &[u8],
    seq: u64,
    located: Option<(Vec<u8>, &'static str)>,
) -> CliResult<PhysicalSlotState> {
    let Some((bytes, payload_source)) = located else {
        // No physical slot CF row at all: the raw CF is the only remaining
        // physical location a payload could live in (compression writes both).
        return match physical_raw_row(vault, slot, key, seq)? {
            Some((raw_bytes, raw_source)) if is_tombstone_value(&raw_bytes) => {
                Ok(PhysicalSlotState::Tombstoned {
                    payload_source: raw_source,
                })
            }
            Some((raw_bytes, raw_source)) => decode_slot_vector(&raw_bytes)
                .map(|vector| PhysicalSlotState::Vector {
                    vector,
                    payload_source: raw_source,
                })
                .map_err(|error| {
                    missing_slot_row_error(
                        vault,
                        cx_id,
                        slot,
                        seq,
                        &format!("slot_raw row exists but failed to decode: {error}"),
                    )
                }),
            None => Err(missing_slot_row_error(
                vault,
                cx_id,
                slot,
                seq,
                "no row found in the slot CF (near-seq or full SST set + WAL) nor in the slot_raw CF",
            )),
        };
    };
    if is_tombstone_value(&bytes) {
        return Ok(PhysicalSlotState::Tombstoned {
            payload_source: match payload_source {
                "slot_cf" => "slot_cf_tombstone",
                _ => "slot_cf_full_set_tombstone",
            },
        });
    }
    match decode_slot_vector(&bytes) {
        Ok(vector) => Ok(PhysicalSlotState::Vector {
            vector,
            payload_source,
        }),
        Err(decode_error) => match physical_raw_row(vault, slot, key, seq)? {
            // Compressed slots persist opaque compressed bytes in the slot CF
            // and the decodable payload in slot_raw.
            Some((raw_bytes, raw_source)) if !is_tombstone_value(&raw_bytes) => {
                decode_slot_vector(&raw_bytes)
                    .map(|vector| PhysicalSlotState::Vector {
                        vector,
                        payload_source: raw_source,
                    })
                    .map_err(|raw_error| {
                        undecodable_slot_row_error(
                            vault,
                            cx_id,
                            slot,
                            seq,
                            &format!(
                                "slot CF decode failed ({decode_error}) and slot_raw CF decode failed ({raw_error})"
                            ),
                        )
                    })
            }
            _ => Err(undecodable_slot_row_error(
                vault,
                cx_id,
                slot,
                seq,
                &format!("slot CF decode failed ({decode_error}) and no decodable slot_raw row exists"),
            )),
        },
    }
}

fn physical_raw_row(
    vault: &Path,
    slot: SlotId,
    key: &[u8],
    seq: u64,
) -> CliResult<Option<(Vec<u8>, &'static str)>> {
    let near =
        latest_cf_rows_near_seqs(vault, ColumnFamily::slot_raw(slot), &[(key.to_vec(), seq)])
            .map_err(|error| slot_readback_io_error(slot, "slot_raw near-seq", &error))?;
    if let Some(Some(bytes)) = near.get(key) {
        return Ok(Some((bytes.clone(), "slot_raw_cf")));
    }
    let full = latest_cf_rows_for_keys(vault, ColumnFamily::slot_raw(slot), &[key.to_vec()])
        .map_err(|error| slot_readback_io_error(slot, "slot_raw full-set", &error))?;
    Ok(full
        .get(key)
        .and_then(Clone::clone)
        .map(|bytes| (bytes, "slot_raw_cf_full_set")))
}

fn slot_readback_io_error(slot: SlotId, phase: &str, error: &str) -> CliError {
    CliError::io(format!(
        "cx-list physical slot readback ({phase}) failed for slot {}: {error}",
        slot.get()
    ))
}

fn missing_slot_row_error(
    vault: &Path,
    cx_id: CxId,
    slot: SlotId,
    seq: u64,
    detail: &str,
) -> CliError {
    CalyxError::aster_corrupt_shard(format!(
        "cx-list --include-slots fail-closed: base row for cx {cx_id} in {} lists slot {} \
         (provenance seq {seq}) but no physical slot payload row exists: {detail}",
        vault.display(),
        slot.get()
    ))
    .into()
}

fn undecodable_slot_row_error(
    vault: &Path,
    cx_id: CxId,
    slot: SlotId,
    seq: u64,
    detail: &str,
) -> CliError {
    CalyxError::aster_corrupt_shard(format!(
        "cx-list --include-slots fail-closed: physical slot row for cx {cx_id} slot {} \
         (provenance seq {seq}) in {} is not decodable: {detail}",
        slot.get(),
        vault.display()
    ))
    .into()
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

fn slot_summary<'a>(states: impl Iterator<Item = &'a PhysicalSlotState>) -> serde_json::Value {
    let mut dense_slots = 0usize;
    let mut sparse_slots = 0usize;
    let mut multi_slots = 0usize;
    let mut tombstoned_slots = 0usize;
    let mut absent_reasons = BTreeMap::<String, usize>::new();
    for state in states {
        match state {
            PhysicalSlotState::Tombstoned { .. } => tombstoned_slots += 1,
            PhysicalSlotState::Vector { vector, .. } => match vector {
                SlotVector::Dense { .. } => dense_slots += 1,
                SlotVector::Sparse { .. } => sparse_slots += 1,
                SlotVector::Multi { .. } => multi_slots += 1,
                SlotVector::Absent { reason } => {
                    let key = serde_json::to_value(reason)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_else(|| format!("{reason:?}"));
                    *absent_reasons.entry(key).or_insert(0) += 1;
                }
            },
        }
    }
    let absent_slots = absent_reasons.values().sum::<usize>();
    json!({
        "slot_count": dense_slots + sparse_slots + multi_slots + tombstoned_slots + absent_slots,
        "dense_slots": dense_slots,
        "sparse_slots": sparse_slots,
        "multi_slots": multi_slots,
        "tombstoned_slots": tombstoned_slots,
        "absent_slots": absent_slots,
        "absent_reasons": absent_reasons,
    })
}
