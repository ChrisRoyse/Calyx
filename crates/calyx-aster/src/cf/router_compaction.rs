use super::{CfRouter, ColumnFamily, NO_COMMIT_DOMAIN};
use crate::mvcc::is_tombstone_value;
use crate::sst::level::SstLevel;
use crate::sst::write_sst;
use crate::storage_names::flush_sst_file_name;
use calyx_core::{CalyxError, Result};
use std::fs;

impl CfRouter {
    /// Collapses each listed CF to one newest router-domain SST and removes
    /// tombstone rows after the replacement is durable.
    pub fn compact_tombstoned_cfs(&mut self, cfs: &[ColumnFamily]) -> Result<()> {
        self.compact_tombstoned_cfs_at(cfs, NO_COMMIT_DOMAIN)
    }

    pub(crate) fn compact_tombstoned_cfs_at(
        &mut self,
        cfs: &[ColumnFamily],
        commit_watermark: u64,
    ) -> Result<()> {
        self.flush_pending_at(commit_watermark)?;
        let mut unique = Vec::new();
        for cf in cfs {
            if !unique.contains(cf) {
                unique.push(*cf);
            }
        }
        for cf in unique {
            self.compact_tombstoned_cf(cf, commit_watermark)?;
        }
        Ok(())
    }

    fn compact_tombstoned_cf(&mut self, cf: ColumnFamily, commit_watermark: u64) -> Result<()> {
        let Some(level) = self.levels.get(&cf) else {
            return Ok(());
        };
        let input_paths = level.file_paths_newest_first();
        if input_paths.is_empty() {
            return Ok(());
        }
        let visible = level.iter()?;
        let retained = visible
            .into_iter()
            .filter(|entry| !is_tombstone_value(&entry.value))
            .collect::<Vec<_>>();
        let ordinal = usize::try_from(self.next_sequence(cf)).map_err(|_| {
            CalyxError::aster_corrupt_shard(format!(
                "router compaction ordinal for {} exceeds the platform usize range",
                cf.name()
            ))
        })?;
        let output = self
            .cf_dir(cf)
            .join(flush_sst_file_name(commit_watermark, ordinal));
        write_sst(
            &output,
            retained
                .iter()
                .map(|entry| (entry.key.as_slice(), entry.value.as_slice())),
        )?;
        let replacement = SstLevel::from_oldest_first_with_lookup([output.clone()])?;
        self.levels.insert(cf, replacement);

        for input in input_paths {
            if input == output {
                continue;
            }
            fs::remove_file(&input).map_err(|error| {
                CalyxError::disk_pressure(format!(
                    "reclaim router compaction input {}: {error}",
                    input.display()
                ))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::scan_tombstone_inventory;
    use crate::mvcc::tombstone_value;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "calyx-router-tombstone-compaction-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            Self(root)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn compaction_removes_physical_tombstones_and_preserves_live_rows() {
        let root = TestRoot::new();
        let mut router = CfRouter::open(&root.0, 1024 * 1024).expect("open router");
        router
            .put(ColumnFamily::Base, b"live", b"value")
            .expect("write live row");
        router
            .put(ColumnFamily::Base, b"deleted", &tombstone_value())
            .expect("write tombstone");
        router.flush_pending().expect("flush rows");
        router
            .put(ColumnFamily::Base, b"replaced", &tombstone_value())
            .expect("write obsolete tombstone");
        router.flush_pending().expect("flush obsolete tombstone");
        router
            .put(ColumnFamily::Base, b"replaced", b"new-value")
            .expect("replace obsolete tombstone");
        router.flush_pending().expect("flush replacement row");
        let before = scan_tombstone_inventory(&root.0).expect("scan before");

        router
            .compact_tombstoned_cfs(&[ColumnFamily::Base])
            .expect("compact tombstones");

        let after = scan_tombstone_inventory(&root.0).expect("scan after");
        assert_eq!(before.tombstone_keys(), 2);
        assert_eq!(after.tombstone_keys(), 0);
        assert_eq!(
            router.get(ColumnFamily::Base, b"live").unwrap(),
            Some(b"value".to_vec())
        );
        assert_eq!(router.get(ColumnFamily::Base, b"deleted").unwrap(), None);
        assert_eq!(
            router.get(ColumnFamily::Base, b"replaced").unwrap(),
            Some(b"new-value".to_vec())
        );

        drop(router);
        let reopened = CfRouter::open(&root.0, 1024 * 1024).expect("reopen router");
        assert_eq!(
            reopened.get(ColumnFamily::Base, b"live").unwrap(),
            Some(b"value".to_vec())
        );
        assert_eq!(reopened.get(ColumnFamily::Base, b"deleted").unwrap(), None);
        assert_eq!(
            reopened.get(ColumnFamily::Base, b"replaced").unwrap(),
            Some(b"new-value".to_vec())
        );
    }
}
