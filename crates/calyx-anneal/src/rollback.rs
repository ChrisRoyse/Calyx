use std::collections::HashMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result, Seq, Ts};
use serde::{Deserialize, Serialize};

pub use crate::rollback_codec::{rollback_live_key, rollback_snapshot_key};

use crate::rollback_codec::{
    CHANGE_PREFIX, LIVE_PREFIX, decode_live_key, decode_live_value, decode_snapshot_value,
    encode_live_value, snapshot_row,
};

pub const CALYX_ANNEAL_UNKNOWN_CHANGE_ID: &str = "CALYX_ANNEAL_UNKNOWN_CHANGE_ID";
pub const CALYX_ANNEAL_CHANGE_COMMITTED: &str = "CALYX_ANNEAL_CHANGE_COMMITTED";
pub const CALYX_ANNEAL_INVALID_ROLLBACK_STATE: &str = "CALYX_ANNEAL_INVALID_ROLLBACK_STATE";

const ID_BUCKET: u64 = 1_000_000;

pub type LogicalTime = Ts;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChangeId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKey {
    ConfigCache([u8; 32]),
    HnswGraph([u8; 32]),
    QuantLevel([u8; 32]),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPtr {
    ConfigCacheKeyHash([u8; 32]),
    HnswGraphPath(String),
    QuantLevelRecordHash([u8; 32]),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSnapshot {
    pub change_id: ChangeId,
    pub key: ArtifactKey,
    pub prior_ptr: ArtifactPtr,
    pub candidate_ptr: ArtifactPtr,
    pub ts: LogicalTime,
    pub description: String,
    pub promoted: bool,
    pub reverted: bool,
    pub committed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackReadback {
    pub snapshot: ArtifactSnapshot,
    pub live_ptr: ArtifactPtr,
    pub snapshot_key: Vec<u8>,
    pub snapshot_bytes: Vec<u8>,
    pub live_key: Vec<u8>,
    pub live_bytes: Vec<u8>,
}

pub trait RollbackStorage: Send + Sync {
    fn put_many(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Seq>;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterRollbackStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterRollbackStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> RollbackStorage for AsterRollbackStorage<'_, C>
where
    C: Clock,
{
    fn put_many(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Seq> {
        self.vault.write_cf_batch(
            rows.into_iter()
                .map(|(key, value)| (ColumnFamily::AnnealRollback, key, value)),
        )
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealRollback, key)
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealRollback)
    }
}

pub struct RollbackStore<'a, S> {
    clock: &'a dyn Clock,
    storage: S,
    state: RwLock<RollbackState>,
}

#[derive(Clone, Debug)]
struct RollbackState {
    snapshots: HashMap<ChangeId, ArtifactSnapshot>,
    live_ptrs: HashMap<ArtifactKey, ArtifactPtr>,
    seed: u64,
    counter: u64,
    last_id: u64,
}

impl<'a, S> RollbackStore<'a, S>
where
    S: RollbackStorage,
{
    pub fn open(clock: &'a dyn Clock, seed: u64, storage: S) -> Result<Self> {
        let mut state = RollbackState::new(seed);
        for (key, value) in storage.scan()? {
            if key.starts_with(CHANGE_PREFIX) {
                let snapshot = decode_snapshot_value(&value)?;
                state.last_id = state.last_id.max(snapshot.change_id.0);
                state.snapshots.insert(snapshot.change_id, snapshot);
            } else if key.starts_with(LIVE_PREFIX) {
                let artifact_key = decode_live_key(&key)?;
                let ptr = decode_live_value(&value)?;
                state.live_ptrs.insert(artifact_key, ptr);
            }
        }
        Ok(Self {
            clock,
            storage,
            state: RwLock::new(state),
        })
    }

    pub fn install_live_ptr(&self, key: ArtifactKey, ptr: ArtifactPtr) -> Result<()> {
        self.storage
            .put_many(vec![(rollback_live_key(&key), encode_live_value(&ptr)?)])?;
        self.write_state()?.live_ptrs.insert(key, ptr);
        Ok(())
    }

    pub fn prepare(&self, key: ArtifactKey, candidate_ptr: ArtifactPtr) -> Result<ChangeId> {
        self.prepare_with_description(key, candidate_ptr, "")
    }

    pub fn prepare_with_description(
        &self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        description: impl Into<String>,
    ) -> Result<ChangeId> {
        let mut state = self.write_state()?;
        let prior_ptr = state
            .live_ptrs
            .get(&key)
            .cloned()
            .ok_or_else(|| invalid_state("prepare requires an existing live pointer"))?;
        let change_id = state.allocate_id(self.clock.now())?;
        let snapshot = ArtifactSnapshot {
            change_id,
            key,
            prior_ptr,
            candidate_ptr,
            ts: self.clock.now(),
            description: description.into(),
            promoted: false,
            reverted: false,
            committed: false,
        };
        self.storage.put_many(vec![snapshot_row(&snapshot)?])?;
        state.snapshots.insert(change_id, snapshot);
        Ok(change_id)
    }

    pub fn promote(&self, change_id: ChangeId) -> Result<()> {
        let mut state = self.write_state()?;
        let mut snapshot = state.snapshot(change_id)?.clone();
        if snapshot.committed {
            return Err(change_committed(change_id));
        }
        if snapshot.reverted {
            return Err(invalid_state("cannot promote a reverted rollback snapshot"));
        }
        snapshot.promoted = true;
        let rows = vec![
            (
                rollback_live_key(&snapshot.key),
                encode_live_value(&snapshot.candidate_ptr)?,
            ),
            snapshot_row(&snapshot)?,
        ];
        self.storage.put_many(rows)?;
        state
            .live_ptrs
            .insert(snapshot.key.clone(), snapshot.candidate_ptr.clone());
        state.snapshots.insert(change_id, snapshot);
        Ok(())
    }

    pub fn rollback(&self, change_id: ChangeId) -> Result<()> {
        let mut state = self.write_state()?;
        let mut snapshot = state.snapshot(change_id)?.clone();
        if snapshot.committed {
            return Err(change_committed(change_id));
        }
        snapshot.reverted = true;
        let rows = vec![
            (
                rollback_live_key(&snapshot.key),
                encode_live_value(&snapshot.prior_ptr)?,
            ),
            snapshot_row(&snapshot)?,
        ];
        self.storage.put_many(rows)?;
        state
            .live_ptrs
            .insert(snapshot.key.clone(), snapshot.prior_ptr.clone());
        state.snapshots.insert(change_id, snapshot);
        Ok(())
    }

    pub fn commit(&self, change_id: ChangeId) -> Result<()> {
        let mut state = self.write_state()?;
        let mut snapshot = state.snapshot(change_id)?.clone();
        snapshot.committed = true;
        self.storage.put_many(vec![snapshot_row(&snapshot)?])?;
        state.snapshots.insert(change_id, snapshot);
        Ok(())
    }

    pub fn live_ptr(&self, key: &ArtifactKey) -> Result<Option<ArtifactPtr>> {
        Ok(self.read_state()?.live_ptrs.get(key).cloned())
    }

    pub fn snapshot(&self, change_id: ChangeId) -> Result<Option<ArtifactSnapshot>> {
        Ok(self.read_state()?.snapshots.get(&change_id).cloned())
    }

    pub fn readback(&self, change_id: ChangeId) -> Result<RollbackReadback> {
        let snapshot_key = rollback_snapshot_key(change_id);
        let snapshot_bytes = self
            .storage
            .get(&snapshot_key)?
            .ok_or_else(|| unknown_change(change_id))?;
        let snapshot = decode_snapshot_value(&snapshot_bytes)?;
        let live_key = rollback_live_key(&snapshot.key);
        let live_bytes = self
            .storage
            .get(&live_key)?
            .ok_or_else(|| invalid_state("rollback live pointer row is missing"))?;
        let live_ptr = decode_live_value(&live_bytes)?;
        Ok(RollbackReadback {
            snapshot,
            live_ptr,
            snapshot_key,
            snapshot_bytes,
            live_key,
            live_bytes,
        })
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, RollbackState>> {
        self.state
            .read()
            .map_err(|_| CalyxError::backpressure("rollback state lock poisoned"))
    }

    fn write_state(&self) -> Result<RwLockWriteGuard<'_, RollbackState>> {
        self.state
            .write()
            .map_err(|_| CalyxError::backpressure("rollback state lock poisoned"))
    }
}

impl RollbackState {
    fn new(seed: u64) -> Self {
        Self {
            snapshots: HashMap::new(),
            live_ptrs: HashMap::new(),
            seed,
            counter: 0,
            last_id: 0,
        }
    }

    fn allocate_id(&mut self, ts: Ts) -> Result<ChangeId> {
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| invalid_state("rollback counter exhausted"))?;
        let time_base = ts.saturating_mul(ID_BUCKET);
        let candidate = time_base
            .saturating_add(self.seed % ID_BUCKET)
            .saturating_add(self.counter);
        let monotonic = self
            .last_id
            .checked_add(1)
            .ok_or_else(|| invalid_state("rollback change id exhausted"))?;
        let next = candidate.max(monotonic);
        self.last_id = next;
        Ok(ChangeId(next))
    }

    fn snapshot(&self, change_id: ChangeId) -> Result<&ArtifactSnapshot> {
        self.snapshots
            .get(&change_id)
            .ok_or_else(|| unknown_change(change_id))
    }
}

fn unknown_change(change_id: ChangeId) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_UNKNOWN_CHANGE_ID,
        message: format!("unknown Anneal rollback change_id {}", change_id.0),
        remediation: "read anneal_rollback CF and use a prepared change_id",
    }
}

fn change_committed(change_id: ChangeId) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_CHANGE_COMMITTED,
        message: format!("Anneal rollback change_id {} is committed", change_id.0),
        remediation: "open a new Anneal change instead of reverting committed state",
    }
}

fn invalid_state(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_INVALID_ROLLBACK_STATE,
        message: message.into(),
        remediation: "repair anneal_rollback CF rows before continuing Anneal",
    }
}
