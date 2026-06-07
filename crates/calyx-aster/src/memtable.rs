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
}
