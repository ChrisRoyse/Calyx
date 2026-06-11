use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, Result};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use super::{MistakeEntry, MistakeLog, MistakeRef, MistakeStorage};
use crate::LogicalTime;
use crate::ledger_anneal::CALYX_ASTER_CF_UNAVAILABLE;

pub const DEFAULT_REPLAY_CAPACITY: usize = 4096;
pub const CALYX_ANNEAL_INVALID_CAPACITY: &str = "CALYX_ANNEAL_INVALID_CAPACITY";
pub const CALYX_ANNEAL_REPLAY_INVALID_ROW: &str = "CALYX_ANNEAL_REPLAY_INVALID_ROW";

const REPLAY_SNAPSHOT_TAG: &str = "anneal_replay_snapshot_v1";
const REPLAY_SNAPSHOT_KEY: &[u8] = b"snapshot/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayEntry {
    pub cx_id: CxId,
    pub surprise: f64,
    pub mistake_ref: MistakeRef,
    pub added_ts: LogicalTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplaySnapshot {
    pub capacity: usize,
    pub entries: Vec<ReplayEntry>,
}

#[derive(Serialize, Deserialize)]
struct ReplaySnapshotRow {
    tag: String,
    snapshot: ReplaySnapshot,
}

pub trait ReplayStorage: Send + Sync {
    fn load_snapshot(&self) -> Result<Option<Vec<u8>>>;
    fn save_snapshot(&self, value: &[u8]) -> Result<()>;
}

pub struct AsterReplayStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterReplayStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> ReplayStorage for AsterReplayStorage<'_, C>
where
    C: Clock,
{
    fn load_snapshot(&self) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(
                self.vault.latest_seq(),
                ColumnFamily::AnnealReplay,
                &replay_snapshot_key(),
            )
            .map_err(|error| cf_unavailable("read anneal_replay CF", error))
    }

    fn save_snapshot(&self, value: &[u8]) -> Result<()> {
        self.vault
            .write_cf(
                ColumnFamily::AnnealReplay,
                replay_snapshot_key(),
                value.to_vec(),
            )
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_replay CF", error))
    }
}

pub struct ReplayBuffer<S> {
    heap: BinaryHeap<ReplayEntry>,
    capacity: usize,
    clock: Arc<dyn Clock>,
    storage: S,
}

impl<S> ReplayBuffer<S>
where
    S: ReplayStorage,
{
    pub fn open(storage: S, capacity: usize, clock: Arc<dyn Clock>) -> Result<Self> {
        validate_capacity(capacity)?;
        let heap = match storage.load_snapshot()? {
            Some(bytes) => heap_from_entries(decode_replay_snapshot(&bytes)?.entries, capacity)?,
            None => BinaryHeap::new(),
        };
        Ok(Self {
            heap,
            capacity,
            clock,
            storage,
        })
    }

    pub fn open_default(storage: S, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open(storage, DEFAULT_REPLAY_CAPACITY, clock)
    }

    pub fn push(&mut self, entry: ReplayEntry) -> Result<bool> {
        validate_entry(&entry)?;
        let mut next_heap = self.heap.clone();
        let accepted = push_into_heap(&mut next_heap, self.capacity, entry)?;
        if accepted {
            let value = encode_snapshot_heap(self.capacity, &next_heap)?;
            self.storage.save_snapshot(&value)?;
            self.heap = next_heap;
        }
        Ok(accepted)
    }

    pub fn sample_batch(&self, n: usize, seed: u64) -> Vec<ReplayEntry> {
        let mut candidates = self.entries_by_priority();
        if n >= candidates.len() {
            return candidates;
        }
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut sampled = Vec::with_capacity(n);
        while sampled.len() < n && !candidates.is_empty() {
            let total: f64 = candidates.iter().map(|entry| entry.surprise).sum();
            let index = if total > 0.0 {
                weighted_index(&candidates, rng.gen_range(0.0..total))
            } else {
                0
            };
            sampled.push(candidates.remove(index));
        }
        sampled
    }

    pub fn seed_from_log<M>(&mut self, log: &MistakeLog<M>, n: usize) -> Result<usize>
    where
        M: MistakeStorage,
    {
        let mut accepted = 0;
        for row in log.readback_recent(n)? {
            let entry = ReplayEntry::from_mistake(row.seq, &row.entry)?;
            if self.push(entry)? {
                accepted += 1;
            }
        }
        Ok(accepted)
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn entries_by_priority(&self) -> Vec<ReplayEntry> {
        let mut entries = self.heap.clone().into_vec();
        entries.sort_by(|left, right| right.cmp(left));
        entries
    }

    pub fn top_surprises(&self, n: usize) -> Vec<f64> {
        self.entries_by_priority()
            .into_iter()
            .take(n)
            .map(|entry| entry.surprise)
            .collect()
    }

    pub fn snapshot(&self) -> ReplaySnapshot {
        ReplaySnapshot {
            capacity: self.capacity,
            entries: self.entries_by_priority(),
        }
    }

    pub fn entry(
        &self,
        cx_id: CxId,
        surprise: f64,
        mistake_ref: MistakeRef,
    ) -> Result<ReplayEntry> {
        ReplayEntry::new(cx_id, surprise, mistake_ref, self.clock.now())
    }
}

impl ReplayEntry {
    pub fn new(
        cx_id: CxId,
        surprise: f64,
        mistake_ref: MistakeRef,
        added_ts: LogicalTime,
    ) -> Result<Self> {
        let entry = Self {
            cx_id,
            surprise,
            mistake_ref,
            added_ts,
        };
        validate_entry(&entry)?;
        Ok(entry)
    }

    pub fn from_mistake(seq: u64, entry: &MistakeEntry) -> Result<Self> {
        Self::new(
            entry.cx_id,
            entry.surprise,
            MistakeRef {
                seq,
                surprise: entry.surprise,
            },
            entry.ts,
        )
    }
}

impl PartialEq for ReplayEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cx_id == other.cx_id
            && self.surprise.to_bits() == other.surprise.to_bits()
            && self.mistake_ref.seq == other.mistake_ref.seq
            && self.mistake_ref.surprise.to_bits() == other.mistake_ref.surprise.to_bits()
            && self.added_ts == other.added_ts
    }
}

