//! Append-only ledger writer and row-store adapters.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Clock, LedgerRef, Result};

use crate::codec::{decode, encode};
use crate::entry::{ActorId, HASH_BYTES, LedgerEntry, SubjectId};
use crate::kind::EntryKind;
use crate::redaction::RedactionPolicy;

const ROW_EXT: &str = "ledger";

/// Physical ledger row keyed by big-endian sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub seq: u64,
    pub bytes: Vec<u8>,
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
        self.redaction_policy.check_payload_with_policy(&payload)?;
        self.verify_tip()?;
        let seq = self.next_seq;
        actor.validate()?;
        let actor = self.redaction_policy.apply_to_actor(actor);
        actor.validate()?;
        let ts = self.next_ts()?;
        let entry = LedgerEntry::new(seq, self.prev_hash, kind, subject, payload, actor, ts);
        self.store.put_new(seq, &encode(&entry))?;
        self.last_ts = ts;
        self.next_seq = seq
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
        self.prev_hash = entry.entry_hash;
        Ok(LedgerRef {
            seq,
            hash: entry.entry_hash,
        })
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

    fn next_ts(&self) -> Result<u64> {
        let clock_ts = self.clock.now();
        Ok(if clock_ts <= self.last_ts {
            self.last_ts
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
}

impl MemoryLedgerStore {
    pub fn insert_raw(&mut self, seq: u64, bytes: Vec<u8>) {
        self.rows.insert(seq, bytes);
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
}

/// Disk-backed row store used for manual FSV before Aster group-commit wiring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryLedgerStore {
    root: PathBuf,
}

impl DirectoryLedgerStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|error| CalyxError::disk_pressure(format!("create ledger CF dir: {error}")))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn row_path(&self, seq: u64) -> PathBuf {
        self.root.join(format!("{seq:016x}.{ROW_EXT}"))
    }
}

impl LedgerCfStore for DirectoryLedgerStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|error| CalyxError::disk_pressure(format!("read ledger CF dir: {error}")))?
        {
            let path = entry
                .map_err(|error| {
                    CalyxError::disk_pressure(format!("read ledger CF entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some(ROW_EXT) {
                continue;
            }
            let seq = parse_row_seq(&path)?;
            let bytes = fs::read(&path)
                .map_err(|error| CalyxError::disk_pressure(format!("read ledger row: {error}")))?;
            rows.push(LedgerRow { seq, bytes });
        }
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        let path = self.row_path(seq);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => {
                    append_only_violation(format!("ledger row {} already exists", path.display()))
                }
                _ => CalyxError::disk_pressure(format!("create ledger row: {error}")),
            })?;
        file.write_all(bytes)
            .map_err(|error| CalyxError::disk_pressure(format!("write ledger row: {error}")))?;
        file.sync_all()
            .map_err(|error| CalyxError::disk_pressure(format!("sync ledger row: {error}")))?;
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
    Ok((next_seq, prev_hash, last_ts))
}

fn parse_row_seq(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CalyxError::ledger_corrupt("ledger row has invalid file name"))?;
    u64::from_str_radix(stem, 16)
        .map_err(|error| CalyxError::ledger_corrupt(format!("ledger row seq parse: {error}")))
}

