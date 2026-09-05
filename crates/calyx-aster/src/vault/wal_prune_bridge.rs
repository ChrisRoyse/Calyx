//! Ingest-lifecycle WAL segment pruning (issue #1992).
//!
//! Checkpointing advances the durable coverage floor and #1985's flush-SST
//! compaction retires SST debt, but nothing ever deleted the covered WAL
//! segments, so a production vault carried tens of GB of WAL history back to
//! segment index `0` and every open header-scanned all of it. This drives
//! bounded WAL-prune rounds under the same durable commit lock and coverage
//! authority the compactor uses.

use super::AsterVault;
use super::durable::DurableVault;
use crate::wal::{WalPruneConfig, WalPruneOutcome, prune_covered_segments};
use calyx_core::{Clock, Result};

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Retires one bounded round of WAL segments whose highest seq is at or
    /// below the verified durable coverage floor.
    ///
    /// Runs under the durable commit lock: it flushes so the manifest covers
    /// the latest committed seq, derives the coverage floor from
    /// [`Self::verified_durable_coverage_seq`] (the exact authority
    /// `compact_cf_flush_debt` uses — fail-closed if the manifest does not
    /// cover the tip), then deletes a bounded contiguous prefix of covered
    /// segments. The active segment and anything above the floor are never
    /// touched. Non-durable vaults have no WAL and return a no-op outcome.
    pub fn prune_covered_wal_segments(&self, config: WalPruneConfig) -> Result<WalPruneOutcome> {
        config.validate()?;
        let Some(durable) = &self.durable else {
            return Ok(WalPruneOutcome::default());
        };
        self.with_durable_commit_lock(|| self.prune_covered_wal_segments_locked(durable, config))
    }

    fn prune_covered_wal_segments_locked(
        &self,
        durable: &DurableVault,
        config: WalPruneConfig,
    ) -> Result<WalPruneOutcome> {
        // Flush so the manifest durable_seq covers the latest committed seq;
        // `verified_durable_coverage_seq` fails closed otherwise, so a covered
        // segment can never be deleted while its rows are only in the WAL.
        self.flush_locked()?;
        let coverage_floor = self.verified_durable_coverage_seq(durable)?;
        prune_covered_segments(durable.root().join("wal"), coverage_floor, config)
    }
}
