use std::collections::BTreeMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

use crate::LogicalTime;
use crate::ledger_anneal::CALYX_ASTER_CF_UNAVAILABLE;

pub const DEFAULT_MISTAKE_SURPRISE_THRESHOLD: f64 = 0.3;
pub const CALYX_ANNEAL_INVALID_WINDOW: &str = "CALYX_ANNEAL_INVALID_WINDOW";
pub const CALYX_ANNEAL_MISTAKE_INVALID_ROW: &str = "CALYX_ANNEAL_MISTAKE_INVALID_ROW";
pub const CALYX_ANNEAL_MISTAKE_APPEND_ONLY: &str = "CALYX_ANNEAL_MISTAKE_APPEND_ONLY";

const MISTAKE_ROW_TAG: &str = "anneal_mistake_v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeEntry {
    pub cx_id: CxId,
    pub predicted: f64,
    pub observed: f64,
    pub anchor: AnchorKind,
    pub ts: LogicalTime,
    pub surprise: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeRef {
    pub seq: u64,
    pub surprise: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeReadback {
    pub seq: u64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub entry: MistakeEntry,
}

pub trait MistakeStorage: Send + Sync {
    fn put_new(&self, seq: u64, value: &[u8]) -> Result<()>;
    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterMistakeStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterMistakeStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> MistakeStorage for AsterMistakeStorage<'_, C>
where
    C: Clock,
{
    fn put_new(&self, seq: u64, value: &[u8]) -> Result<()> {
        let key = mistake_key(seq);
        let existing = self
            .vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealMistakes, &key)
            .map_err(|error| cf_unavailable("read anneal_mistakes CF", error))?;
        if existing.is_some() {
            return Err(append_only(seq));
        }
        self.vault
            .write_cf(ColumnFamily::AnnealMistakes, key, value.to_vec())
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_mistakes CF", error))
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealMistakes)
            .map_err(|error| cf_unavailable("scan anneal_mistakes CF", error))
    }
}

pub struct MistakeLog<S> {
    storage: S,
    window_size: usize,
    high_surprise_threshold: f64,
    clock: Arc<dyn Clock>,
    state: RwLock<MistakeState>,
}

#[derive(Clone, Debug, Default)]
struct MistakeState {
    entries: BTreeMap<u64, MistakeEntry>,
    last_seq: u64,
}

impl<S> MistakeLog<S>
where
    S: MistakeStorage,
{
    pub fn open(storage: S, window_size: usize, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open_with_threshold(
            storage,
            window_size,
            DEFAULT_MISTAKE_SURPRISE_THRESHOLD,
            clock,
        )
    }

    pub fn open_with_threshold(
        storage: S,
        window_size: usize,
        high_surprise_threshold: f64,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        if window_size == 0 {
            return Err(invalid_window("MistakeLog window_size must be > 0"));
        }
        if !high_surprise_threshold.is_finite() || high_surprise_threshold < 0.0 {
            return Err(invalid_row(
                "mistake surprise threshold must be finite and >= 0",
            ));
        }
        let state = MistakeState::from_rows(storage.scan()?)?;
        Ok(Self {
            storage,
            window_size,
            high_surprise_threshold,
            clock,
            state: RwLock::new(state),
        })
    }

    pub fn append(
        &self,
        cx_id: CxId,
        predicted: f64,
        observed: f64,
        anchor: AnchorKind,
    ) -> Result<MistakeRef> {
        validate_observation(predicted, observed)?;
        let surprise = (predicted - observed).abs();
        let entry = MistakeEntry {
            cx_id,
            predicted,
            observed,
            anchor,
            ts: self.clock.now(),
            surprise,
        };
        let value = encode_mistake_entry(&entry)?;
        let mut state = self.write_state()?;
        let seq = state
            .last_seq
            .checked_add(1)
            .ok_or_else(|| invalid_row("mistake sequence exhausted"))?;
        state.entries.insert(seq, entry);
        state.last_seq = seq;
        self.storage.put_new(seq, &value)?;
        Ok(MistakeRef { seq, surprise })
    }

    pub fn mistake_rate(&self, window: usize) -> Result<f64> {
        if window == 0 {
            return Err(invalid_window("mistake_rate window must be > 0"));
        }
        let state = self.read_state()?;
        if state.entries.is_empty() {
            return Ok(0.0);
        }
        let high_surprise = last_entries(&state.entries, window)
            .iter()
            .filter(|entry| entry.surprise > self.high_surprise_threshold)
            .count();
        Ok(high_surprise as f64 / window as f64)
    }

    pub fn mistake_rate_default_window(&self) -> Result<f64> {
        self.mistake_rate(self.window_size)
    }

    pub fn recent(&self, n: usize) -> Result<Vec<MistakeEntry>> {
        Ok(last_entries(&self.read_state()?.entries, n))
    }

    pub fn get(&self, seq: u64) -> Result<Option<MistakeEntry>> {
        Ok(self.read_state()?.entries.get(&seq).cloned())
    }

    pub fn readback_recent(&self, n: usize) -> Result<Vec<MistakeReadback>> {
        let mut rows = decode_readback_rows(self.storage.scan()?)?;
        if n < rows.len() {
            rows.drain(0..rows.len() - n);
        }
        Ok(rows)
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, MistakeState>> {
        self.state
            .read()
            .map_err(|_| CalyxError::backpressure("mistake log state lock poisoned"))
    }

    fn write_state(&self) -> Result<RwLockWriteGuard<'_, MistakeState>> {
        self.state
            .write()
            .map_err(|_| CalyxError::backpressure("mistake log state lock poisoned"))
    }
}