fn append_only_violation(message: impl Into<String>) -> CalyxError {
    CalyxError::ledger_append_only_violation(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{Clock, CxId};
    use proptest::prelude::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[test]
    fn appender_clamps_repeated_clock_values() {
        let mut appender = sample_appender([1000, 1000, 1001]);

        append_sample(&mut appender, 1).unwrap();
        append_sample(&mut appender, 2).unwrap();
        append_sample(&mut appender, 3).unwrap();

        let ts = entry_ts(&appender);
        assert_eq!(ts, vec![1000, 1001, 1002]);
        assert_eq!(appender.last_ts(), 1002);
    }

    #[test]
    fn appender_recovers_last_ts_across_restart() {
        let mut first = sample_appender([5000]);
        append_sample(&mut first, 1).unwrap();
        let store = first.into_store();

        let mut reopened =
            LedgerAppender::open(store, SequenceClock::new([4999])).expect("reopen appender");
        append_sample(&mut reopened, 2).unwrap();

        assert_eq!(entry_ts(&reopened), vec![5000, 5001]);
        assert_eq!(reopened.last_ts(), 5001);
    }

    #[test]
    fn actor_length_edges_fail_closed() {
        assert!(ActorId::Agent(String::new()).validate().is_ok());
        assert!(ActorId::Agent("x".repeat(64)).validate().is_ok());
        assert_eq!(
            ActorId::Agent("x".repeat(65)).validate().unwrap_err().code,
            "CALYX_LEDGER_ACTOR_TOO_LONG"
        );

        let mut appender = sample_appender([1]);
        let error = appender
            .append(
                EntryKind::Ingest,
                sample_subject(1),
                b"{}".to_vec(),
                ActorId::Agent("x".repeat(65)),
            )
            .unwrap_err();
        assert_eq!(error.code, "CALYX_LEDGER_ACTOR_TOO_LONG");
        assert!(appender.scan_entries().unwrap().is_empty());
    }

    #[test]
    fn recovered_zero_ts_still_clamps_forward() {
        let mut store = MemoryLedgerStore::default();
        let entry = LedgerEntry::new(
            0,
            [0; HASH_BYTES],
            EntryKind::Ingest,
            sample_subject(1),
            b"{}".to_vec(),
            ActorId::Service("svc".to_string()),
            0,
        );
        store.insert_raw(0, encode(&entry));

        let mut reopened =
            LedgerAppender::open(store, SequenceClock::new([0])).expect("reopen appender");
        append_sample(&mut reopened, 2).unwrap();

        assert_eq!(entry_ts(&reopened), vec![0, 1]);
    }

    proptest! {
        #[test]
        fn appender_timestamps_are_monotone_for_any_clock_values(
            values in proptest::collection::vec(0_u64..(u64::MAX - 32), 1..16),
        ) {
            let mut appender = LedgerAppender::open(
                MemoryLedgerStore::default(),
                SequenceClock::new(values.clone()),
            ).expect("open appender");

            for index in 0..values.len() {
                append_sample(&mut appender, index as u8).unwrap();
            }

            let ts = entry_ts(&appender);
            prop_assert!(ts.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }

    fn sample_appender<const N: usize>(
        values: [u64; N],
    ) -> LedgerAppender<MemoryLedgerStore, SequenceClock> {
        LedgerAppender::open(MemoryLedgerStore::default(), SequenceClock::new(values))
            .expect("open appender")
    }

    fn append_sample(
        appender: &mut LedgerAppender<MemoryLedgerStore, SequenceClock>,
        seed: u8,
    ) -> Result<LedgerRef> {
        appender.append(
            EntryKind::Ingest,
            sample_subject(seed),
            b"{}".to_vec(),
            ActorId::Service("svc".to_string()),
        )
    }

    fn sample_subject(seed: u8) -> SubjectId {
        SubjectId::Cx(CxId::from_bytes([seed; 16]))
    }

    fn entry_ts<C: Clock>(appender: &LedgerAppender<MemoryLedgerStore, C>) -> Vec<u64> {
        appender
            .scan_entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.ts)
            .collect()
    }

    #[derive(Debug)]
    struct SequenceClock {
        values: Mutex<VecDeque<u64>>,
        fallback: u64,
    }

    impl SequenceClock {
        fn new(values: impl IntoIterator<Item = u64>) -> Self {
            let values = values.into_iter().collect::<VecDeque<_>>();
            let fallback = values.back().copied().unwrap_or(0);
            Self {
                values: Mutex::new(values),
                fallback,
            }
        }
    }

    impl Clock for SequenceClock {
        fn now(&self) -> u64 {
            self.values
                .lock()
                .expect("clock lock")
                .pop_front()
                .unwrap_or(self.fallback)
        }
    }
}
