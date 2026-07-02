//! Physical slot-CF readback for `cx-list --include-slots`.
//!
//! Base rows persist only slot ids plus payload hashes, so
//! [`decode_constellation_base`] always yields `Absent` placeholders that carry
//! no payload state. The slot CFs (and `slot_raw` for compressed slots) are the
//! only physical source of truth `--include-slots` may report from. Split out of
//! the parent module to keep each file within the modularization line budget
//! (issue #1098); behavior is unchanged.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::vault::encode::decode_slot_vector;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use serde_json::json;

use super::check_deadline;
use crate::bounded_progress::{Deadline, ProgressSink};
use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};
use crate::provenance_read::{ResolvedRow, RowSource, VaultReadContext};

pub(super) fn slot_row_json(slot: SlotId, state: &PhysicalSlotState) -> serde_json::Value {
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

pub(super) fn tombstone_row(key: &[u8]) -> serde_json::Value {
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
#[derive(Debug)]
pub(super) enum PhysicalSlotState {
    Vector {
        vector: SlotVector,
        payload_source: &'static str,
    },
    Tombstoned {
        payload_source: &'static str,
    },
}

/// Reads the physical slot CF rows for every base-listed slot of every live
/// constellation, grouped per slot CF (the same exact provenance-resolved
/// read path `weave-loom` dense-slot coverage uses, issue #1096). Ingest
/// stages one physical slot row per base-listed slot in the same WAL batch as
/// the base row, so a base-listed slot with no physical
/// `slot_XX`/`slot_raw_XX` row fails closed as `CALYX_ASTER_CORRUPT_SHARD`
/// instead of being reported as absent.
pub(super) fn physical_slot_states(
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
    let mut read_context = VaultReadContext::new(vault);
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
        let batch = read_context
            .latest_cf_rows_for_provenance(ColumnFamily::slot(slot), &pairs)
            .map_err(|error| slot_readback_io_error(slot, "provenance-resolved", &error))?;
        progress.emit(json!({
            "event": "cx_list.progress",
            "phase": "slot_lookup_resolved",
            "slot": slot.get(),
            "read_stats": batch.stats,
            "elapsed_ms": deadline.elapsed_ms(),
        }))?;
        for (cx_id, key, seq) in &members {
            let located = batch
                .rows
                .get(key)
                .and_then(Option::as_ref)
                .map(|row| (row.value.clone(), slot_cf_payload_source(row)));
            let state =
                resolve_slot_state(vault, &mut read_context, *cx_id, slot, key, *seq, located)?;
            out.insert((*cx_id, slot), state);
        }
    }
    Ok(out)
}

/// `payload_source` label for a slot-CF row by resolution stage. Commit-batch
/// and WAL-tail reads keep the historical `slot_cf` label; full-level reads
/// keep the `slot_cf_full_set` label introduced by issue #1060.
fn slot_cf_payload_source(row: &ResolvedRow) -> &'static str {
    match row.source {
        RowSource::CommitBatch | RowSource::WalTail => "slot_cf",
        RowSource::FullSet => "slot_cf_full_set",
    }
}

fn resolve_slot_state(
    vault: &Path,
    read_context: &mut VaultReadContext,
    cx_id: CxId,
    slot: SlotId,
    key: &[u8],
    seq: u64,
    located: Option<(Vec<u8>, &'static str)>,
) -> CliResult<PhysicalSlotState> {
    let Some((bytes, payload_source)) = located else {
        // No physical slot CF row at all: the raw CF is the only remaining
        // physical location a payload could live in (compression writes both).
        return match physical_raw_row(read_context, slot, key, seq)? {
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
                "no row found in the slot CF (commit batch, full SST set, or WAL tail) nor in the slot_raw CF",
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
        Err(decode_error) => match physical_raw_row(read_context, slot, key, seq)? {
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
    read_context: &mut VaultReadContext,
    slot: SlotId,
    key: &[u8],
    seq: u64,
) -> CliResult<Option<(Vec<u8>, &'static str)>> {
    let batch = read_context
        .latest_cf_rows_for_provenance(ColumnFamily::slot_raw(slot), &[(key.to_vec(), seq)])
        .map_err(|error| slot_readback_io_error(slot, "slot_raw provenance-resolved", &error))?;
    Ok(batch.rows.get(key).and_then(Option::as_ref).map(|row| {
        let source = match row.source {
            RowSource::CommitBatch | RowSource::WalTail => "slot_raw_cf",
            RowSource::FullSet => "slot_raw_cf_full_set",
        };
        (row.value.clone(), source)
    }))
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

pub(super) fn slot_summary<'a>(
    states: impl Iterator<Item = &'a PhysicalSlotState>,
) -> serde_json::Value {
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
