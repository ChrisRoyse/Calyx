//! Append-only ledger writer and row-store adapters.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Clock, LedgerRef, Result};

use crate::codec::{decode, encode};
use crate::entry::{ActorId, HASH_BYTES, LedgerEntry, SubjectId};
use crate::head_anchor::{LedgerHeadAnchor, verify_recovered_tip};
use crate::kind::EntryKind;
use crate::redaction::RedactionPolicy;

pub use crate::directory_store::DirectoryLedgerStore;

/// Physical ledger row keyed by big-endian sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedLedgerEntry {
    entry: LedgerEntry,
    bytes: Vec<u8>,
}

impl PreparedLedgerEntry {
    pub const fn seq(&self) -> u64 {
        self.entry.seq
    }

    pub const fn entry_hash(&self) -> [u8; HASH_BYTES] {
        self.entry.entry_hash
    }

    pub const fn prev_hash(&self) -> [u8; HASH_BYTES] {
        self.entry.prev_hash
    }

    pub const fn ts(&self) -> u64 {
        self.entry.ts
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn ledger_ref(&self) -> LedgerRef {
        LedgerRef {
            seq: self.entry.seq,
            hash: self.entry.entry_hash,
        }
    }
}

/// Minimal append-only `ledger` CF contract used by `LedgerAppender`.
pub trait LedgerCfStore {
    /// Returns all rows sorted by sequence number.
    fn scan(&self) -> Result<Vec<LedgerRow>>;

    /// Writes a new row. Implementations must reject overwrites.
    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()>;

    /// Rejects delete paths for the ledger CF.
    fn delete(&mut self, seq: u64) -> Result<()> {
        reject_delete(seq)
    }

    /// Rejects tombstone paths for the ledger CF.
    fn tombstone(&mut self, seq: u64) -> Result<()> {
        reject_tombstone(seq)
    }

    /// External monotonic head witness, if this store has one.
    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        Ok(None)
    }

    /// Persists the newest committed head witness.
    fn put_head_anchor(&mut self, _anchor: &LedgerHeadAnchor) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreparePosition {
    seq: u64,
    prev_hash: [u8; HASH_BYTES],
    last_ts: u64,
}

/// The single write path for the hash-chained append-only ledger.
#[derive(Debug)]
pub struct LedgerAppender<S, C> {
    store: S,
    clock: C,
    next_seq: u64,
    prev_hash: [u8; HASH_BYTES],
    last_ts: u64,
    redaction_policy: RedactionPolicy,
}

impl<S, C> LedgerAppender<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    /// Opens an appender and recovers its tip from existing ledger rows.
    pub fn open(store: S, clock: C) -> Result<Self> {
        Self::open_with_policy(store, clock, RedactionPolicy::default())
    }

    /// Opens an appender with an explicit redaction policy.
    pub fn open_with_policy(store: S, clock: C, redaction_policy: RedactionPolicy) -> Result<Self> {
        let (next_seq, prev_hash, last_ts) = recover_tip(&store)?;
        Ok(Self {
            store,
            clock,
            next_seq,
            prev_hash,
            last_ts,
            redaction_policy,
        })
    }

    /// Appends one chained entry and returns its provenance reference.
    pub fn append(
        &mut self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let prepared = self.prepare(kind, subject, payload, actor)?;
        self.commit_prepared(&prepared)
    }

