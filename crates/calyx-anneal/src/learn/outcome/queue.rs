use std::collections::BTreeMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use calyx_aster::cf::{ColumnFamily, OnlineKeyKind, online_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, CalyxError, Clock, CxId, Result, VaultStore};
use serde::{Deserialize, Serialize};

use super::{
    CALYX_ANNEAL_OUTCOME_APPEND_ONLY, OutcomeQueueEntry, OutcomeQueueReadback, invalid_row,
    validate_anchor, validate_entry, validate_entry_without_seq,
};
use crate::{CALYX_ASTER_CF_UNAVAILABLE, LogicalTime};

const OUTCOME_ROW_TAG: &str = "anneal_outcome_delta_j_v1";

pub trait OutcomeStorage: Send + Sync {
    fn put_anchor(&self, cx_id: CxId, anchor: &Anchor) -> Result<()>;
    fn put_queue_new(&self, seq: u64, value: &[u8]) -> Result<()>;
    fn scan_queue(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterOutcomeStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterOutcomeStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> OutcomeStorage for AsterOutcomeStorage<'_, C>
where
    C: Clock,
{
    fn put_anchor(&self, cx_id: CxId, anchor: &Anchor) -> Result<()> {
        self.vault.anchor(cx_id, anchor.clone())
    }

    fn put_queue_new(&self, seq: u64, value: &[u8]) -> Result<()> {
        let key = outcome_queue_key(seq);
        let existing = self
            .vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::Online, &key)
            .map_err(|error| cf_unavailable("read online CF", error))?;
        if existing.is_some() {
            return Err(append_only(seq));
        }
        self.vault
            .write_cf(ColumnFamily::Online, key, value.to_vec())
            .map(|_| ())
            .map_err(|error| cf_unavailable("write online CF", error))
    }

    fn scan_queue(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::Online)
            .map_err(|error| cf_unavailable("scan online CF", error))?
            .into_iter()
            .filter(|(key, _)| is_outcome_queue_key(key))
            .collect())
    }
}

pub struct OutcomeQueue<S> {
    storage: S,
    clock: Arc<dyn Clock>,
    state: RwLock<OutcomeQueueState>,
}

#[derive(Clone, Debug, Default)]
struct OutcomeQueueState {
    entries: BTreeMap<u64, OutcomeQueueEntry>,
    last_seq: u64,
}

impl<S> OutcomeQueue<S>
where
    S: OutcomeStorage,
{
    pub fn open(storage: S, clock: Arc<dyn Clock>) -> Result<Self> {
        let state = OutcomeQueueState::from_rows(storage.scan_queue()?)?;
        Ok(Self {
            storage,
            clock,
            state: RwLock::new(state),
        })
    }

    pub fn record_anchor(&self, cx_id: CxId, anchor: &Anchor) -> Result<()> {
        validate_anchor(anchor)?;
        self.storage.put_anchor(cx_id, anchor)
    }

    pub fn push(&self, mut entry: OutcomeQueueEntry) -> Result<OutcomeQueueEntry> {
        validate_entry_without_seq(&entry)?;
        let mut state = self.write_state()?;
        let seq = state
            .last_seq
            .checked_add(1)
            .ok_or_else(|| invalid_row("outcome queue sequence exhausted"))?;
        entry.seq = seq;
        entry.ts = self.clock.now();
        validate_entry(&entry)?;
        let value = encode_outcome_queue_entry(&entry)?;
        self.storage.put_queue_new(seq, &value)?;
        state.entries.insert(seq, entry.clone());
        state.last_seq = seq;
        Ok(entry)
    }

    pub fn readback_recent(&self, n: usize) -> Result<Vec<OutcomeQueueReadback>> {
        let mut rows = decode_readback_rows(self.storage.scan_queue()?)?;
        if n < rows.len() {
            rows.drain(0..rows.len() - n);
        }
        Ok(rows)
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self.read_state()?.entries.len())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.read_state()?.entries.is_empty())
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, OutcomeQueueState>> {
        self.state
            .read()
            .map_err(|_| CalyxError::backpressure("outcome queue lock poisoned"))
    }

    fn write_state(&self) -> Result<RwLockWriteGuard<'_, OutcomeQueueState>> {
        self.state
            .write()
            .map_err(|_| CalyxError::backpressure("outcome queue lock poisoned"))
    }
}

