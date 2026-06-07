use super::AsterVault;
use crate::cf::ColumnFamily;
use crate::compaction::{
    CompactionCatalog, CompactionResult, CompactionScheduler, CompactionSchedulerOptions,
    CompactionThrottle, catalog_from_vault_dir,
};
use calyx_core::{Clock, Result};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
pub struct VaultCompactionScheduler {
    catalog: Arc<CompactionCatalog>,
    scheduler: CompactionScheduler,
}

impl VaultCompactionScheduler {
    pub fn shard_count_for_cf(&self, cf: ColumnFamily) -> usize {
        self.catalog.shard_count_for_cf(cf)
    }

    pub fn stop(self) -> std::thread::Result<()> {
        self.scheduler.stop()
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn compaction_catalog(&self) -> Result<Option<Arc<CompactionCatalog>>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        durable.flush()?;
        self.rows.flush_all_cfs()?;
        Ok(Some(Arc::new(catalog_from_vault_dir(durable.root())?)))
    }

    pub fn compact_cf_once(&self, cf: ColumnFamily) -> Result<Option<CompactionResult>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        durable.flush()?;
        self.rows.flush_all_cfs()?;
        let catalog = catalog_from_vault_dir(durable.root())?;
        let output = compaction_output_path(durable.root(), cf, self.latest_seq());
        catalog
            .compact_cf(cf, output, CompactionThrottle::unlimited())
            .map(Some)
    }

    pub fn start_compaction_scheduler(
        &self,
        options: CompactionSchedulerOptions,
    ) -> Result<Option<VaultCompactionScheduler>> {
        let Some(catalog) = self.compaction_catalog()? else {
            return Ok(None);
        };
        let scheduler = CompactionScheduler::start(catalog.clone(), options);
        Ok(Some(VaultCompactionScheduler { catalog, scheduler }))
    }
}

fn compaction_output_path(root: &std::path::Path, cf: ColumnFamily, seq: u64) -> PathBuf {
    root.join("cf")
        .join(cf.name())
        .join(format!("compacted-{seq:020}.sst"))
}