    /// Builds the next ledger row without mutating the store or appender tip.
    pub fn prepare(
        &self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<PreparedLedgerEntry> {
        self.prepare_at(
            PreparePosition {
                seq: self.next_seq,
                prev_hash: self.prev_hash,
                last_ts: self.last_ts,
            },
            kind,
            subject,
            payload,
            actor,
        )
    }

    /// Builds the row that must follow an uncommitted staged ledger row.
    pub fn prepare_after(
        &self,
        predecessor: &PreparedLedgerEntry,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<PreparedLedgerEntry> {
        let seq = predecessor
            .seq()
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
        self.prepare_at(
            PreparePosition {
                seq,
                prev_hash: predecessor.entry_hash(),
                last_ts: predecessor.ts(),
            },
            kind,
            subject,
            payload,
            actor,
        )
    }

    fn prepare_at(
        &self,
        position: PreparePosition,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<PreparedLedgerEntry> {
        self.redaction_policy.check_payload_with_policy(&payload)?;
        self.verify_tip()?;
        actor.validate()?;
        let actor = self.redaction_policy.apply_to_actor(actor);
        actor.validate()?;
        let ts = self.next_ts_after(position.last_ts)?;
        let entry = LedgerEntry::new(
            position.seq,
            position.prev_hash,
            kind,
            subject,
            payload,
            actor,
            ts,
        );
        let bytes = encode(&entry);
        Ok(PreparedLedgerEntry { entry, bytes })
    }

    /// Commits a previously prepared row and advances the recovered tip.
    pub fn commit_prepared(&mut self, prepared: &PreparedLedgerEntry) -> Result<LedgerRef> {
        if prepared.entry.seq != self.next_seq || prepared.entry.prev_hash != self.prev_hash {
            return Err(CalyxError::ledger_chain_broken(format!(
                "prepared ledger seq {} does not match appender next_seq {}",
                prepared.entry.seq, self.next_seq
            )));
        }
        self.store.put_new(prepared.entry.seq, prepared.bytes())?;
        let anchor = LedgerHeadAnchor::new(
            prepared.entry.seq.saturating_add(1),
            prepared.entry.entry_hash,
        )?;
        self.store.put_head_anchor(&anchor)?;
        self.last_ts = prepared.entry.ts;
        self.next_seq = prepared
            .entry
            .seq
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
        self.prev_hash = prepared.entry.entry_hash;
        Ok(prepared.ledger_ref())
    }

    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub const fn prev_hash(&self) -> [u8; HASH_BYTES] {
        self.prev_hash
    }

    pub const fn last_ts(&self) -> u64 {
        self.last_ts
    }

    pub fn scan_entries(&self) -> Result<Vec<LedgerEntry>> {
        self.store
            .scan()?
            .into_iter()
            .map(|row| decode(&row.bytes))
            .collect()
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    fn verify_tip(&self) -> Result<()> {
        let (next_seq, prev_hash, last_ts) = recover_tip(&self.store)?;
        if next_seq == self.next_seq && prev_hash == self.prev_hash && last_ts == self.last_ts {
            return Ok(());
        }
        Err(CalyxError::ledger_chain_broken(format!(
            "ledger tip changed: appender expected next_seq {}, store has {}",
            self.next_seq, next_seq
        )))
    }

    fn next_ts_after(&self, last_ts: u64) -> Result<u64> {
        let clock_ts = self.clock.now();
        Ok(if clock_ts <= last_ts {
            last_ts
                .checked_add(1)
                .ok_or_else(|| CalyxError::ledger_chain_broken("ledger timestamp exhausted"))?
        } else {
            clock_ts
        })
    }
}

/// In-memory row store for deterministic tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryLedgerStore {
    rows: BTreeMap<u64, Vec<u8>>,
    anchor: Option<LedgerHeadAnchor>,
}

impl MemoryLedgerStore {
    pub fn insert_raw(&mut self, seq: u64, bytes: Vec<u8>) {
        self.rows.insert(seq, bytes);
    }

    pub fn remove_raw(&mut self, seq: u64) {
        self.rows.remove(&seq);
    }
}

impl LedgerCfStore for MemoryLedgerStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(self
            .rows
            .iter()
            .map(|(seq, bytes)| LedgerRow {
                seq: *seq,
                bytes: bytes.clone(),
            })
            .collect())
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        if self.rows.contains_key(&seq) {
            return Err(append_only_violation(format!(
                "ledger seq {seq} already exists"
            )));
        }
        self.rows.insert(seq, bytes.to_vec());
        Ok(())
    }

    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        Ok(self.anchor.clone())
    }

    fn put_head_anchor(&mut self, anchor: &LedgerHeadAnchor) -> Result<()> {
        if let Some(current) = &self.anchor
            && anchor.height < current.height
        {
            return Err(append_only_violation(format!(
                "ledger head anchor regressed from {} to {}",
                current.height, anchor.height
            )));
        }
        self.anchor = Some(anchor.clone());
        Ok(())
    }
}

pub fn reject_delete(seq: u64) -> Result<()> {
    Err(append_only_violation(format!(
        "delete forbidden for ledger seq {seq}"
    )))
}

pub fn reject_tombstone(seq: u64) -> Result<()> {
    Err(append_only_violation(format!(
        "tombstone forbidden for ledger seq {seq}"
    )))
}

fn recover_tip(store: &impl LedgerCfStore) -> Result<(u64, [u8; HASH_BYTES], u64)> {
    let mut next_seq = 0_u64;
    let mut prev_hash = [0_u8; HASH_BYTES];
    let mut last_ts = 0_u64;
    for row in store.scan()? {
        if row.seq != next_seq {
            return Err(CalyxError::ledger_chain_broken(format!(
                "ledger seq gap: expected {}, found {}",
                next_seq, row.seq
            )));
        }
        let entry = decode(&row.bytes)?;
        if entry.seq != row.seq {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger key seq {} != encoded seq {}",
                row.seq, entry.seq
            )));
        }
        if entry.prev_hash != prev_hash {
            return Err(CalyxError::ledger_chain_broken(format!(
                "ledger seq {} prev_hash does not match prior entry",
                row.seq
            )));
        }
        prev_hash = entry.entry_hash;
        last_ts = entry.ts;
        next_seq = next_seq
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
    }
    let anchor = store.head_anchor()?;
    verify_recovered_tip(anchor.as_ref(), next_seq, prev_hash)?;
    Ok((next_seq, prev_hash, last_ts))
}

fn append_only_violation(message: impl Into<String>) -> CalyxError {
    CalyxError::ledger_append_only_violation(message)
}

#[cfg(test)]
mod tests;
