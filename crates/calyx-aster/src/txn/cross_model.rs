use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Clock, Constellation, Result, Seq};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use serde_json::Value;

use crate::cf::{ColumnFamily, base_key};
use crate::collection::{Collection, CollectionMode};
use crate::index::{IndexMaintenance, collection_has_maintained_index};
use crate::layers::blob::{self, BLOB_CHUNK_SIZE, BlobId};
use crate::layers::document::{self, DocId};
use crate::layers::kv;
use crate::layers::relational::{
    RecordKey, Row, decode_record_value, encode_record_value, record_key,
};
use crate::layers::timeseries::{self, RollupWindow};
use crate::vault::AsterVault;
use crate::vault::encode::WriteRow;

use super::validation::{
    expires_at, invalid_argument, reject_lens_collection, require_mode, validate_row,
};
use super::{CALYX_TXN_COST_CAP, IsolationLevel, TxnHandle, txn_error};

const EMPTY_TXN_KEY_PREFIX: &[u8] = b"txn\0empty\0";

pub struct CrossModelTxn<'h> {
    handle: &'h TxnHandle,
    batch: Vec<WriteRow>,
    snapshot_seq: Seq,
    isolation: IsolationLevel,
    started_at: Instant,
    cost_cap_ms: Option<u32>,
    completed: bool,
}

impl<'h> CrossModelTxn<'h> {
    pub(crate) fn new(
        handle: &'h TxnHandle,
        isolation: IsolationLevel,
        cost_cap_ms: Option<u32>,
        snapshot_seq: Seq,
        started_at: Instant,
    ) -> Self {
        Self {
            handle,
            batch: Vec::new(),
            snapshot_seq,
            isolation,
            started_at,
            cost_cap_ms,
            completed: false,
        }
    }

    pub fn snapshot_seq(&self) -> Seq {
        self.snapshot_seq
    }

    pub fn batch_len(&self) -> usize {
        self.batch.len()
    }

    pub fn isolation(&self) -> IsolationLevel {
        self.isolation
    }

