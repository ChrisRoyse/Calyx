use std::collections::BTreeSet;

use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::erase::{EraseRegistry, EraseScope};
use calyx_core::{Ts, VaultStore};
use serde_json::{Value, json};

use super::{Engine, EngineResult, VaultHandle, parse_params, vault_not_open};
use crate::paths::VaultRef;
use anchors::{merge_duplicate_anchors, repair_duplicate_anchor_bloat};
use codec::{
    append_recurrence_if_needed, constellation_value, constellation_value_from_base,
    cx_id_from_key, decode_put_input, delete_input_row, input_row, parse_cx_id, prepare_put,
    prepare_put_decoded, put_ack_value, validate_batch_bytes, validate_batch_items,
    validate_scan_limit,
};
use params::{
    CxAnchorParams, CxDeleteParams, CxGetParams, CxPutBatchParams, CxPutParams, CxScanParams,
};
use tombstone::{ensure_tombstone_index, index_erase_tombstone, scan_tombstones};

impl Engine {
    pub(super) fn cx_put(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxPutParams>(params, "cx.put")?;
        let flush_policy = self.config.flush_policy.clone();
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let prepared = prepare_put(handle, params.item, params.ts, false)?;
        let id = prepared.constellation.cx_id;
        let deduped = prepared.deduped;
        let input_len = prepared.input_len;
        let input_hash = prepared.input_hash;
        let recurrence_context = prepared.recurrence_context;
        if deduped {
            merge_duplicate_anchors(handle, &prepared.constellation)?;
        } else {
            let row = input_row(id, prepared.input_bytes);
            handle.vault.put(prepared.constellation)?;
            handle.vault.write_cf_batch([row])?;
        }
        let recurrence =
            append_recurrence_if_needed(handle, id, params.ts, recurrence_context, deduped)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "put",
            "vault_ref": handle.vault_ref.as_str(),
            "cx_id": id.to_string(),
            "deduped": deduped,
            "recurrence_occurrence": recurrence,
            "latest_seq": handle.vault.latest_seq(),
            "item": put_ack_value(id, deduped, recurrence, input_len, &input_hash),
        }))
    }

    pub(super) fn cx_put_batch(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxPutBatchParams>(params, "cx.put_batch")?;
        validate_batch_items(params.items.len())?;
        let flush_policy = self.config.flush_policy.clone();
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let mut prepared = Vec::with_capacity(params.items.len());
        let mut seen = BTreeSet::new();
        let mut total_bytes = 0_usize;
        for item in params.items {
            let decoded = decode_put_input(&item)?;
            total_bytes = total_bytes.saturating_add(decoded.bytes.len());
            validate_batch_bytes(total_bytes)?;
            let id = handle
                .vault
                .cx_id_for_input(&decoded.bytes, item.panel_version);
            let within_batch_duplicate = !seen.insert(id);
            prepared.push(prepare_put_decoded(
                handle,
                item,
                params.ts,
                within_batch_duplicate,
                decoded,
                id,
            )?);
        }

        let mut constellations = Vec::new();
        let mut duplicate_constellations = Vec::new();
        let mut input_rows = Vec::new();
        let mut ack_parts = Vec::with_capacity(prepared.len());
        for prepared in prepared {
            let id = prepared.constellation.cx_id;
            let deduped = prepared.deduped;
            if deduped {
                duplicate_constellations.push(prepared.constellation);
            } else {
                input_rows.push(input_row(id, prepared.input_bytes));
                constellations.push(prepared.constellation);
            }
            ack_parts.push((
                id,
                deduped,
                prepared.recurrence_context,
                prepared.input_len,
                prepared.input_hash,
            ));
        }
        handle.vault.put_batch(constellations)?;
        handle.vault.write_cf_batch(input_rows)?;
        for constellation in &duplicate_constellations {
            merge_duplicate_anchors(handle, constellation)?;
        }
        let mut response_items = Vec::with_capacity(ack_parts.len());
        for (id, deduped, recurrence_context, input_len, input_hash) in ack_parts {
            let recurrence =
                append_recurrence_if_needed(handle, id, params.ts, recurrence_context, deduped)?;
            response_items.push(put_ack_value(
                id,
                deduped,
                recurrence,
                input_len,
                &input_hash,
            ));
        }
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "put_batch",
            "vault_ref": handle.vault_ref.as_str(),
            "count": response_items.len(),
            "latest_seq": handle.vault.latest_seq(),
            "items": response_items,
        }))
    }

    pub(super) fn cx_get(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxGetParams>(params, "cx.get")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let cx_id = parse_cx_id(&params.cx_id)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let snapshot = params.snapshot.unwrap_or_else(|| handle.vault.snapshot());
        let constellation = handle.vault.get(cx_id, snapshot)?;
        Ok(json!({
            "status": "found",
            "vault_ref": handle.vault_ref.as_str(),
            "snapshot": snapshot,
            "item": constellation_value(handle, snapshot, &constellation, params.include_input)?,
        }))
    }

    pub(super) fn cx_scan(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxScanParams>(params, "cx.scan")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let limit = validate_scan_limit(params.limit)?;
        let after_key = params
            .cursor
            .as_deref()
            .map(parse_cx_id)
            .transpose()?
            .map(|id| id.as_bytes().to_vec());
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let snapshot = params.snapshot.unwrap_or_else(|| handle.vault.snapshot());
        let rows = handle.vault.scan_cf_range_page_at(
            snapshot,
            ColumnFamily::Base,
            &KeyRange {
                start: Vec::new(),
                end: None,
            },
            after_key.as_deref(),
            limit,
        )?;
        let mut items = Vec::with_capacity(rows.len());
        for (key, value) in &rows {
            let id = cx_id_from_key(key)?;
            items.push(constellation_value_from_base(
                handle,
                snapshot,
                id,
                value,
                params.include_input,
            )?);
        }
        let next_cursor = if rows.len() == limit {
            rows.last()
                .map(|(key, _)| cx_id_from_key(key))
                .transpose()?
                .map(|id| id.to_string())
        } else {
            None
        };
        let tombstones = scan_tombstones(handle, snapshot)?;
        Ok(json!({
            "status": "scanned",
            "vault_ref": handle.vault_ref.as_str(),
            "snapshot": snapshot,
            "limit": limit,
            "next_cursor": next_cursor,
            "items": items,
            "tombstones": tombstones.items,
            "tombstones_truncated": tombstones.truncated,
        }))
    }

    pub(super) fn cx_anchor(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxAnchorParams>(params, "cx.anchor")?;
        let flush_policy = self.config.flush_policy.clone();
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let cx_id = parse_cx_id(&params.cx_id)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        handle.vault.anchor(cx_id, params.anchor)?;
        handle.flush_after_write(&flush_policy)?;
        let stored = handle.vault.get(cx_id, handle.vault.snapshot())?;
        Ok(json!({
            "status": "anchored",
            "vault_ref": handle.vault_ref.as_str(),
            "cx_id": cx_id.to_string(),
            "anchor_count": stored.anchors.len(),
            "latest_seq": handle.vault.latest_seq(),
            "item": constellation_value(handle, handle.vault.snapshot(), &stored, false)?,
        }))
    }

    pub(super) fn cx_delete(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxDeleteParams>(params, "cx.delete")?;
        let flush_policy = self.config.flush_policy.clone();
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let cx_id = parse_cx_id(&params.cx_id)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let context = handle
            .context
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let erase = handle.vault.erase_defer_key_shred(
            EraseScope::Cx(cx_id),
            &context,
            &EraseRegistry::new(),
        )?;
        drop(context);
        delete_input_row(handle, cx_id)?;
        index_erase_tombstone(handle, erase.tombstone.as_ref())?;
        handle.flush_after_write(&flush_policy)?;
        handle
            .context
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shred_key_for_erasure();
        let tombstones = scan_tombstones(handle, handle.vault.snapshot())?;
        Ok(json!({
            "status": "deleted",
            "vault_ref": handle.vault_ref.as_str(),
            "cx_id": cx_id.to_string(),
            "latest_seq": handle.vault.latest_seq(),
            "erase": erase,
            "tombstones": tombstones.items,
            "tombstones_truncated": tombstones.truncated,
        }))
    }

    fn open_vault_for_cx(
        &mut self,
        vault_ref: &VaultRef,
        ts: Ts,
    ) -> EngineResult<&mut VaultHandle> {
        let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) else {
            return Err(vault_not_open(vault_ref.as_str()).into());
        };
        handle.touch(ts);
        handle.charge_query(ts)?;
        ensure_tombstone_index(handle)?;
        Ok(handle)
    }
}

mod anchors;
mod codec;
mod params;
mod tombstone;

pub(super) fn ensure_cx_tombstone_index(handle: &VaultHandle) -> EngineResult<()> {
    ensure_tombstone_index(handle)
}

pub(super) fn repair_cx_anchor_bloat(handle: &VaultHandle) -> EngineResult<()> {
    repair_duplicate_anchor_bloat(handle)
}
