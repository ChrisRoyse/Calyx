use super::page;
use super::{SstEntry, SstKeyState, SstLookupMetadata, SstReader};
use calyx_core::Result;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SstLevel {
    pub(super) files: Vec<LevelFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LevelFile {
    pub(super) path: PathBuf,
    lookup: Option<SstLookupMetadata>,
}

impl LevelFile {
    fn without_lookup(path: PathBuf) -> Self {
        Self { path, lookup: None }
    }

    fn with_lookup(path: PathBuf) -> Result<Self> {
        let lookup = SstReader::open(&path)?.lookup_metadata();
        Ok(Self { path, lookup })
    }

    fn may_contain(&self, key: &[u8]) -> bool {
        let Some(lookup) = &self.lookup else {
            return true;
        };
        key >= lookup.first_key.as_slice()
            && key <= lookup.last_key.as_slice()
            && lookup.bloom.may_contain(key)
    }
}

impl SstLevel {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub fn from_oldest_first(files: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut files = files
            .into_iter()
            .map(LevelFile::without_lookup)
            .collect::<Vec<_>>();
        files.reverse();
        Self { files }
    }

    pub fn from_oldest_first_with_lookup(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            files.push(LevelFile::with_lookup(path)?);
        }
        files.reverse();
        Ok(Self { files })
    }

    pub fn push(&mut self, path: PathBuf) {
        self.files.insert(0, LevelFile::without_lookup(path));
    }

    pub fn push_with_lookup(&mut self, path: PathBuf) -> Result<()> {
        self.files.insert(0, LevelFile::with_lookup(path)?);
        Ok(())
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        for file in &self.files {
            if !file.may_contain(key) {
                continue;
            }
            let reader = SstReader::open(&file.path)?;
            if let Some(value) = reader.get(key)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub(crate) fn values_for_key(&self, key: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut values = Vec::new();
        for file in &self.files {
            if !file.may_contain(key) {
                continue;
            }
            let reader = SstReader::open(&file.path)?;
            if let Some(value) = reader.get(key)? {
                values.push(value);
            }
        }
        Ok(values)
    }

    pub fn range(&self, start: &[u8], end: &[u8]) -> Result<Vec<SstEntry>> {
        let mut per_file = self
            .files
            .par_iter()
            .enumerate()
            .map(|(index, file)| -> Result<(usize, Vec<SstEntry>)> {
                Ok((index, SstReader::open(&file.path)?.range(start, end)?))
            })
            .collect::<Result<Vec<_>>>()?;
        per_file.sort_by_key(|(index, _)| *index);

        let mut rows = BTreeMap::new();
        for (_, entries) in per_file {
            for entry in entries {
                rows.entry(entry.key).or_insert(entry.value);
            }
        }
        Ok(rows
            .into_iter()
            .map(|(key, value)| SstEntry { key, value })
            .collect())
    }

    pub fn range_keys(&self, start: &[u8], end: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.range_keys_until(start, Some(end))
    }

    pub fn range_keys_until(&self, start: &[u8], end: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
        let mut per_file = self
            .files
            .par_iter()
            .enumerate()
            .map(|(index, file)| -> Result<(usize, Vec<SstKeyState>)> {
                Ok((
                    index,
                    SstReader::open(&file.path)?.range_key_states_until(start, end)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        per_file.sort_by_key(|(index, _)| *index);

        let mut rows = BTreeMap::<Vec<u8>, bool>::new();
        for (_, entries) in per_file {
            for entry in entries {
                rows.entry(entry.key).or_insert(entry.is_tombstone);
            }
        }
        Ok(rows
            .into_iter()
            .filter_map(|(key, is_tombstone)| (!is_tombstone).then_some(key))
            .collect())
    }

    pub fn range_page_until(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
        after_key: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<SstEntry>> {
        self.range_page_with_overlay(start, end, after_key, limit, Vec::new())
    }

    pub(crate) fn range_page_with_overlay(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
        after_key: Option<&[u8]>,
        limit: usize,
        overlay: Vec<SstEntry>,
    ) -> Result<Vec<SstEntry>> {
        page::range_page(self, start, end, after_key, limit, overlay)
    }

    pub(crate) fn range_pages_with_overlay<F, E>(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
        after_key: Option<&[u8]>,
        limit: usize,
        overlay: Vec<SstEntry>,
        on_page: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(Vec<SstEntry>) -> std::result::Result<(), E>,
        E: From<calyx_core::CalyxError>,
    {
        page::range_pages(self, start, end, after_key, limit, overlay, on_page)
    }

    pub fn iter(&self) -> Result<Vec<SstEntry>> {
        let mut rows = BTreeMap::new();
        for file in &self.files {
            for entry in SstReader::open(&file.path)?.iter()? {
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

    pub(crate) fn file_paths_newest_first(&self) -> Vec<PathBuf> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }

    #[cfg(test)]
    fn candidate_file_count_for_key(&self, key: &[u8]) -> usize {
        self.files
            .iter()
            .filter(|file| file.may_contain(key))
            .count()
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
    fn range_key_scan_preserves_newest_order_and_tombstones() {
        let dir = test_dir("range-key-tombstone");
        let old = dir.join("old.sst");
        let mid = dir.join("mid.sst");
        let new = dir.join("new.sst");
        let tombstone = crate::mvcc::tombstone_value();
        write_sst(
            &old,
            [
                (b"k1".as_slice(), b"old-1".as_slice()),
                (b"k2".as_slice(), b"old-2".as_slice()),
            ],
        )
        .unwrap();
        write_sst(&mid, [(b"k1".as_slice(), tombstone.as_slice())]).unwrap();
        write_sst(
            &new,
            [
                (b"k2".as_slice(), b"new-2".as_slice()),
                (b"k3".as_slice(), b"new-3".as_slice()),
            ],
        )
        .unwrap();
        let mut level = SstLevel::new();
        level.push(old);
        level.push(mid);
        level.push(new);

        let rows = level.range(b"k1", b"k4").unwrap();
        let values = rows
            .iter()
            .map(|row| (row.key.clone(), row.value.clone()))
            .collect::<BTreeMap<_, _>>();
        assert!(crate::mvcc::is_tombstone_value(
            values.get(b"k1".as_slice()).unwrap()
        ));
        assert_eq!(values.get(b"k2".as_slice()).unwrap().as_slice(), b"new-2");
        assert_eq!(values.get(b"k3".as_slice()).unwrap().as_slice(), b"new-3");

        assert_eq!(
            level.range_keys(b"k1", b"k4").unwrap(),
            [b"k2".to_vec(), b"k3".to_vec()]
        );
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

    #[test]
    fn metadata_bounds_point_lookup_to_candidate_sst() {
        let dir = test_dir("metadata-point");
        let mut files = Vec::new();
        for index in 0..128u8 {
            let key = vec![index; 16];
            let value = vec![index.wrapping_add(1); 8];
            let path = dir.join(format!("{index:03}.sst"));
            write_sst(&path, [(key.as_slice(), value.as_slice())]).unwrap();
            files.push(path);
        }
        let level = SstLevel::from_oldest_first_with_lookup(files).unwrap();
        let key = vec![42u8; 16];

        assert_eq!(level.file_count(), 128);
        assert_eq!(level.candidate_file_count_for_key(&key), 1);
        assert_eq!(level.get(&key).unwrap(), Some(vec![43u8; 8]));
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
