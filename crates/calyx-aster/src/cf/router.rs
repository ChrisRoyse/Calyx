use super::ColumnFamily;
use crate::memtable::Memtable;
use crate::sst::level::SstLevel;
use crate::sst::{SstEntry, SstSummary};
use calyx_core::{CalyxError, Result, SlotId};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MEMTABLE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct CfRouter {
    vault_dir: PathBuf,
    memtables: HashMap<ColumnFamily, Memtable>,
    levels: HashMap<ColumnFamily, SstLevel>,
    next_file: HashMap<ColumnFamily, u64>,
    memtable_byte_cap: usize,
}

impl CfRouter {
    pub fn open(vault_dir: impl AsRef<Path>, memtable_byte_cap: usize) -> Result<Self> {
        let vault_dir = vault_dir.as_ref().to_path_buf();
        let memtable_byte_cap = if memtable_byte_cap == 0 {
            DEFAULT_MEMTABLE_BYTES
        } else {
            memtable_byte_cap
        };
        fs::create_dir_all(vault_dir.join("cf"))
            .map_err(|error| CalyxError::disk_pressure(format!("create CF root: {error}")))?;
        let mut router = Self {
            vault_dir,
            memtables: HashMap::new(),
            levels: HashMap::new(),
            next_file: HashMap::new(),
            memtable_byte_cap,
        };
        for cf in ColumnFamily::STATIC {
            router.ensure_cf(cf)?;
        }
        router.load_existing()?;
        Ok(router)
    }

    pub fn put(&mut self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        self.ensure_cf(cf)?;
        let table = self.memtable_mut(cf);
        if let Err(error) = table.put(key, value) {
            if error.code != "CALYX_BACKPRESSURE" {
                return Err(error);
            }
            self.flush_cf(cf)?;
            self.memtable_mut(cf).put(key, value)?;
        }
        if self.memtable_mut(cf).needs_flush() {
            self.flush_cf(cf)?;
        }
        Ok(())
    }

    pub fn flush_cf(&mut self, cf: ColumnFamily) -> Result<SstSummary> {
        self.ensure_cf(cf)?;
        let fresh = Memtable::new(self.memtable_byte_cap);
        let frozen = std::mem::replace(self.memtable_mut(cf), fresh).freeze();
        let seq = self.next_sequence(cf);
        let path = self.cf_dir(cf).join(format!("{seq:020}.sst"));
        let summary = frozen.flush_to_sst(&path)?;
        self.levels
            .entry(cf)
            .or_default()
            .push(summary.path.clone());
        Ok(summary)
    }

    pub fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(value) = self.memtables.get(&cf).and_then(|table| table.get(key)) {
            return Ok(Some(value.to_vec()));
        }
        self.levels
            .get(&cf)
            .map_or(Ok(None), |level| level.get(key))
    }

    pub fn range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<Vec<SstEntry>> {
        let mut rows = BTreeMap::new();
        if let Some(level) = self.levels.get(&cf) {
            for entry in level.range(start, end)? {
                rows.insert(entry.key, entry.value);
            }
        }
        if let Some(table) = self.memtables.get(&cf) {
            for (key, value) in table.iter() {
                if key >= start && key < end {
                    rows.insert(key.to_vec(), value.to_vec());
                }
            }
        }
        Ok(rows
            .into_iter()
            .map(|(key, value)| SstEntry { key, value })
            .collect())
    }

    pub fn level_file_count(&self, cf: ColumnFamily) -> usize {
        self.levels.get(&cf).map_or(0, SstLevel::file_count)
    }

    fn ensure_cf(&mut self, cf: ColumnFamily) -> Result<()> {
        fs::create_dir_all(self.cf_dir(cf))
            .map_err(|error| CalyxError::disk_pressure(format!("create CF dir: {error}")))?;
        self.memtables
            .entry(cf)
            .or_insert_with(|| Memtable::new(self.memtable_byte_cap));
        self.levels.entry(cf).or_default();
        self.next_file.entry(cf).or_insert(1);
        Ok(())
    }

    fn load_existing(&mut self) -> Result<()> {
        let cf_root = self.vault_dir.join("cf");
        for entry in fs::read_dir(cf_root)
            .map_err(|error| CalyxError::disk_pressure(format!("read CF root: {error}")))?
        {
            let path = entry
                .map_err(|error| CalyxError::disk_pressure(format!("read CF entry: {error}")))?
                .path();
            if !path.is_dir() {
                continue;
            }
            let Some(cf) = parse_cf_dir(&path) else {
                continue;
            };
            let mut files = list_sst_files(&path)?;
            files.sort();
            let next = files
                .last()
                .and_then(|file| file.file_stem()?.to_string_lossy().parse::<u64>().ok())
                .unwrap_or(0)
                + 1;
            self.ensure_cf(cf)?;
            self.levels.insert(cf, SstLevel::from_oldest_first(files));
            self.next_file.insert(cf, next);
        }
        Ok(())
    }

    fn memtable_mut(&mut self, cf: ColumnFamily) -> &mut Memtable {
        self.memtables
            .entry(cf)
            .or_insert_with(|| Memtable::new(self.memtable_byte_cap))
    }

    fn next_sequence(&mut self, cf: ColumnFamily) -> u64 {
        let next = self.next_file.entry(cf).or_insert(1);
        let seq = *next;
        *next += 1;
        seq
    }

    fn cf_dir(&self, cf: ColumnFamily) -> PathBuf {
        self.vault_dir.join("cf").join(cf.name())
    }
}

fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("read CF file: {error}")))?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    Ok(files)
}

fn parse_cf_dir(path: &Path) -> Option<ColumnFamily> {
    let name = path.file_name()?.to_string_lossy();
    match name.as_ref() {
        "base" => Some(ColumnFamily::Base),
        "xterm" => Some(ColumnFamily::XTerm),
        "scalars" => Some(ColumnFamily::Scalars),
        "anchors" => Some(ColumnFamily::Anchors),
        "ledger" => Some(ColumnFamily::Ledger),
        "online" => Some(ColumnFamily::Online),
        _ if name.starts_with("slot_") => parse_slot_name(&name),
        _ => None,
    }
}

fn parse_slot_name(name: &str) -> Option<ColumnFamily> {
    let raw = name.ends_with(".raw");
    let slot = name
        .trim_start_matches("slot_")
        .trim_end_matches(".raw")
        .parse::<u16>()
        .ok()?;
    Some(if raw {
        ColumnFamily::slot_raw(SlotId::new(slot))
    } else {
        ColumnFamily::slot(SlotId::new(slot))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf::ColumnFamily;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn put_get_and_flush_dispatch_per_cf() {
        let dir = test_dir("put-get");
        let mut router = CfRouter::open(&dir, 12).unwrap();

        router.put(ColumnFamily::Base, b"k1", b"v1").unwrap();
        router
            .put(ColumnFamily::slot(SlotId::new(0)), b"k1", b"s1")
            .unwrap();
        router.flush_cf(ColumnFamily::Base).unwrap();
        router.flush_cf(ColumnFamily::slot(SlotId::new(0))).unwrap();

        assert_eq!(
            router.get(ColumnFamily::Base, b"k1").unwrap(),
            Some(b"v1".to_vec())
        );
        assert_eq!(
            router
                .get(ColumnFamily::slot(SlotId::new(0)), b"k1")
                .unwrap(),
            Some(b"s1".to_vec())
        );
        assert_eq!(router.level_file_count(ColumnFamily::Base), 1);
        assert_eq!(
            router.level_file_count(ColumnFamily::slot(SlotId::new(0))),
            1
        );
        cleanup(dir);
    }

    #[test]
    fn range_merges_memtable_and_sst_with_memtable_winning() {
        let dir = test_dir("range");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        router.put(ColumnFamily::Base, b"k1", b"old").unwrap();
        router.flush_cf(ColumnFamily::Base).unwrap();
        router.put(ColumnFamily::Base, b"k1", b"new").unwrap();
        router.put(ColumnFamily::Base, b"k2", b"two").unwrap();

        let rows = router.range(ColumnFamily::Base, b"", b"\xff").unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].value, b"new");
        assert_eq!(rows[1].value, b"two");
        cleanup(dir);
    }

    #[test]
    fn reopen_loads_existing_sst_files() {
        let dir = test_dir("reopen");
        let mut router = CfRouter::open(&dir, 8).unwrap();
        router.put(ColumnFamily::Base, b"k", b"value").unwrap();
        router.flush_cf(ColumnFamily::Base).unwrap();
        drop(router);

        let reopened = CfRouter::open(&dir, 8).unwrap();

        assert_eq!(
            reopened.get(ColumnFamily::Base, b"k").unwrap(),
            Some(b"value".to_vec())
        );
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-router-{name}-{}-{id}",
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
