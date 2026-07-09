use std::time::Duration;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::CollectionMode;
use calyx_aster::index::btree::btree_range_at;
use calyx_aster::layers::blob::{BLOB_CHUNK_SIZE, BlobId};
use calyx_aster::layers::relational::{decode_record_value, record_key};
use calyx_aster::layers::{BlobLayer, KvLayer, RelationalLayer, TimeSeriesLayer};
use calyx_core::{Ts, VaultStore};
use serde_json::{Value, json};

use super::{Engine, EngineResult, VaultHandle, parse_params, vault_not_open};
use crate::paths::VaultRef;
use codec::*;
use params::{
    BlobGetParams, BlobPutParams, KvDeleteParams, KvGetParams, KvSetParams, RelDeleteParams,
    RelGetParams, RelInsertParams, RelQueryParams, RelScanParams, RelUpdateParams, TsRangeParams,
    TsWriteParams, TxnCommitParams,
};

const CALYX_LEAPABLE_STORAGE_INPUT_INVALID: &str = "CALYX_LEAPABLE_STORAGE_INPUT_INVALID";
const CALYX_LEAPABLE_COLLECTION_MISMATCH: &str = "CALYX_LEAPABLE_COLLECTION_MISMATCH";
const CALYX_LEAPABLE_REL_CONFLICT: &str = "CALYX_LEAPABLE_REL_CONFLICT";
const CALYX_LEAPABLE_REL_NOT_FOUND: &str = "CALYX_LEAPABLE_REL_NOT_FOUND";
const CALYX_LEAPABLE_INDEX_NOT_FOUND: &str = "CALYX_LEAPABLE_INDEX_NOT_FOUND";
const CALYX_LEAPABLE_TXN_INJECTED_CRASH: &str = "CALYX_LEAPABLE_TXN_INJECTED_CRASH";

pub(super) fn is_storage_method(method: &str) -> bool {
    matches!(
        method,
        "rel.insert"
            | "rel.get"
            | "rel.update_row"
            | "rel.delete"
            | "rel.scan"
            | "rel.query"
            | "kv.set"
            | "kv.get"
            | "kv.delete"
            | "ts.write"
            | "ts.range"
            | "blob.put"
            | "blob.get"
            | "txn.commit"
    )
}

impl Engine {
    pub(super) fn dispatch_storage(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> EngineResult<Value> {
        match method {
            "rel.insert" => self.rel_insert(params),
            "rel.get" => self.rel_get(params),
            "rel.update_row" => self.rel_update_row(params),
            "rel.delete" => self.rel_delete(params),
            "rel.scan" => self.rel_scan(params),
            "rel.query" => self.rel_query(params),
            "kv.set" => self.kv_set(params),
            "kv.get" => self.kv_get(params),
            "kv.delete" => self.kv_delete(params),
            "ts.write" => self.ts_write(params),
            "ts.range" => self.ts_range(params),
            "blob.put" => self.blob_put(params),
            "blob.get" => self.blob_get(params),
            "txn.commit" => self.txn_commit(params),
            _ => unreachable!("storage method prefiltered"),
        }
    }

    fn rel_insert(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelInsertParams>(params, "rel.insert")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let pk = record_key_from_param(params.pk)?;
        let row = row_from_param(params.row)?;
        let col = ensure_record_collection_for_row(
            handle,
            &params.collection_name,
            params.collection,
            &row,
        )?;
        let layer = RelationalLayer::new(&handle.vault);
        if layer.get_record(&col, &pk)?.is_some() {
            return Err(storage_error(
                CALYX_LEAPABLE_REL_CONFLICT,
                "rel.insert refuses to overwrite an existing row",
                "use rel.update_row for existing rows",
            )
            .into());
        }
        let seq = layer.put_record(&col, &pk, &row)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "inserted",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "pk": key_value(&pk),
            "seq": seq,
            "row": row_value(&row),
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn rel_get(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelGetParams>(params, "rel.get")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Records)?;
        let pk = record_key_from_param(params.pk)?;
        let snapshot = params.snapshot.unwrap_or_else(|| handle.vault.snapshot());
        let row = RelationalLayer::new(&handle.vault).get_record_at(snapshot, &col, &pk)?;
        Ok(json!({
            "status": if row.is_some() { "found" } else { "absent" },
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "snapshot": snapshot,
            "pk": key_value(&pk),
            "row": row.as_ref().map(row_value)
        }))
    }

    fn rel_update_row(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelUpdateParams>(params, "rel.update_row")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Records)?;
        let pk = record_key_from_param(params.pk)?;
        let layer = RelationalLayer::new(&handle.vault);
        let Some(mut row) = layer.get_record(&col, &pk)? else {
            return Err(rel_not_found(&col.name, &pk).into());
        };
        for name in params.unset {
            row.fields.remove(&name);
        }
        for (name, value) in params.set {
            row.fields.insert(name, record_value_from_param(value)?);
        }
        let seq = layer.put_record(&col, &pk, &row)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "updated",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "pk": key_value(&pk),
            "seq": seq,
            "row": row_value(&row),
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn rel_delete(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelDeleteParams>(params, "rel.delete")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Records)?;
        let pk = record_key_from_param(params.pk)?;
        let Some(old_row) = RelationalLayer::new(&handle.vault).get_record(&col, &pk)? else {
            return Err(rel_not_found(&col.name, &pk).into());
        };
        let seq = write_rel_delete(handle, &col, &pk, &old_row)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "deleted",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "pk": key_value(&pk),
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn rel_scan(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelScanParams>(params, "rel.scan")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Records)?;
        let limit = validate_limit(params.limit)?;
        let snapshot = params.snapshot.unwrap_or_else(|| handle.vault.snapshot());
        let after_key = params
            .cursor
            .map(record_key_from_param)
            .transpose()?
            .map(|pk| record_key(&col, &pk))
            .transpose()?;
        let rows = handle.vault.scan_cf_range_page_at(
            snapshot,
            ColumnFamily::Relational,
            &rel_collection_range(&col),
            after_key.as_deref(),
            limit,
        )?;
        let mut items = Vec::with_capacity(rows.len());
        for (key, value) in &rows {
            let pk = rel_pk_from_full_key(&col, key)?;
            let row = decode_record_value(value)?;
            items.push(rel_row_json(&pk, &row));
        }
        let next_cursor = if rows.len() == limit {
            rows.last()
                .map(|(key, _)| rel_pk_from_full_key(&col, key))
                .transpose()?
                .map(|pk| key_value(&pk))
        } else {
            None
        };
        Ok(json!({
            "status": "scanned",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "snapshot": snapshot,
            "limit": limit,
            "next_cursor": next_cursor,
            "items": items
        }))
    }