    pub fn put_record<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        col: &Collection,
        pk: &RecordKey,
        row: &Row,
    ) -> Result<()> {
        self.prepare_write(vault)?;
        reject_lens_collection(col)?;
        require_mode(col, CollectionMode::Records, "relational")?;
        validate_row(col, row)?;
        let key = record_key(col, pk)?;
        let value = encode_record_value(row)?;
        let old_row = self
            .staged_or_snapshot_cf(vault, ColumnFamily::Relational, &key)?
            .map(|bytes| decode_record_value(&bytes))
            .transpose()?;
        let mut rows = vec![(ColumnFamily::Relational, key, value)];
        IndexMaintenance::stage_put(vault, &mut rows, col, pk, old_row.as_ref(), row)?;
        self.extend_rows(rows);
        Ok(())
    }

    pub fn put_doc<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        col: &Collection,
        doc_id: DocId,
        doc: &Value,
    ) -> Result<()> {
        self.prepare_write(vault)?;
        reject_lens_collection(col)?;
        let rows = document::stage_doc_rows_at(vault, self.snapshot_seq, col, doc_id, doc)?;
        self.extend_rows(rows);
        Ok(())
    }

    pub fn kv_set<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        col: &Collection,
        ns: u64,
        key: &[u8],
        val: &[u8],
        ttl: Option<Duration>,
    ) -> Result<()> {
        self.prepare_write(vault)?;
        reject_lens_collection(col)?;
        require_mode(col, CollectionMode::KV, "kv")?;
        kv::validate_user_key(key)?;
        kv::validate_payload(val)?;
        let expires_at = expires_at(vault.clock_now(), ttl)?;
        let full_key = kv::kv_key(col, ns, key);
        let value = kv::encode_value(expires_at, val);
        let pk = RecordKey::from_bytes(full_key.clone())?;
        let mut rows = vec![(ColumnFamily::Kv, full_key.clone(), value)];
        if collection_has_maintained_index(col) {
            let old_index_row = self
                .staged_or_snapshot_kv_value_at(vault, col, ns, key)?
                .map(|old| kv::kv_index_row(col, ns, key, &old))
                .transpose()?;
            let new_index_row = kv::kv_index_row(col, ns, key, val)?;
            IndexMaintenance::stage_put(
                vault,
                &mut rows,
                col,
                &pk,
                old_index_row.as_ref(),
                &new_index_row,
            )?;
        }
        self.extend_rows(rows);
        Ok(())
    }

    pub fn ts_write<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        col: &Collection,
        series: u64,
        ts: u64,
        val: f64,
    ) -> Result<()> {
        self.prepare_write(vault)?;
        reject_lens_collection(col)?;
        require_mode(col, CollectionMode::TimeSeries, "time-series")?;
        if !val.is_finite() {
            return Err(invalid_argument(
                "time-series value must be finite (NaN/inf rejected)",
            ));
        }
        let point_key = timeseries::point_key(col, series, ts);
        let pk = RecordKey::from_bytes(point_key.clone())?;
        let mut rows = vec![(
            ColumnFamily::TimeSeries,
            point_key.clone(),
            timeseries::encode_point(val),
        )];
        for window in RollupWindow::ALL {
            let key = timeseries::rollup_key(col, series, window, window.window_start(ts));
            let current = self
                .staged_or_snapshot_cf(vault, ColumnFamily::TimeSeries, &key)?
                .map(|bytes| timeseries::decode_rollup(&bytes))
                .transpose()?;
            let updated = timeseries::fold_rollup(current, val);
            rows.push((
                ColumnFamily::TimeSeries,
                key,
                timeseries::encode_rollup(&updated),
            ));
        }
        if collection_has_maintained_index(col) {
            let old_index_row = self
                .staged_or_snapshot_cf(vault, ColumnFamily::TimeSeries, &point_key)?
                .map(|bytes| {
                    timeseries::decode_point(&bytes)
                        .and_then(|old| timeseries::ts_index_row(col, series, ts, old))
                })
                .transpose()?;
            let new_index_row = timeseries::ts_index_row(col, series, ts, val)?;
            IndexMaintenance::stage_put(
                vault,
                &mut rows,
                col,
                &pk,
                old_index_row.as_ref(),
                &new_index_row,
            )?;
        }
        self.extend_rows(rows);
        Ok(())
    }

    pub fn blob_put_chunk<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        col: &Collection,
        blob_id: BlobId,
        idx: u32,
        chunk: &[u8],
    ) -> Result<()> {
        self.prepare_write(vault)?;
        reject_lens_collection(col)?;
        require_mode(col, CollectionMode::Blob, "blob")?;
        if chunk.len() > BLOB_CHUNK_SIZE {
            return Err(invalid_argument(format!(
                "blob chunk exceeds {BLOB_CHUNK_SIZE} bytes"
            )));
        }
        self.batch.push(WriteRow {
            cf: ColumnFamily::Blob,
            key: blob::chunk_key(col, blob_id, idx),
            value: chunk.to_vec(),
        });
        Ok(())
    }

    pub fn put_constellation<C: Clock>(
        &mut self,
        vault: &AsterVault<C>,
        constellation: &Constellation,
    ) -> Result<()> {
        self.prepare_write(vault)?;
        if constellation.vault_id != self.handle.vault_id() {
            return Err(CalyxError::vault_access_denied(
                "constellation belongs to another vault",
            ));
        }
        let key = base_key(constellation.cx_id);
        if vault
            .read_cf_at(self.snapshot_seq, ColumnFamily::Base, &key)?
            .is_some()
        {
            return Err(CalyxError::aster_corrupt_shard(
                "cross-model txn cannot overwrite an existing constellation",
            ));
        }
        let mut rows = Vec::new();
        vault.stage_constellation_rows(&mut rows, constellation)?;
        self.batch.extend(rows);
        Ok(())
    }

    pub fn get_record<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        col: &Collection,
        pk: &RecordKey,
    ) -> Result<Option<Row>> {
        require_mode(col, CollectionMode::Records, "relational")?;
        let key = record_key(col, pk)?;
        self.read_cf(vault, ColumnFamily::Relational, &key)?
            .map(|bytes| decode_record_value(&bytes))
            .transpose()
    }

    pub fn kv_get<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        col: &Collection,
        ns: u64,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        require_mode(col, CollectionMode::KV, "kv")?;
        kv::validate_user_key(key)?;
        self.kv_value_at(vault, col, ns, key)
    }

    pub fn read_cf<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.handle.verify_vault(vault)?;
        if self.isolation == IsolationLevel::Serializable
            && let Some(row) = self
                .batch
                .iter()
                .rev()
                .find(|row| row.cf == cf && row.key.as_slice() == key)
        {
            return Ok(Some(row.value.clone()));
        }
        let snapshot = if self.isolation == IsolationLevel::ReadCommitted || self.snapshot_seq == 0
        {
            vault.latest_seq()
        } else {
            self.snapshot_seq
        };
        vault.read_cf_at(snapshot, cf, key)
    }

    pub fn commit<C: Clock>(mut self, vault: &AsterVault<C>) -> Result<Seq> {
        self.handle.verify_vault(vault)?;
        self.ensure_snapshot(vault);
        if self.cost_cap_ms.is_none() && !self.batch.is_empty() {
            return self.fail(CALYX_TXN_COST_CAP, "cross-model writes require cost_cap_ms");
        }
        if let Some(cap) = self.cost_cap_ms {
            let cap = Duration::from_millis(cap as u64);
            if self.started_at.elapsed() > cap {
                return self.fail(CALYX_TXN_COST_CAP, "transaction exceeded its cost cap");
            }
        }
        if self.batch.is_empty() {
            self.stage_empty_marker(vault);
        }
        let payload = self.ledger_payload();
        let subject = self.ledger_subject();
        let rows = self
            .batch
            .iter()
            .map(|row| (row.cf, row.key.clone(), row.value.clone()));
        let result = vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-cross-model-txn".to_string()),
        );
        self.finish();
        result
    }

    pub fn rollback(mut self) -> Result<()> {
        self.batch.clear();
        self.finish();
        Ok(())
    }

    fn prepare_write<C: Clock>(&mut self, vault: &AsterVault<C>) -> Result<()> {
        self.handle.verify_vault(vault)?;
        self.ensure_snapshot(vault);
        Ok(())
    }

    fn ensure_snapshot<C: Clock>(&mut self, vault: &AsterVault<C>) {
        if self.snapshot_seq == 0 {
            self.snapshot_seq = vault.latest_seq();
        }
    }

    fn extend_rows(&mut self, rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>) {
        self.batch.extend(
            rows.into_iter()
                .map(|(cf, key, value)| WriteRow { cf, key, value }),
        );
    }

    fn kv_value_at<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        col: &Collection,
        ns: u64,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        let full_key = kv::kv_key(col, ns, key);
        let Some(bytes) = self.read_cf(vault, ColumnFamily::Kv, &full_key)? else {
            return Ok(None);
        };
        let (expires_at, payload) = kv::decode_value(&bytes)?;
        if kv::is_expired(expires_at, vault.clock_now()) {
            return Ok(None);
        }
        Ok(Some(payload.to_vec()))
    }

    fn staged_or_snapshot_kv_value_at<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        col: &Collection,
        ns: u64,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        let full_key = kv::kv_key(col, ns, key);
        let Some(bytes) = self.staged_or_snapshot_cf(vault, ColumnFamily::Kv, &full_key)? else {
            return Ok(None);
        };
        let (expires_at, payload) = kv::decode_value(&bytes)?;
        if kv::is_expired(expires_at, vault.clock_now()) {
            return Ok(None);
        }
        Ok(Some(payload.to_vec()))
    }

    fn staged_or_snapshot_cf<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.handle.verify_vault(vault)?;
        if let Some(row) = self
            .batch
            .iter()
            .rev()
            .find(|row| row.cf == cf && row.key.as_slice() == key)
        {
            return Ok(Some(row.value.clone()));
        }
        let snapshot = if self.snapshot_seq == 0 {
            vault.latest_seq()
        } else {
            self.snapshot_seq
        };
        vault.read_cf_at(snapshot, cf, key)
    }

    fn stage_empty_marker<C: Clock>(&mut self, vault: &AsterVault<C>) {
        let mut key = EMPTY_TXN_KEY_PREFIX.to_vec();
        key.extend_from_slice(&self.handle.vault_id().as_ulid().to_bytes());
        key.extend_from_slice(&self.snapshot_seq.to_be_bytes());
        key.extend_from_slice(&vault.clock_now().to_be_bytes());
        self.batch.push(WriteRow {
            cf: ColumnFamily::Online,
            key,
            value: b"cross-model-empty-txn-v1".to_vec(),
        });
    }

    fn ledger_subject(&self) -> SubjectId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"calyx:txn:cross-model:v1");
        hasher.update(&self.handle.vault_id().as_ulid().to_bytes());
        hasher.update(&self.snapshot_seq.to_be_bytes());
        for row in &self.batch {
            hasher.update(&row.cf.keyspace_tag());
            hasher.update(&(row.key.len() as u64).to_be_bytes());
            hasher.update(&row.key);
            hasher.update(&(row.value.len() as u64).to_be_bytes());
            hasher.update(&row.value);
        }
        SubjectId::Query(hasher.finalize().as_bytes().to_vec())
    }

    fn ledger_payload(&self) -> Vec<u8> {
        let mut payload = PayloadBuilder::default();
        payload
            .insert_str("vault_id", self.handle.vault_id().to_string())
            .insert_str("snapshot_seq", self.snapshot_seq.to_string())
            .insert_str("isolation", format!("{:?}", self.isolation))
            .insert_str("row_count", self.batch.len().to_string())
            .insert_str("batch_hash", self.batch_hash());
        RedactionPolicy::default().apply_to_payload(&payload)
    }

    fn batch_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for row in &self.batch {
            hasher.update(&row.cf.keyspace_tag());
            hasher.update(&(row.key.len() as u64).to_be_bytes());
            hasher.update(&row.key);
            hasher.update(&(row.value.len() as u64).to_be_bytes());
            hasher.update(&row.value);
        }
        hasher.finalize().to_hex().to_string()
    }

    fn fail<T>(&mut self, code: &'static str, message: &'static str) -> Result<T> {
        self.finish();
        Err(txn_error(
            code,
            message,
            "rollback has discarded the staged batch",
        ))
    }

    fn finish(&mut self) {
        if !self.completed {
            self.completed = true;
            self.handle.release();
        }
    }
}

impl Drop for CrossModelTxn<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.batch.clear();
            self.finish();
        }
    }
}