pub fn outcome_queue_key(seq: u64) -> Vec<u8> {
    online_key(OnlineKeyKind::DeltaJQueue, seq)
}

pub fn outcome_queue_seq_from_key(key: &[u8]) -> Result<u64> {
    if key.len() != 9 || key.first().copied() != Some(3) {
        return Err(invalid_row(format!(
            "outcome queue key has invalid shape: {} bytes",
            key.len()
        )));
    }
    let seq: [u8; 8] = key[1..]
        .try_into()
        .map_err(|_| invalid_row("outcome queue key missing sequence"))?;
    Ok(u64::from_be_bytes(seq))
}

pub fn encode_outcome_queue_entry(entry: &OutcomeQueueEntry) -> Result<Vec<u8>> {
    validate_entry(entry)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(
        &OutcomeQueueRow {
            tag: OUTCOME_ROW_TAG.to_string(),
            entry: entry.clone(),
        },
        &mut bytes,
    )
    .map_err(|error| invalid_row(format!("encode outcome queue row: {error}")))?;
    Ok(bytes)
}

pub fn decode_outcome_queue_entry(bytes: &[u8]) -> Result<OutcomeQueueEntry> {
    let row: OutcomeQueueRow = ciborium::de::from_reader(bytes)
        .map_err(|error| invalid_row(format!("decode outcome queue row: {error}")))?;
    if row.tag != OUTCOME_ROW_TAG {
        return Err(invalid_row("outcome queue row has invalid tag"));
    }
    validate_entry(&row.entry)?;
    Ok(row.entry)
}

impl OutcomeQueueState {
    fn from_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Self> {
        let mut entries = BTreeMap::new();
        let mut last_seq = 0;
        for (key, value) in rows {
            let seq = outcome_queue_seq_from_key(&key)?;
            let entry = decode_outcome_queue_entry(&value)?;
            if entry.seq != seq {
                return Err(invalid_row("outcome queue seq does not match key"));
            }
            if entries.insert(seq, entry).is_some() {
                return Err(invalid_row(format!("duplicate outcome queue seq {seq}")));
            }
            last_seq = last_seq.max(seq);
        }
        Ok(Self { entries, last_seq })
    }
}

#[derive(Serialize, Deserialize)]
struct OutcomeQueueRow {
    tag: String,
    entry: OutcomeQueueEntry,
}

fn decode_readback_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Vec<OutcomeQueueReadback>> {
    let mut readbacks = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        let seq = outcome_queue_seq_from_key(&key)?;
        let entry = decode_outcome_queue_entry(&value)?;
        readbacks.push(OutcomeQueueReadback {
            seq,
            key,
            value,
            entry,
        });
    }
    readbacks.sort_by_key(|row| row.seq);
    Ok(readbacks)
}

pub fn encode_ledger_value(anchor: &Anchor, observed: f64, surprise: f64) -> Result<Vec<u8>> {
    serde_json::to_vec(&(anchor, observed, surprise))
        .map_err(|error| invalid_row(format!("encode outcome ledger value: {error}")))
}

fn is_outcome_queue_key(key: &[u8]) -> bool {
    key.len() == 9 && key.first().copied() == Some(3)
}

fn append_only(seq: u64) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_OUTCOME_APPEND_ONLY,
        message: format!("outcome queue seq {seq} already exists"),
        remediation: "append outcome queue entries under a fresh monotonic sequence key",
    }
}

fn cf_unavailable(context: &str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "restore Aster online CF availability",
    }
}

pub fn logical_ts(anchor: &Anchor) -> LogicalTime {
    anchor.observed_at.max(1)
}
