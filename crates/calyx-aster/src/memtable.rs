//! Bounded ordered memtable for Aster writes.

use crate::sst::{self, SstSummary};
use calyx_core::{CalyxError, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// In-memory ordered table with a byte high-water mark.
#[derive(Debug, Clone)]
pub struct Memtable {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    byte_cap: usize,
    estimated_bytes: usize,
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

impl Memtable {
    /// Creates an empty memtable.
    pub fn new(byte_cap: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            byte_cap,
            estimated_bytes: 0,
        }
    }

    /// Inserts or replaces one key/value pair, failing closed at the byte cap.
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Result<()> {
        let key = key.as_ref();
        let value = value.as_ref();
        let existing = self.entries.get(key).map(|old| key.len() + old.len());
        let next_bytes = self.estimated_bytes - existing.unwrap_or(0) + key.len() + value.len();
        if next_bytes > self.byte_cap {
            return Err(CalyxError::backpressure(format!(
                "memtable byte cap {} exceeded by projected {} bytes",
                self.byte_cap, next_bytes
            )));
        }

        self.entries.insert(key.to_vec(), value.to_vec());
        self.estimated_bytes = next_bytes;
        Ok(())
    }

    /// Returns a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// Returns entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
    }

    /// Flushes the memtable into an immutable SSTable.
    pub fn flush_to_sst(&self, path: impl AsRef<Path>) -> Result<SstSummary> {
        sst::write_sst(path, self.iter())
    }

    pub fn freeze(self) -> FrozenMemtable {
        FrozenMemtable {
            entries: self.entries,
        }
    }

    pub fn needs_flush(&self) -> bool {
        self.estimated_bytes >= self.byte_cap.saturating_mul(9) / 10
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }

    pub fn byte_cap(&self) -> usize {
        self.byte_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn memtable_orders_keys_and_tracks_bytes() {
        let mut table = Memtable::new(64);

        table.put(b"k2", b"two").expect("put k2");
        table.put(b"k1", b"one").expect("put k1");

        let keys: Vec<_> = table.iter().map(|(key, _)| key.to_vec()).collect();
        assert_eq!(keys, [b"k1".to_vec(), b"k2".to_vec()]);
        assert_eq!(table.get(b"k2"), Some(b"two".as_slice()));
        assert_eq!(table.estimated_bytes(), 10);
    }

    #[test]
    fn memtable_fails_closed_at_byte_cap() {
        let mut table = Memtable::new(8);

        table.put(b"k1", b"one").expect("first fits");
        let error = table.put(b"k2", b"two").expect_err("second exceeds cap");

        assert_eq!(error.code, "CALYX_BACKPRESSURE");
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(b"k2"), None);
    }

    #[test]
    fn freeze_hands_off_sorted_entries_and_flushes() {
        let dir = test_dir("freeze");
        let path = dir.join("frozen.sst");
        let mut table = Memtable::new(64);
        table.put(b"k2", b"two").expect("put k2");
        table.put(b"k1", b"one").expect("put k1");
        let before = table
            .iter()
            .map(|(key, value)| (key.to_vec(), value.to_vec()))
            .collect::<Vec<_>>();

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
    fn needs_flush_triggers_at_ninety_percent() {
        let mut table = Memtable::new(10);

        table.put(b"1234", b"12345").expect("nine bytes");

        assert!(table.needs_flush());
        let error = table.put(b"x", b"y").expect_err("over cap");
        assert_eq!(error.code, "CALYX_BACKPRESSURE");
    }

    #[test]
    fn empty_and_zero_cap_edges_are_fail_closed() {
        let frozen = Memtable::new(8).freeze();
        assert!(frozen.is_empty());

        let mut zero = Memtable::new(0);
        let error = zero.put(b"k", b"v").expect_err("zero cap rejects");
        assert_eq!(error.code, "CALYX_BACKPRESSURE");
        assert!(zero.needs_flush());
    }

    proptest! {
        #[test]
        fn successful_puts_never_exceed_byte_cap(pairs in proptest::collection::vec((proptest::collection::vec(any::<u8>(), 1..8), proptest::collection::vec(any::<u8>(), 0..8)), 0..64)) {
            let mut table = Memtable::new(256);
            for (key, value) in pairs {
                let _ = table.put(&key, &value);
                prop_assert!(table.estimated_bytes() <= table.byte_cap());
            }
        }

        #[test]
        fn freeze_preserves_sorted_iteration(pairs in proptest::collection::vec((proptest::collection::vec(any::<u8>(), 1..8), proptest::collection::vec(any::<u8>(), 0..8)), 0..32)) {
            let mut table = Memtable::new(1024);
            for (key, value) in pairs {
                let _ = table.put(&key, &value);
            }
            let before = table.iter().map(|(key, value)| (key.to_vec(), value.to_vec())).collect::<Vec<_>>();
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
