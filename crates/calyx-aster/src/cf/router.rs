use super::ColumnFamily;
use crate::compaction::TieringPolicy;
use crate::memtable::{Memtable, MemtableUsage};
use crate::resource::ResourceCounters;
use crate::sst::level::SstLevel;
use crate::sst::{SstEntry, SstSummary};
use crate::storage_names::{SstName, classify_sst, parse_cf_dir_name, sst_order_key};
use calyx_core::{CalyxError, Result};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_MEMTABLE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct CfRouter {
    vault_dir: PathBuf,
    tiering_policy: Option<TieringPolicy>,
    memtables: HashMap<ColumnFamily, Memtable>,
    levels: HashMap<ColumnFamily, SstLevel>,
    next_file: HashMap<ColumnFamily, u64>,
    memtable_byte_cap: usize,
    resource_counters: Arc<ResourceCounters>,
}

impl CfRouter {
    pub fn open(vault_dir: impl AsRef<Path>, memtable_byte_cap: usize) -> Result<Self> {
        Self::open_with_tiering(vault_dir, memtable_byte_cap, None)
    }

    pub fn open_with_tiering(
        vault_dir: impl AsRef<Path>,
        memtable_byte_cap: usize,
        tiering_policy: Option<TieringPolicy>,
    ) -> Result<Self> {
        let vault_dir = vault_dir.as_ref().to_path_buf();
        let memtable_byte_cap = if memtable_byte_cap == 0 {
            DEFAULT_MEMTABLE_BYTES
        } else {
            memtable_byte_cap
        };
        fs::create_dir_all(vault_dir.join("cf"))
            .map_err(|error| CalyxError::disk_pressure(format!("create CF root: {error}")))?;
        if let Some(policy) = &tiering_policy {
            for tier_root in policy.tier_roots() {
                fs::create_dir_all(tier_root.join("cf")).map_err(|error| {
                    CalyxError::disk_pressure(format!("create tiered CF root: {error}"))
                })?;
            }
        }
        let mut router = Self {
            vault_dir,
            tiering_policy,
            memtables: HashMap::new(),
            levels: HashMap::new(),
            next_file: HashMap::new(),
            memtable_byte_cap,
            resource_counters: Arc::new(ResourceCounters::default()),
        };
        for cf in ColumnFamily::STATIC {
            router.ensure_cf(cf)?;
        }
        router.load_existing()?;
        Ok(router)
    }

    pub fn put(&mut self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        self.ensure_cf(cf)?;
        let mut counted_backpressure = false;
        let ack = match self.memtable_mut(cf).write(key, value, 0) {
            Ok(ack) => ack,
            Err(error) => {
                if error.code != "CALYX_BACKPRESSURE" {
                    return Err(error);
                }
                self.flush_cf(cf)?;
                match self.memtable_mut(cf).write(key, value, 0) {
                    Ok(ack) => {
                        self.resource_counters.record_memtable_absorbed();
                        counted_backpressure = true;
                        ack
                    }
                    Err(retry_error) => {
                        if retry_error.code == "CALYX_BACKPRESSURE" {
                            self.resource_counters.record_memtable_rejected();
                        }
                        return Err(retry_error);
                    }
                }
            }
        };
        if ack.flush_triggered {
            if !counted_backpressure {
                self.resource_counters.record_memtable_absorbed();
            }
            self.flush_cf(cf)?;
        }
        Ok(())
    }

