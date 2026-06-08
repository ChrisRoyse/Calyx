use super::{SstEntry, SstReader};
use calyx_core::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SstLevel {
    files: Vec<PathBuf>,
}

impl SstLevel {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub fn from_oldest_first(files: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut level = Self::new();
        for file in files {
            level.push(file);
        }
        level
    }

    pub fn push(&mut self, path: PathBuf) {
        self.files.insert(0, path);
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        for file in &self.files {
            let reader = SstReader::open(file)?;
            if let Some(value) = reader.get(key)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn range(&self, start: &[u8], end: &[u8]) -> Result<Vec<SstEntry>> {
        let mut rows = BTreeMap::new();
        for file in &self.files {
            for entry in SstReader::open(file)?.range(start, end)? {
                rows.entry(entry.key).or_insert(entry.value);
            }
        }
        Ok(rows
            .into_iter()
            .map(|(key, value)| SstEntry { key, value })
            .collect())
    }

    pub fn iter(&self) -> Result<Vec<SstEntry>> {
        let mut rows = BTreeMap::new();
        for file in &self.files {
            for entry in SstReader::open(file)?.iter()? {
                rows.entry(entry.key).or_insert(entry.value);
            }
        }
        Ok(rows
            .into_iter()
            .map(|(key, value)| SstEntry { key, value })
            .collect())
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sst::write_sst;
    use proptest::prelude::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn newest_first_point_lookup_wins() {
        let dir = test_dir("newest");
        let old = dir.join("old.sst");
        let new = dir.join("new.sst");
        write_sst(&old, [(b"k1".as_slice(), b"old".as_slice())]).unwrap();
        write_sst(&new, [(b"k1".as_slice(), b"new".as_slice())]).unwrap();
        let mut level = SstLevel::new();
        level.push(old);
        level.push(new);

        assert_eq!(level.get(b"k1").unwrap(), Some(b"new".to_vec()));
        cleanup(dir);
    }

    #[test]
    fn range_merge_deduplicates_sorted_with_newest_winning() {
        let dir = test_dir("range");
        let a = dir.join("a.sst");
        let b = dir.join("b.sst");
        write_sst(&a, [(b"k1".as_slice(), b"a1".as_slice()), (b"k3", b"a3")]).unwrap();
        write_sst(&b, [(b"k2".as_slice(), b"b2".as_slice()), (b"k3", b"b3")]).unwrap();
        let mut level = SstLevel::new();
        level.push(a);
        level.push(b);

        let rows = level.range(b"k1", b"k4").unwrap();

        assert_eq!(
            rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>(),
            [b"k1".to_vec(), b"k2".to_vec(), b"k3".to_vec()]
        );
        assert_eq!(rows[2].value, b"b3");
        cleanup(dir);
    }

    #[test]
    fn empty_and_oldest_only_edges() {
        let dir = test_dir("edges");
        let mut level = SstLevel::new();
        assert_eq!(level.get(b"none").unwrap(), None);
        assert!(level.range(b"", b"\xff").unwrap().is_empty());
        let old = dir.join("old.sst");
        write_sst(&old, [(b"k".as_slice(), b"v".as_slice())]).unwrap();
        level.push(old);
        assert_eq!(level.get(b"k").unwrap(), Some(b"v".to_vec()));
        cleanup(dir);
    }

    proptest! {
        #[test]
        fn level_returns_latest_values(pairs in proptest::collection::vec((proptest::collection::vec(any::<u8>(), 1..8), proptest::collection::vec(any::<u8>(), 0..8)), 1..32)) {
            let dir = test_dir("proptest");
            let mut expected = BTreeMap::new();
            let mut level = SstLevel::new();
            for (index, (key, value)) in pairs.iter().enumerate() {
                let path = dir.join(format!("{index:02}.sst"));
                write_sst(&path, [(key.as_slice(), value.as_slice())]).unwrap();
                level.push(path);
                expected.insert(key.clone(), value.clone());
            }
            for (key, value) in expected {
                prop_assert_eq!(level.get(&key).unwrap(), Some(value));
            }
            cleanup(dir);
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-level-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).unwrap();
    }
}
