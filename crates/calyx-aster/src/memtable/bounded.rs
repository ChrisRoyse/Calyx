use crate::sst::{self, SstSummary};
use calyx_core::{CalyxError, Result, Seq};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Logical per-row overhead used by memtable admission accounting.
pub const ENTRY_OVERHEAD_BYTES: usize = 4;

/// Backward-compatible name used by existing Aster callers.
pub type Memtable = BoundedMemtable;

/// A successful bounded memtable write acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteAck {
    pub seq: Seq,
    pub accepted_bytes: usize,
    pub used_bytes: usize,
    pub cap_bytes: usize,
    pub high_water_bytes: usize,
    pub flush_triggered: bool,
}

/// Snapshot of one memtable's byte accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemtableUsage {
    pub used_bytes: usize,
    pub cap_bytes: usize,
    pub high_water_bytes: usize,
    pub flush_triggered: bool,
}

/// In-memory ordered table with a hard byte cap and high-water flush signal.
#[derive(Debug)]
pub struct BoundedMemtable {
    entries: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    cap_bytes: usize,
    high_water_bytes: usize,
    used_bytes: AtomicUsize,
}

/// Immutable handoff created when a mutable memtable rotates.
#[derive(Debug, Clone)]
pub struct FrozenMemtable {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl FrozenMemtable {
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
    }

    pub fn flush_to_sst(&self, path: impl AsRef<Path>) -> Result<SstSummary> {
        sst::write_sst(path, self.iter())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl BoundedMemtable {
    /// Creates an empty memtable with the default 80% high-water threshold.
    pub fn new(cap_bytes: usize) -> Self {
        Self::with_high_water(cap_bytes, high_water_for(cap_bytes))
    }

    /// Creates an empty memtable with an explicit high-water threshold.
    pub fn with_high_water(cap_bytes: usize, high_water_bytes: usize) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            cap_bytes,
            high_water_bytes: high_water_bytes.min(cap_bytes),
            used_bytes: AtomicUsize::new(0),
        }
    }

    /// Estimates the bytes charged for one key/value row.
    pub fn entry_size(key: &[u8], value: &[u8]) -> usize {
        key.len()
            .saturating_add(value.len())
            .saturating_add(ENTRY_OVERHEAD_BYTES)
    }

    /// Inserts or replaces one key/value pair, failing closed at the byte cap.
    pub fn write(&self, key: &[u8], value: &[u8], seq: Seq) -> Result<WriteAck> {
        let accepted_bytes = Self::entry_size(key, value);
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CalyxError::backpressure("memtable lock poisoned"))?;
        let existing = entries
            .get(key)
            .map(|old| Self::entry_size(key, old))
            .unwrap_or(0);
        let current = self.used_bytes.load(Ordering::Acquire);
        let next_bytes = current
            .saturating_sub(existing)
            .saturating_add(accepted_bytes);
        if accepted_bytes > self.cap_bytes || next_bytes > self.cap_bytes {
            return Err(CalyxError::backpressure(format!(
                "memtable byte cap {} exceeded by projected {} bytes",
                self.cap_bytes, next_bytes
            )));
        }

