use std::path::PathBuf;

use calyx_aster::cf::ColumnFamily;
use calyx_core::{Clock, Result};

use crate::{ArtifactPtr, BudgetHandle};

use super::artifact::{artifact_bytes, artifact_hash, source_rows, write_artifact};
use super::{AsterRebuildSource, MvccSnapshot, RebuildTarget, Rebuilder, invalid_target};

pub struct AnnIndexRebuilder<'a, C>
where
    C: Clock,
{
    source: AsterRebuildSource<'a, C>,
    artifact_dir: PathBuf,
    derived_probe: Option<ColumnFamily>,
}

impl<'a, C> AnnIndexRebuilder<'a, C>
where
    C: Clock,
{
    pub fn new(source: AsterRebuildSource<'a, C>, artifact_dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            artifact_dir: artifact_dir.into(),
            derived_probe: None,
        }
    }

    pub fn with_derived_probe_for_test(mut self, cf: ColumnFamily) -> Self {
        self.derived_probe = Some(cf);
        self
    }
}

impl<C> Rebuilder for AnnIndexRebuilder<'_, C>
where
    C: Clock,
{
    fn rebuild(
        &self,
        target: &RebuildTarget,
        snapshot: MvccSnapshot,
        budget: &mut BudgetHandle,
    ) -> Result<ArtifactPtr> {
        let RebuildTarget::AnnIndex { slot_id } = target else {
            return Err(invalid_target("AnnIndexRebuilder received non-ANN target"));
        };
        if let Some(cf) = self.derived_probe {
            self.source.scan_cf(snapshot, cf)?;
        }
        let rows = source_rows(
            &[
                ("base", self.source.scan_cf(snapshot, ColumnFamily::Base)?),
                (
                    "slot",
                    self.source
                        .scan_cf(snapshot, ColumnFamily::slot(*slot_id))?,
                ),
            ],
            budget,
        )?;
        let bytes = artifact_bytes("ann_index_v1", target, snapshot, &rows)?;
        write_artifact(&self.artifact_dir, "ann", target, &bytes).map(ArtifactPtr::HnswGraphPath)
    }
}

pub struct KernelIndexRebuilder<'a, C>
where
    C: Clock,
{
    source: AsterRebuildSource<'a, C>,
}

impl<'a, C> KernelIndexRebuilder<'a, C>
where
    C: Clock,
{
    pub const fn new(source: AsterRebuildSource<'a, C>) -> Self {
        Self { source }
    }
}

impl<C> Rebuilder for KernelIndexRebuilder<'_, C>
where
    C: Clock,
{
    fn rebuild(
        &self,
        target: &RebuildTarget,
        snapshot: MvccSnapshot,
        budget: &mut BudgetHandle,
    ) -> Result<ArtifactPtr> {
        let RebuildTarget::KernelIndex { .. } = target else {
            return Err(invalid_target(
                "KernelIndexRebuilder received non-kernel target",
            ));
        };
        let rows = source_rows(
            &[("base", self.source.scan_cf(snapshot, ColumnFamily::Base)?)],
            budget,
        )?;
        Ok(ArtifactPtr::QuantLevelRecordHash(artifact_hash(
            "kernel_index_v1",
            target,
            snapshot,
            &rows,
        )?))
    }
}

pub struct GuardProfileRebuilder<'a, C>
where
    C: Clock,
{
    source: AsterRebuildSource<'a, C>,
}

impl<'a, C> GuardProfileRebuilder<'a, C>
where
    C: Clock,
{
    pub const fn new(source: AsterRebuildSource<'a, C>) -> Self {
        Self { source }
    }
}

impl<C> Rebuilder for GuardProfileRebuilder<'_, C>
where
    C: Clock,
{
    fn rebuild(
        &self,
        target: &RebuildTarget,
        snapshot: MvccSnapshot,
        budget: &mut BudgetHandle,
    ) -> Result<ArtifactPtr> {
        let RebuildTarget::GuardProfile { .. } = target else {
            return Err(invalid_target(
                "GuardProfileRebuilder received non-guard target",
            ));
        };
        let rows = source_rows(
            &[(
                "anchors",
                self.source.scan_cf(snapshot, ColumnFamily::Anchors)?,
            )],
            budget,
        )?;
        Ok(ArtifactPtr::ConfigCacheKeyHash(artifact_hash(
            "guard_profile_v1",
            target,
            snapshot,
            &rows,
        )?))
    }
}
