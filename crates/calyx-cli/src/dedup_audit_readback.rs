use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, SlotId, SlotVector};
use serde_json::json;

use crate::cf_read::{hex_bytes, latest_cf_rows, vault_id_from_base};

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

pub fn readback_cx_list(vault: &Path) -> crate::error::CliResult {
    let rows = latest_cf_rows(vault, ColumnFamily::Base)?;
    let mut values = Vec::new();
    let mut slot_cache = BTreeMap::<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>::new();
    let mut raw_slot_cache = BTreeMap::<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>::new();
    for (key, value) in rows {
        let cx = decode_constellation_base(&value).map_err(|error| error.to_string())?;
        let slots = decoded_slot_entries(vault, &mut slot_cache, &mut raw_slot_cache, &cx)?;
        values.push(json!({
            "key_hex": hex_bytes(&key),
            "cx_id": cx.cx_id,
            "created_at": cx.created_at,
            "panel_version": cx.panel_version,
            "flags": cx.flags,
            "slot_summary": slot_summary(slots.iter().map(|(_, vector, _)| vector)),
            "slots": slots.iter().map(|(slot, vector, source)| {
                match vector {
                    SlotVector::Dense { dim, data } => json!({
                        "slot": slot.get(),
                        "kind": "dense",
                        "payload_source": source,
                        "dim": dim,
                        "values": data.len(),
                    }),
                    SlotVector::Sparse { dim, entries } => json!({
                        "slot": slot.get(),
                        "kind": "sparse",
                        "payload_source": source,
                        "dim": dim,
                        "entries": entries.len(),
                    }),
                    SlotVector::Multi { token_dim, tokens } => json!({
                        "slot": slot.get(),
                        "kind": "multi",
                        "payload_source": source,
                        "token_dim": token_dim,
                        "tokens": tokens.len(),
                    }),
                    SlotVector::Absent { reason } => json!({
                        "slot": slot.get(),
                        "kind": "absent",
                        "payload_source": source,
                        "reason": reason,
                    }),
                }
            }).collect::<Vec<_>>(),
            "base_hex": hex_bytes(&value),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn decoded_slot_entries(
    vault: &Path,
    slot_cache: &mut BTreeMap<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>,
    raw_slot_cache: &mut BTreeMap<SlotId, BTreeMap<Vec<u8>, Vec<u8>>>,
    cx: &calyx_core::Constellation,
) -> Result<Vec<(SlotId, SlotVector, &'static str)>, String> {
    let key = slot_key(cx.cx_id);
    let mut out = Vec::with_capacity(cx.slots.len());
    for (slot, placeholder) in &cx.slots {
        if !slot_cache.contains_key(slot) {
            slot_cache.insert(*slot, latest_cf_rows(vault, ColumnFamily::slot(*slot))?);
        }
        let Some(value) = slot_cache.get(slot).and_then(|rows| rows.get(&key)) else {
            out.push((
                *slot,
                placeholder.clone(),
                "base_hash_placeholder_missing_slot_cf",
            ));
            continue;
        };
        match decode_slot_vector(value) {
            Ok(vector) => out.push((*slot, vector, "slot_cf")),
            Err(_) => {
                if !raw_slot_cache.contains_key(slot) {
                    raw_slot_cache
                        .insert(*slot, latest_cf_rows(vault, ColumnFamily::slot_raw(*slot))?);
                }
                let vector = raw_slot_cache
                    .get(slot)
                    .and_then(|rows| rows.get(&key))
                    .map(|raw| decode_slot_vector(raw).map_err(|error| error.to_string()))
                    .transpose()?;
                out.push(match vector {
                    Some(vector) => (*slot, vector, "slot_raw_cf"),
                    None => (
                        *slot,
                        placeholder.clone(),
                        "base_hash_placeholder_missing_raw_cf",
                    ),
                });
            }
        }
    }
    Ok(out)
}

fn slot_summary<'a>(vectors: impl Iterator<Item = &'a SlotVector>) -> serde_json::Value {
    let mut dense_slots = 0usize;
    let mut sparse_slots = 0usize;
    let mut multi_slots = 0usize;
    let mut absent_reasons = BTreeMap::<String, usize>::new();
    for vector in vectors {
        match vector {
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
        }
    }
    json!({
        "slot_count": dense_slots + sparse_slots + multi_slots + absent_reasons.values().sum::<usize>(),
        "dense_slots": dense_slots,
        "sparse_slots": sparse_slots,
        "multi_slots": multi_slots,
        "absent_slots": absent_reasons.values().sum::<usize>(),
        "absent_reasons": absent_reasons,
    })
}