        entries.insert(key.to_vec(), value.to_vec());
        self.used_bytes.store(next_bytes, Ordering::Release);
        Ok(WriteAck {
            seq,
            accepted_bytes,
            used_bytes: next_bytes,
            cap_bytes: self.cap_bytes,
            high_water_bytes: self.high_water_bytes,
            flush_triggered: self.flush_trigger_for(next_bytes),
        })
    }

    /// Compatibility wrapper for existing router call sites.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        self.write(key.as_ref(), value.as_ref(), 0).map(|_| ())
    }

    /// Returns true once the current byte use reaches the high-water threshold.
    pub fn flush_trigger(&self) -> bool {
        self.flush_trigger_for(self.used_bytes())
    }

    /// Decrements the usage counter after bytes are physically flushed.
    pub fn reset_after_flush(&self, flushed_bytes: usize) {
        let _ = self
            .used_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                Some(used.saturating_sub(flushed_bytes))
            });
    }

    /// Returns a value by key.
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        match self.entries.lock() {
            Ok(entries) => entries.get(key).cloned(),
            Err(poisoned) => poisoned.into_inner().get(key).cloned(),
        }
    }

    /// Returns cloned entries in key order.
    pub fn iter(&self) -> std::vec::IntoIter<(Vec<u8>, Vec<u8>)> {
        self.snapshot_entries().into_iter()
    }

    /// Flushes the current memtable snapshot into an immutable SSTable.
    pub fn flush_to_sst(&self, path: impl AsRef<Path>) -> Result<SstSummary> {
        let entries = self.snapshot_entries();
        sst::write_sst(
            path,
            entries
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice())),
        )
    }

    pub fn freeze(self) -> FrozenMemtable {
        let entries = self
            .entries
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        FrozenMemtable { entries }
    }

    pub fn needs_flush(&self) -> bool {
        self.flush_trigger()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |entries| entries.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes.load(Ordering::Acquire)
    }

    pub fn estimated_bytes(&self) -> usize {
        self.used_bytes()
    }

    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    pub fn byte_cap(&self) -> usize {
        self.cap_bytes()
    }

    pub fn high_water_bytes(&self) -> usize {
        self.high_water_bytes
    }

    pub fn usage(&self) -> MemtableUsage {
        let used_bytes = self.used_bytes();
        MemtableUsage {
            used_bytes,
            cap_bytes: self.cap_bytes,
            high_water_bytes: self.high_water_bytes,
            flush_triggered: self.flush_trigger_for(used_bytes),
        }
    }

    fn snapshot_entries(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn flush_trigger_for(&self, used_bytes: usize) -> bool {
        self.cap_bytes == 0 || used_bytes >= self.high_water_bytes
    }
}