    fn rel_query(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RelQueryParams>(params, "rel.query")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Records)?;
        let limit = validate_limit(params.limit)?;
        let snapshot = params.snapshot.unwrap_or_else(|| handle.vault.snapshot());
        let gte = params.gte.map(record_value_from_param).transpose()?;
        let lte = params.lte.map(record_value_from_param).transpose()?;
        let spec = runtime_btree_spec(&col, &params.index_name, gte.as_ref().or(lte.as_ref()))?;
        let pks = btree_range_at(
            &handle.vault,
            snapshot,
            &col,
            &spec,
            gte.as_ref(),
            lte.as_ref(),
            limit,
        )?;
        let layer = RelationalLayer::new(&handle.vault);
        let mut items = Vec::with_capacity(pks.len());
        for pk in pks {
            if let Some(row) = layer.get_record_at(snapshot, &col, &pk)? {
                items.push(rel_row_json(&pk, &row));
            }
        }
        Ok(json!({
            "status": "queried",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "index_name": spec.name,
            "snapshot": snapshot,
            "items": items
        }))
    }

    fn kv_set(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<KvSetParams>(params, "kv.set")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let key = bytes_from_param(params.key)?;
        let value = bytes_from_param(params.value)?;
        let value_len = value.len();
        let value_hash = blake3::hash(&value);
        let value_echo = params
            .echo_value
            .then(|| bytes_value(value.as_slice(), true));
        let ttl = params.ttl_ms.map(Duration::from_millis);
        let col = ensure_collection(
            handle,
            &params.collection_name,
            CollectionMode::KV,
            params.collection,
        )?;
        let seq = KvLayer::new(&handle.vault).kv_set(&col, params.ns, &key, &value, ttl)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "set",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "ns": params.ns,
            "key_hex": hex(&key),
            "value_len": value_len,
            "value_blake3": hex(value_hash.as_bytes()),
            "value": value_echo,
            "ttl_ms": params.ttl_ms,
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn kv_get(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<KvGetParams>(params, "kv.get")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::KV)?;
        let key = bytes_from_param(params.key)?;
        let value = KvLayer::new(&handle.vault).kv_get(&col, params.ns, &key)?;
        Ok(json!({
            "status": if value.is_some() { "found" } else { "absent" },
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "ns": params.ns,
            "key_hex": hex(&key),
            "value": value
                .as_ref()
                .map(|bytes| bytes_value(bytes.as_slice(), params.include_text))
        }))
    }

    fn kv_delete(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<KvDeleteParams>(params, "kv.delete")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::KV)?;
        let key = bytes_from_param(params.key)?;
        let seq = KvLayer::new(&handle.vault).kv_delete(&col, params.ns, &key)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "deleted",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "ns": params.ns,
            "key_hex": hex(&key),
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn ts_write(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<TsWriteParams>(params, "ts.write")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        validate_ts_value(params.value)?;
        let col = ensure_collection(
            handle,
            &params.collection_name,
            CollectionMode::TimeSeries,
            params.collection,
        )?;
        let seq = TimeSeriesLayer::new(&handle.vault).ts_write(
            &col,
            params.series,
            params.point_ts,
            params.value,
        )?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "written",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "series": params.series,
            "point_ts": params.point_ts,
            "value": params.value,
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn ts_range(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<TsRangeParams>(params, "ts.range")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::TimeSeries)?;
        let points = TimeSeriesLayer::new(&handle.vault).ts_range(
            &col,
            params.series,
            params.start_ts,
            params.end_ts,
        )?;
        Ok(json!({
            "status": "ranged",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "series": params.series,
            "points": points.into_iter().map(|(ts, value)| json!({"ts": ts, "value": value})).collect::<Vec<_>>()
        }))
    }

    fn blob_put(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<BlobPutParams>(params, "blob.put")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let data = bytes_from_param(params.input)?;
        let col = ensure_collection(
            handle,
            &params.collection_name,
            CollectionMode::Blob,
            params.collection,
        )?;
        let content_hash = blake3::hash(&data);
        let mut id = [0_u8; 16];
        id.copy_from_slice(&content_hash.as_bytes()[..16]);
        let blob_id = BlobId::from_bytes(id);
        let layer = BlobLayer::new(&handle.vault);
        let seq = layer.blob_put(&col, blob_id, &data)?;
        let manifest = layer
            .blob_manifest(&col, blob_id)?
            .expect("manifest just written");
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "put",
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "blob_id": hex(blob_id.as_bytes()),
            "content_hash": hex(content_hash.as_bytes()),
            "total_bytes": manifest.total_bytes,
            "chunk_count": manifest.chunk_count,
            "chunk_size": BLOB_CHUNK_SIZE,
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn blob_get(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<BlobGetParams>(params, "blob.get")?;
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let col = require_collection(handle, &params.collection_name, CollectionMode::Blob)?;
        let blob_id = blob_id_from_hex(&params.blob_id)?;
        let layer = BlobLayer::new(&handle.vault);
        let manifest = layer.blob_manifest(&col, blob_id)?;
        let data = if params.include_data && manifest.is_some() {
            layer.blob_get(&col, blob_id)?
        } else {
            None
        };
        Ok(json!({
            "status": if manifest.is_some() { "found" } else { "absent" },
            "vault_ref": handle.vault_ref.as_str(),
            "collection_name": col.name,
            "blob_id": params.blob_id,
            "manifest": manifest.map(blob_manifest_value),
            "data": data
                .as_ref()
                .map(|bytes| bytes_value(bytes.as_slice(), params.include_text))
        }))
    }

    fn txn_commit(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<TxnCommitParams>(params, "txn.commit")?;
        let flush_policy = self.config.flush_policy.clone();
        let handle = self.open_vault_for_storage(&params.vault_ref, params.ts)?;
        let txn_handle = handle.txn.clone();
        let mut txn = txn_handle.begin_on(
            &handle.vault,
            isolation_level(params.isolation),
            Some(params.cost_cap_ms),
            Duration::from_millis(params.timeout_ms),
        )?;
        let op_count = params.ops.len();
        for op in params.ops {
            stage_txn_op(handle, &mut txn, op)?;
        }
        let staged_rows = txn.batch_len();
        if params.inject_crash_after_stage {
            txn.rollback()?;
            return Err(storage_error(
                CALYX_LEAPABLE_TXN_INJECTED_CRASH,
                format!("injected crash after staging {staged_rows} rows"),
                "rerun without inject_crash_after_stage; staged writes were rolled back",
            )
            .into());
        }
        let seq = txn.commit(&handle.vault)?;
        handle.flush_after_write(&flush_policy)?;
        Ok(json!({
            "status": "committed",
            "vault_ref": handle.vault_ref.as_str(),
            "op_count": op_count,
            "staged_rows": staged_rows,
            "seq": seq,
            "latest_seq": handle.vault.latest_seq()
        }))
    }

    fn open_vault_for_storage(
        &mut self,
        vault_ref: &str,
        ts: Ts,
    ) -> EngineResult<&mut VaultHandle> {
        let vault_ref = VaultRef::parse(vault_ref)?;
        let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) else {
            return Err(vault_not_open(vault_ref.as_str()).into());
        };
        handle.touch(ts);
        Ok(handle)
    }
}

mod codec;
mod params;