impl Eq for ReplayEntry {}

impl PartialOrd for ReplayEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReplayEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.surprise
            .total_cmp(&other.surprise)
            .then_with(|| other.added_ts.cmp(&self.added_ts))
            .then_with(|| other.mistake_ref.seq.cmp(&self.mistake_ref.seq))
            .then_with(|| self.cx_id.as_bytes().cmp(other.cx_id.as_bytes()))
    }
}

pub fn replay_snapshot_key() -> Vec<u8> {
    REPLAY_SNAPSHOT_KEY.to_vec()
}

pub fn encode_replay_snapshot(snapshot: &ReplaySnapshot) -> Result<Vec<u8>> {
    validate_capacity(snapshot.capacity)?;
    for entry in &snapshot.entries {
        validate_entry(entry)?;
    }
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(
        &ReplaySnapshotRow {
            tag: REPLAY_SNAPSHOT_TAG.to_string(),
            snapshot: snapshot.clone(),
        },
        &mut bytes,
    )
    .map_err(|error| invalid_row(format!("encode anneal_replay snapshot: {error}")))?;
    Ok(bytes)
}

pub fn decode_replay_snapshot(bytes: &[u8]) -> Result<ReplaySnapshot> {
    let row: ReplaySnapshotRow = ciborium::de::from_reader(bytes)
        .map_err(|error| invalid_row(format!("decode anneal_replay snapshot: {error}")))?;
    if row.tag != REPLAY_SNAPSHOT_TAG {
        return Err(invalid_row("anneal_replay snapshot has invalid tag"));
    }
    validate_capacity(row.snapshot.capacity)?;
    for entry in &row.snapshot.entries {
        validate_entry(entry)?;
    }
    Ok(row.snapshot)
}

fn encode_snapshot_heap(capacity: usize, heap: &BinaryHeap<ReplayEntry>) -> Result<Vec<u8>> {
    encode_replay_snapshot(&ReplaySnapshot {
        capacity,
        entries: sorted_entries(heap),
    })
}

fn heap_from_entries(
    entries: Vec<ReplayEntry>,
    capacity: usize,
) -> Result<BinaryHeap<ReplayEntry>> {
    let mut heap = BinaryHeap::new();
    for entry in entries {
        validate_entry(&entry)?;
        push_into_heap(&mut heap, capacity, entry)?;
    }
    Ok(heap)
}

fn push_into_heap(
    heap: &mut BinaryHeap<ReplayEntry>,
    capacity: usize,
    entry: ReplayEntry,
) -> Result<bool> {
    validate_capacity(capacity)?;
    if heap.len() < capacity {
        heap.push(entry);
        return Ok(true);
    }
    let Some((min_index, min_entry)) = heap
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| left.cmp(right))
    else {
        return Ok(false);
    };
    if entry.surprise.total_cmp(&min_entry.surprise) != Ordering::Greater {
        return Ok(false);
    }
    let mut entries = heap.clone().into_vec();
    entries.swap_remove(min_index);
    entries.push(entry);
    *heap = BinaryHeap::from(entries);
    Ok(true)
}

fn sorted_entries(heap: &BinaryHeap<ReplayEntry>) -> Vec<ReplayEntry> {
    let mut entries = heap.clone().into_vec();
    entries.sort_by(|left, right| right.cmp(left));
    entries
}

fn weighted_index(entries: &[ReplayEntry], draw: f64) -> usize {
    let mut cumulative = 0.0;
    for (index, entry) in entries.iter().enumerate() {
        cumulative += entry.surprise;
        if draw < cumulative {
            return index;
        }
    }
    entries.len().saturating_sub(1)
}

fn validate_capacity(capacity: usize) -> Result<()> {
    if capacity == 0 {
        return Err(CalyxError {
            code: CALYX_ANNEAL_INVALID_CAPACITY,
            message: "replay buffer capacity must be > 0".to_string(),
            remediation: "configure a positive anneal replay capacity",
        });
    }
    Ok(())
}

fn validate_entry(entry: &ReplayEntry) -> Result<()> {
    if !entry.surprise.is_finite() || entry.surprise < 0.0 {
        return Err(invalid_row("replay surprise must be finite and >= 0"));
    }
    if !entry.mistake_ref.surprise.is_finite() || entry.mistake_ref.surprise < 0.0 {
        return Err(invalid_row(
            "replay mistake_ref surprise must be finite and >= 0",
        ));
    }
    if entry.mistake_ref.seq == 0 {
        return Err(invalid_row("replay mistake_ref seq must be > 0"));
    }
    if entry.surprise.to_bits() != entry.mistake_ref.surprise.to_bits() {
        return Err(invalid_row("replay surprise must match mistake_ref"));
    }
    Ok(())
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_REPLAY_INVALID_ROW,
        message: message.into(),
        remediation: "repair or quarantine anneal_replay CF snapshot before learning",
    }
}

fn cf_unavailable(context: &str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "restore Aster anneal_replay CF availability",
    }
}