fn high_water_for(cap_bytes: usize) -> usize {
    cap_bytes.saturating_mul(4) / 5
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn memtable_orders_keys_and_tracks_bytes() {
        let mut table = Memtable::new(64);

        table.put(b"k2", b"two").expect("put k2");
        table.put(b"k1", b"one").expect("put k1");

        let keys: Vec<_> = table.iter().map(|(key, _)| key).collect();
        assert_eq!(keys, [b"k1".to_vec(), b"k2".to_vec()]);
        assert_eq!(table.get(b"k2"), Some(b"two".to_vec()));
        assert_eq!(
            table.estimated_bytes(),
            Memtable::entry_size(b"k2", b"two") + Memtable::entry_size(b"k1", b"one")
        );
    }

    #[test]
    fn memtable_fails_closed_at_byte_cap() {
        let row_bytes = Memtable::entry_size(b"k1", b"one");
        let mut table = Memtable::new(row_bytes);

        table.put(b"k1", b"one").expect("first fits");
        let error = table.put(b"k2", b"two").expect_err("second exceeds cap");

        assert_eq!(error.code, "CALYX_BACKPRESSURE");
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(b"k2"), None);
    }

    #[test]
    fn write_ack_reports_high_water_flush_trigger() {
        let row = [0xA5; 12];
        let table = Memtable::with_high_water(64, Memtable::entry_size(b"k1", &row) * 2);

        let first = table.write(b"k1", &row, 7).expect("first fits");
        let second = table.write(b"k2", &row, 8).expect("second fits");

        assert_eq!(first.seq, 7);
        assert!(!first.flush_triggered);
        assert_eq!(second.seq, 8);
        assert!(second.flush_triggered);
        assert!(table.flush_trigger());
    }

    #[test]
    fn reset_after_flush_saturates_and_allows_subsequent_write() {
        let row = [0x11; 8];
        let table = Memtable::new(64);
        let written = table.write(b"k1", &row, 1).expect("write");

        table.reset_after_flush(written.accepted_bytes / 2);
        assert_eq!(
            table.used_bytes(),
            written.accepted_bytes - written.accepted_bytes / 2
        );
        table.reset_after_flush(usize::MAX);
        assert_eq!(table.used_bytes(), 0);
        table.write(b"k2", &row, 2).expect("write after reset");
    }

    #[test]
    fn freeze_hands_off_sorted_entries_and_flushes() {
        let dir = test_dir("freeze");
        let path = dir.join("frozen.sst");
        let mut table = Memtable::new(64);
        table.put(b"k2", b"two").expect("put k2");
        table.put(b"k1", b"one").expect("put k1");
        let before = table.iter().collect::<Vec<_>>();

        let frozen = table.freeze();
        frozen.flush_to_sst(&path).expect("flush frozen");
        let after = frozen
            .iter()
            .map(|(key, value)| (key.to_vec(), value.to_vec()))
            .collect::<Vec<_>>();

        assert_eq!(frozen.len(), 2);
        assert_eq!(before, after);
        assert!(fs::metadata(path).unwrap().len() > 0);
        cleanup(dir);
    }

    #[test]
    fn zero_cap_and_oversized_single_write_are_fail_closed() {
        let frozen = Memtable::new(8).freeze();
        assert!(frozen.is_empty());

        let zero = Memtable::new(0);
        let error = zero.write(b"k", b"v", 1).expect_err("zero cap rejects");
        assert_eq!(error.code, "CALYX_BACKPRESSURE");
        assert!(zero.flush_trigger());

        let oversized = Memtable::new(Memtable::entry_size(b"k", b"v") - 1);
        let error = oversized
            .write(b"k", b"v", 1)
            .expect_err("single row larger than cap");
        assert_eq!(error.code, "CALYX_BACKPRESSURE");
    }

    #[test]
    fn concurrent_writes_never_exceed_cap() {
        let row = [0x33; 8];
        let row_bytes = Memtable::entry_size(b"k000", &row);
        let table = Arc::new(Memtable::new(row_bytes * 8));
        let mut handles = Vec::new();
        for thread_id in 0..8u8 {
            let table = Arc::clone(&table);
            handles.push(thread::spawn(move || {
                for idx in 0..8u8 {
                    let key = [b'k', b'0' + thread_id, b'0' + idx, b'x'];
                    let result = table.write(&key, &row, idx as u64);
                    if let Err(error) = result {
                        assert_eq!(error.code, "CALYX_BACKPRESSURE");
                    }
                    assert!(table.used_bytes() <= table.cap_bytes());
                }
            }));
        }
        for handle in handles {
            handle.join().expect("thread joins");
        }
        assert!(table.used_bytes() <= table.cap_bytes());
    }

    proptest! {
        #[test]
        fn successful_puts_never_exceed_byte_cap(
            cap in 1024usize..=1_048_576usize,
            pairs in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 1..64), proptest::collection::vec(any::<u8>(), 0..64)),
                0..256
            )
        ) {
            let table = Memtable::new(cap);
            for (seq, (key, value)) in pairs.into_iter().enumerate() {
                let result = table.write(&key, &value, seq as u64);
                if let Err(error) = result {
                    prop_assert_eq!(error.code, "CALYX_BACKPRESSURE");
                }
                prop_assert!(table.used_bytes() <= table.cap_bytes());
            }
        }

        #[test]
        fn freeze_preserves_sorted_iteration(pairs in proptest::collection::vec((proptest::collection::vec(any::<u8>(), 1..8), proptest::collection::vec(any::<u8>(), 0..8)), 0..32)) {
            let mut table = Memtable::new(1024);
            for (key, value) in pairs {
                let _ = table.put(&key, &value);
            }
            let before = table.iter().collect::<Vec<_>>();
            let frozen = table.freeze();
            let after = frozen.iter().map(|(key, value)| (key.to_vec(), value.to_vec())).collect::<Vec<_>>();
            prop_assert_eq!(before, after);
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-memtable-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).expect("cleanup test dir");
    }
}