    /// Fails closed before WAL append when a row can never fit in one memtable.
    pub fn ensure_batch_admitted<I, K, V>(&self, rows: I) -> Result<()>
    where
        I: IntoIterator<Item = (ColumnFamily, K, V)>,
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        for (cf, key, value) in rows {
            let row_bytes = Memtable::entry_size(key.as_ref(), value.as_ref());
            if row_bytes > self.memtable_byte_cap {
                self.resource_counters.record_memtable_rejected();
                return Err(CalyxError::backpressure(format!(
                    "memtable byte cap {} cannot fit {} row of {} bytes",
                    self.memtable_byte_cap,
                    cf.name(),
                    row_bytes
                )));
            }
        }
        Ok(())
    }

    /// Shares the backpressure counters this router increments.
    pub fn resource_counters(&self) -> Arc<ResourceCounters> {
        Arc::clone(&self.resource_counters)
    }

    pub fn memtable_usage_by_cf(&self) -> Vec<(ColumnFamily, MemtableUsage)> {
        let mut usage = self
            .memtables
            .iter()
            .map(|(cf, table)| (*cf, table.usage()))
            .collect::<Vec<_>>();
        usage.sort_by_key(|left| left.0.name());
        usage
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
            return Ok(Some(value));
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
                if key.as_slice() >= start && key.as_slice() < end {
                    rows.insert(key, value);
                }
            }
        }
        Ok(rows
            .into_iter()
            .map(|(key, value)| SstEntry { key, value })
            .collect())
    }

    pub fn iter_cf(&self, cf: ColumnFamily) -> Result<Vec<SstEntry>> {
        let mut rows = BTreeMap::new();
        if let Some(level) = self.levels.get(&cf) {
            for entry in level.iter()? {
                rows.insert(entry.key, entry.value);
            }
        }
        if let Some(table) = self.memtables.get(&cf) {
            for (key, value) in table.iter() {
                rows.insert(key, value);
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

    pub fn flush_pending(&mut self) -> Result<Vec<SstSummary>> {
        let cfs = self
            .memtables
            .iter()
            .filter_map(|(cf, table)| (!table.is_empty()).then_some(*cf))
            .collect::<Vec<_>>();
        let mut summaries = Vec::with_capacity(cfs.len());
        for cf in cfs {
            summaries.push(self.flush_cf(cf)?);
        }
        Ok(summaries)
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
        let mut by_cf = HashMap::<ColumnFamily, Vec<PathBuf>>::new();
        for cf_root in self.cf_roots() {
            if !cf_root.exists() {
                continue;
            }
            for entry in fs::read_dir(cf_root)
                .map_err(|error| CalyxError::disk_pressure(format!("read CF root: {error}")))?
            {
                let path = entry
                    .map_err(|error| CalyxError::disk_pressure(format!("read CF entry: {error}")))?
                    .path();
                if !path.is_dir() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(format!(
                            "CF directory entry {} has no name",
                            path.display()
                        ))
                    })?;
                let cf = parse_cf_dir_name(&name)?;
                by_cf.entry(cf).or_default().extend(list_sst_files(&path)?);
            }
        }
        for (cf, mut files) in by_cf {
            sort_ssts_by_sequence(&mut files)?;
            files.dedup();
            // Only router-flushed SSTs participate in the next-file counter;
            // durable batches and compaction outputs use disjoint name shapes.
            let next = files
                .iter()
                .filter_map(|file| match classify_sst(file) {
                    Ok(Some(SstName::Router { seq })) => Some(seq),
                    _ => None,
                })
                .max()
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
        self.tiering_policy.as_ref().map_or_else(
            || self.vault_dir.join("cf").join(cf.name()),
            |policy| policy.place_current_cf(cf).absolute_dir(),
        )
    }

    fn cf_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.vault_dir.join("cf")];
        if let Some(policy) = &self.tiering_policy {
            for tier_root in policy.tier_roots() {
                let cf_root = tier_root.join("cf");
                if !roots.contains(&cf_root) {
                    roots.push(cf_root);
                }
            }
        }
        roots
    }
}

/// Lists SST files in a CF directory, failing closed on any `*.sst` file
/// whose name matches no canonical writer shape (such files were previously
/// loaded into levels while being invisible to the next-file counter).
fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("read CF file: {error}")))?
            .path();
        if sst_order_key(&path)?.is_some() {
            files.push(path);
        }
    }
    Ok(files)
}

fn sort_ssts_by_sequence(files: &mut [PathBuf]) -> Result<()> {
    let mut keyed = files
        .iter()
        .map(|path| {
            Ok((
                sst_order_key(path)?.ok_or_else(|| {
                    CalyxError::aster_corrupt_shard(format!(
                        "non-SST path {} in CF level",
                        path.display()
                    ))
                })?,
                path.clone(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    for (slot, (_, path)) in files.iter_mut().zip(keyed) {
        *slot = path;
    }
    Ok(())
}
