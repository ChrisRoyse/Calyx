use std::collections::BTreeSet;

use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::erase::{EraseRegistry, EraseScope};
use calyx_core::{Ts, VaultStore};
use serde_json::{Value, json};

use super::{Engine, EngineResult, VaultHandle, parse_params, vault_not_open};
use crate::paths::VaultRef;
use codec::{
    append_duplicate_anchors, append_recurrence_if_needed, constellation_value,
    constellation_value_from_base, cx_id_from_key, ensure_tombstone_index, index_erase_tombstone,
    parse_cx_id, predicted_id, prepare_put, scan_tombstones, validate_scan_limit,
};
use params::{
    CxAnchorParams, CxDeleteParams, CxGetParams, CxPutBatchParams, CxPutParams, CxScanParams,
};

impl Engine {
    pub(super) fn cx_put(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxPutParams>(params, "cx.put")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let prepared = prepare_put(handle, params.item, params.ts, false)?;
        let id = prepared.constellation.cx_id;
        let deduped = prepared.deduped;
        if deduped {
            append_duplicate_anchors(handle, &prepared.constellation)?;
        } else {
            handle.vault.put(prepared.constellation.clone())?;
        }
        let recurrence = append_recurrence_if_needed(handle, id, params.ts, prepared, deduped)?;
        handle.vault.flush()?;
        let stored = handle.vault.get(id, handle.vault.snapshot())?;
        Ok(json!({
            "status": "put",
            "vault_ref": handle.vault_ref.as_str(),
            "cx_id": id.to_string(),
            "deduped": deduped,
            "recurrence_occurrence": recurrence,
            "latest_seq": handle.vault.latest_seq(),
            "item": constellation_value(&stored),
        }))
    }

    pub(super) fn cx_put_batch(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxPutBatchParams>(params, "cx.put_batch")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let mut prepared = Vec::with_capacity(params.items.len());
        let mut seen = BTreeSet::new();
        for item in params.items {
            let within_batch_duplicate = !seen.insert(predicted_id(handle, &item)?);
            prepared.push(prepare_put(
                handle,
                item,
                params.ts,
                within_batch_duplicate,
            )?);
        }

        let constellations = prepared
            .iter()
            .filter(|item| !item.deduped)
            .map(|item| item.constellation.clone())
            .collect::<Vec<_>>();
        handle.vault.put_batch(constellations)?;
        let mut response_items = Vec::with_capacity(prepared.len());
        for prepared in prepared {
            let id = prepared.constellation.cx_id;
            let deduped = prepared.deduped;
            if deduped {
                append_duplicate_anchors(handle, &prepared.constellation)?;
            }
            let recurrence = append_recurrence_if_needed(handle, id, params.ts, prepared, deduped)?;
            let stored = handle.vault.get(id, handle.vault.snapshot())?;
            response_items.push(json!({
                "cx_id": id.to_string(),
                "deduped": deduped,
                "recurrence_occurrence": recurrence,
                "item": constellation_value(&stored),
            }));
        }
        handle.vault.flush()?;
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
            "item": constellation_value(&constellation),
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
            items.push(constellation_value_from_base(id, value)?);
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
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let cx_id = parse_cx_id(&params.cx_id)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        handle.vault.anchor(cx_id, params.anchor)?;
        handle.vault.flush()?;
        let stored = handle.vault.get(cx_id, handle.vault.snapshot())?;
        Ok(json!({
            "status": "anchored",
            "vault_ref": handle.vault_ref.as_str(),
            "cx_id": cx_id.to_string(),
            "anchor_count": stored.anchors.len(),
            "latest_seq": handle.vault.latest_seq(),
            "item": constellation_value(&stored),
        }))
    }

    pub(super) fn cx_delete(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CxDeleteParams>(params, "cx.delete")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let cx_id = parse_cx_id(&params.cx_id)?;
        let handle = self.open_vault_for_cx(&vault_ref, params.ts)?;
        let erase = handle.vault.erase(
            EraseScope::Cx(cx_id),
            &mut handle.context,
            &EraseRegistry::new(),
        )?;
        index_erase_tombstone(handle, erase.tombstone.as_ref())?;
        handle.vault.flush()?;
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
        ensure_tombstone_index(handle)?;
        Ok(handle)
    }
}

mod codec;
mod params;

pub(super) fn ensure_cx_tombstone_index(handle: &VaultHandle) -> EngineResult<()> {
    ensure_tombstone_index(handle)
}