impl MistakeState {
    fn from_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Self> {
        let mut entries = BTreeMap::new();
        let mut last_seq = 0;
        for (key, value) in rows {
            let seq = mistake_seq_from_key(&key)?;
            let entry = decode_mistake_entry(&value)?;
            if entries.insert(seq, entry).is_some() {
                return Err(invalid_row(format!("duplicate anneal_mistakes seq {seq}")));
            }
            last_seq = last_seq.max(seq);
        }
        Ok(Self { entries, last_seq })
    }
}

#[derive(Serialize, Deserialize)]
struct MistakeRow {
    tag: String,
    entry: MistakeEntry,
}

pub fn mistake_key(seq: u64) -> Vec<u8> {
    seq.to_be_bytes().to_vec()
}

pub fn mistake_seq_from_key(key: &[u8]) -> Result<u64> {
    let key: [u8; 8] = key
        .try_into()
        .map_err(|_| invalid_row(format!("mistake key has {} bytes, expected 8", key.len())))?;
    Ok(u64::from_be_bytes(key))
}

pub fn encode_mistake_entry(entry: &MistakeEntry) -> Result<Vec<u8>> {
    validate_entry(entry)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(
        &MistakeRow {
            tag: MISTAKE_ROW_TAG.to_string(),
            entry: entry.clone(),
        },
        &mut bytes,
    )
    .map_err(|error| invalid_row(format!("encode anneal_mistakes row: {error}")))?;
    Ok(bytes)
}

pub fn decode_mistake_entry(bytes: &[u8]) -> Result<MistakeEntry> {
    let row: MistakeRow = ciborium::de::from_reader(bytes)
        .map_err(|error| invalid_row(format!("decode anneal_mistakes row: {error}")))?;
    if row.tag != MISTAKE_ROW_TAG {
        return Err(invalid_row("anneal_mistakes row has invalid tag"));
    }
    validate_entry(&row.entry)?;
    Ok(row.entry)
}

fn decode_readback_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Vec<MistakeReadback>> {
    let mut readbacks = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        let seq = mistake_seq_from_key(&key)?;
        let entry = decode_mistake_entry(&value)?;
        readbacks.push(MistakeReadback {
            seq,
            key,
            value,
            entry,
        });
    }
    readbacks.sort_by_key(|row| row.seq);
    Ok(readbacks)
}

fn last_entries(entries: &BTreeMap<u64, MistakeEntry>, n: usize) -> Vec<MistakeEntry> {
    let skip = entries.len().saturating_sub(n);
    entries.values().skip(skip).cloned().collect::<Vec<_>>()
}

fn validate_observation(predicted: f64, observed: f64) -> Result<()> {
    if !predicted.is_finite() || !observed.is_finite() {
        return Err(invalid_row(
            "mistake predicted and observed values must be finite",
        ));
    }
    Ok(())
}

fn validate_entry(entry: &MistakeEntry) -> Result<()> {
    validate_observation(entry.predicted, entry.observed)?;
    if !entry.surprise.is_finite() {
        return Err(invalid_row("mistake surprise must be finite"));
    }
    let expected = (entry.predicted - entry.observed).abs();
    if entry.surprise.to_bits() != expected.to_bits() {
        return Err(invalid_row(format!(
            "mistake surprise {} does not match expected {expected}",
            entry.surprise
        )));
    }
    Ok(())
}

fn invalid_window(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_INVALID_WINDOW,
        message: message.into(),
        remediation: "use a non-zero mistake-rate window",
    }
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_MISTAKE_INVALID_ROW,
        message: message.into(),
        remediation: "repair or quarantine anneal_mistakes CF rows before learning",
    }
}

fn append_only(seq: u64) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_MISTAKE_APPEND_ONLY,
        message: format!("anneal_mistakes seq {seq} already exists"),
        remediation: "append Anneal mistakes under a fresh monotonic sequence key",
    }
}

fn cf_unavailable(context: &str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "restore Aster anneal_mistakes CF availability",
    }
}
