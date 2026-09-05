use super::page;
use super::shared_reader;
use super::{
    SstEntry, SstKeyBounds, SstKeyState, SstLookupMetadata, SstPointReader, SstReader,
    read_sst_key_bounds,
};
use calyx_core::Result;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::storage_names::{SstName, classify_sst};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SstLevel {
    pub(super) files: Vec<LevelFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LevelFile {
    pub(super) path: PathBuf,
    lookup: Option<SstLookupMetadata>,
    bounds: Option<SstKeyBounds>,
}

impl LevelFile {
    fn without_lookup(path: PathBuf) -> Self {
        Self {
            path,
            lookup: None,
            bounds: None,
        }
    }

    fn with_bounds(path: PathBuf) -> Result<Self> {
        let bounds = read_sst_key_bounds(&path)?;
        Ok(Self {
            path,
            lookup: None,
            bounds,
        })
    }

    fn with_lookup(path: PathBuf) -> Result<Self> {
        // Build retained point-read metadata without inserting the mmap-backed
        // reader in the global cache. The value pages were validated once and
        // are not part of the resident lookup state.
        let reader = SstReader::open(&path)?;
        let bounds = Some(reader.key_bounds());
        let lookup = reader.lookup_metadata();
        drop(reader);
        Ok(Self {
            path,
            lookup,
            bounds,
        })
    }

    fn may_contain(&self, key: &[u8]) -> bool {
        if self
            .bounds
            .as_ref()
            .is_some_and(|bounds| !bounds.contains(key))
        {
            return false;
        }
        self.lookup.as_ref().is_none_or(|lookup| {
            key >= lookup.first_key.as_slice()
                && key <= lookup.last_key.as_slice()
                && lookup.bloom.may_contain(key)
        })
    }

    pub(super) fn overlaps(&self, start: &[u8], end: Option<&[u8]>) -> bool {
        self.bounds
            .as_ref()
            .is_none_or(|bounds| bounds.overlaps(start, end))
    }

    fn may_precede(&self, start: &[u8], upper: &[u8], inclusive: bool) -> bool {
        self.bounds
            .as_ref()
            .is_none_or(|bounds| bounds.may_precede(start, upper, inclusive))
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if !self.may_contain(key) {
            return Ok(None);
        }
        let Some(lookup) = &self.lookup else {
            return shared_reader(&self.path)?.get(key);
        };
        let Some(offset) = lookup.record_offset(key) else {
            return Ok(None);
        };
        SstPointReader::open(&self.path)?
            .read_value(offset, key)
            .map(Some)
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

    pub fn from_oldest_first_with_bounds(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            files.push(LevelFile::with_bounds(path)?);
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

    pub fn push_with_bounds(&mut self, path: PathBuf) -> Result<()> {
        self.files.insert(0, LevelFile::with_bounds(path)?);
        Ok(())
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        for file in &self.files {
            if let Some(value) = file.get(key)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// Re-derives the highest checkpointed commit that can affect a
    /// persistent search index and proves every router-flushed key has a
    /// commit-domain durable home. Relevant levels are opened with retained,
    /// fully validated lookup indexes before this method is called.
    pub(crate) fn prove_commit_domain_watermark(&self, durable_seq: u64) -> Result<u64> {
        let mut durable_files = Vec::new();
        let mut router_files = Vec::new();
        let mut watermark = 0_u64;
        for file in &self.files {
            match classify_sst(&file.path)? {
                Some(SstName::DurableBatch { seq, .. } | SstName::Compacted { seq })
                    if seq <= durable_seq =>
                {
                    watermark = watermark.max(seq);
                    durable_files.push(file);
                }
                Some(SstName::RouterLegacy { .. } | SstName::Flush { .. }) => {
                    router_files.push(file);
                }
                Some(SstName::DurableBatch { .. } | SstName::Compacted { .. }) | None => {}
            }
        }

        let mut missing_count = 0_u64;
        let mut samples = Vec::new();
        for router in router_files {
            let lookup = router.lookup.as_ref().ok_or_else(|| {
                calyx_core::CalyxError::aster_corrupt_shard(format!(
                    "persistent-search watermark migration lacks a validated lookup index for router SST {}",
                    router.path.display()
                ))
            })?;
            for key in lookup.keys() {
                if durable_files.iter().any(|file| {
                    file.lookup
                        .as_ref()
                        .is_some_and(|index| index.record_offset(key).is_some())
                }) {
                    continue;
                }
                missing_count += 1;
                if samples.len() < 3 {
                    samples.push(hex_prefix(key));
                }
            }
        }
        if missing_count != 0 {
            return Err(calyx_core::CalyxError {
                code: "CALYX_ASTER_DERIVED_CONTENT_MIGRATION_UNPROVEN",
                message: format!(
                    "persistent-search watermark migration found {missing_count} router-flushed key(s) with no commit-domain durable home, e.g. [{}]",
                    samples.join(", ")
                ),
                remediation: "preserve the vault and run verified CF compaction/recovery so every Base and quantized Slot key has a commit-domain SST before reopening; do not edit MANIFEST or SST files by hand",
            });
        }
        Ok(watermark)
    }

    pub(crate) fn values_for_key(&self, key: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut values = Vec::new();
        for file in &self.files {
            if !file.may_contain(key) {
                continue;
            }
            let reader = shared_reader(&file.path)?;
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
            .filter(|(_, file)| file.overlaps(start, Some(end)))
            .map(|(index, file)| -> Result<(usize, Vec<SstEntry>)> {
                Ok((index, shared_reader(&file.path)?.range(start, end)?))
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

    pub(crate) fn predecessor(
        &self,
        start: &[u8],
        upper: &[u8],
        inclusive: bool,
    ) -> Result<Option<SstEntry>> {
        let mut newest_at_greatest_key = None::<(usize, SstEntry)>;
        for (file_index, file) in self.files.iter().enumerate() {
            if !file.may_precede(start, upper, inclusive) {
                continue;
            }
            let Some(entry) = shared_reader(&file.path)?.predecessor(start, upper, inclusive)?
            else {
                continue;
            };
            let replace = newest_at_greatest_key
                .as_ref()
                .is_none_or(|(best_index, best)| {
                    entry.key > best.key || (entry.key == best.key && file_index < *best_index)
                });
            if replace {
                newest_at_greatest_key = Some((file_index, entry));
            }
        }
        Ok(newest_at_greatest_key.map(|(_, entry)| entry))
    }

    pub fn range_keys(&self, start: &[u8], end: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.range_keys_until(start, Some(end))
    }

    pub fn range_keys_until(&self, start: &[u8], end: Option<&[u8]>) -> Result<Vec<Vec<u8>>> {
        let mut per_file = self
            .files
            .par_iter()
            .enumerate()
            .filter(|(_, file)| file.overlaps(start, end))
            .map(|(index, file)| -> Result<(usize, Vec<SstKeyState>)> {
                Ok((
                    index,
                    shared_reader(&file.path)?.range_key_states_until(start, end)?,
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
            for entry in shared_reader(&file.path)?.iter()? {
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

fn hex_prefix(bytes: &[u8]) -> String {
    let mut value = String::new();
    for byte in bytes.iter().take(12) {
        value.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > 12 {
        value.push_str("...");
    }
    value
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
    fn persistent_search_watermark_proves_router_keys_in_commit_domain() {
        let dir = test_dir("watermark-proof");
        let durable = dir.join("00000000000000000007-0000.sst");
        let router = dir.join("flush-00000000000000000009-0001.sst");
        write_sst(&durable, [(b"cx-1".as_slice(), b"durable".as_slice())]).unwrap();
        write_sst(&router, [(b"cx-1".as_slice(), b"latest".as_slice())]).unwrap();
        let level = SstLevel::from_oldest_first_with_lookup([durable, router]).unwrap();

        assert_eq!(level.prove_commit_domain_watermark(9).unwrap(), 7);
        cleanup(dir);
    }

    #[test]
    fn persistent_search_watermark_rejects_router_only_key() {
        let dir = test_dir("watermark-router-only");
        let durable = dir.join("00000000000000000007-0000.sst");
        let router = dir.join("flush-00000000000000000009-0001.sst");
        write_sst(&durable, [(b"cx-1".as_slice(), b"durable".as_slice())]).unwrap();
        write_sst(&router, [(b"cx-2".as_slice(), b"orphan".as_slice())]).unwrap();
        let level = SstLevel::from_oldest_first_with_lookup([durable, router]).unwrap();

        let error = level
            .prove_commit_domain_watermark(9)
            .expect_err("router-only key must fail migration");
        assert_eq!(error.code, "CALYX_ASTER_DERIVED_CONTENT_MIGRATION_UNPROVEN");
        assert!(error.message.contains("1 router-flushed key"));
        cleanup(dir);
    }

    #[test]
    fn persistent_search_watermark_rejects_coverage_beyond_manifest_floor() {
        let dir = test_dir("watermark-beyond-floor");
        let future = dir.join("00000000000000000009-0000.sst");
        let router = dir.join("flush-00000000000000000009-0001.sst");
        write_sst(&future, [(b"cx-1".as_slice(), b"future".as_slice())]).unwrap();
        write_sst(&router, [(b"cx-1".as_slice(), b"router".as_slice())]).unwrap();
        let level = SstLevel::from_oldest_first_with_lookup([future, router]).unwrap();

        let error = level
            .prove_commit_domain_watermark(8)
            .expect_err("uncheckpointed SST must not prove durable coverage");
        assert_eq!(error.code, "CALYX_ASTER_DERIVED_CONTENT_MIGRATION_UNPROVEN");
        cleanup(dir);
    }

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

    #[test]
    fn validated_lookup_rejects_bloom_false_positive_before_file_io() {
        let dir = test_dir("validated-exact-index");
        let path = dir.join("large.sst");
        let first_value = vec![0x31; 2 * 1024 * 1024];
        let last_value = vec![0x7a; 2 * 1024 * 1024];
        write_sst(
            &path,
            [
                (b"a".as_slice(), first_value.as_slice()),
                (b"z".as_slice(), last_value.as_slice()),
            ],
        )
        .unwrap();
        let level = SstLevel::from_oldest_first_with_lookup([path.clone()]).unwrap();
        let missing = (0..=u16::MAX)
            .map(|value| format!("m{value:04x}").into_bytes())
            .find(|key| level.candidate_file_count_for_key(key) == 1)
            .expect("find a deterministic Bloom candidate absent from the exact index");

        assert_eq!(level.get(b"a").unwrap(), Some(first_value));
        fs::remove_file(&path).unwrap();
        assert_eq!(
            level.get(&missing).unwrap(),
            None,
            "the validated exact index must reject a Bloom false positive without reopening the SST"
        );
        let error = level
            .get(b"a")
            .expect_err("an exact match must fail if its immutable SST disappeared");
        eprintln!(
            "ISSUE1802_EXACT_INDEX large_value_bytes={} bloom_candidate={} missing_without_io=true exact_missing_error={}",
            2 * 1024 * 1024,
            String::from_utf8_lossy(&missing),
            error.code
        );
        cleanup(dir);
    }

    #[test]
    fn validated_lookup_exact_read_fails_loud_after_physical_truncation() {
        let dir = test_dir("validated-truncated-point");
        let path = dir.join("truncated.sst");
        let value = vec![0x55; 2 * 1024 * 1024];
        write_sst(&path, [(b"key".as_slice(), value.as_slice())]).unwrap();
        let level = SstLevel::from_oldest_first_with_lookup([path.clone()]).unwrap();
        // Simulates external truncation: drop the shared-cache mapping first
        // (Windows refuses to truncate a mapped file), the same contract every
        // reclaim site follows before deleting an SST.
        crate::sst::invalidate_reader(&path);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(super::super::HEADER_LEN as u64)
            .unwrap();

        let error = level
            .get(b"key")
            .expect_err("truncated matched SST must not produce a value");
        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        eprintln!(
            "ISSUE1802_TRUNCATED_POINT physical_bytes_after={} error_code={} exact_value_returned=false",
            fs::metadata(&path).unwrap().len(),
            error.code
        );
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
