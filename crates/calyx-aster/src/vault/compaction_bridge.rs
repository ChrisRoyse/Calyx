use super::AsterVault;
use super::durable::DurableVault;
use crate::cf::ColumnFamily;
use crate::compaction::{
    COMPACTION_ADOPTION_FIRST_INDEX, COMPACTION_ADOPTION_LAST_INDEX, CompactionCatalog,
    CompactionResult, CompactionScheduler, CompactionSchedulerOptions, CompactionThrottle,
    DEFAULT_COMPACTION_TARGET_BYTES, RollingSstWriter, SstShard, catalog_from_vault_tiers,
    commit_domain_output_path, compact_shards, durable_compaction_slot_path,
};
use crate::mvcc::is_tombstone_value;
use crate::recurrence::{StoredRecurrenceRow, decode_recurrence_row};
use crate::sst::{invalidate_reader, shared_reader};
use crate::storage_names::{SstName, SstOrderKey, classify_sst, sst_order_key};
use calyx_core::{CalyxError, Clock, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Tunable knobs for ingest-lifecycle flush-SST compaction (issue #1985).
///
/// `calyx ingest` historically never compacted, so a large batch build left
/// millions of uncompacted per-commit flush SSTs and read paths degraded
/// toward O(files). Batch ingest now measures per-CF flush-SST debt at
/// chunk-commit boundaries and drives bounded compaction rounds under the
/// durable commit lock whenever debt crosses `trigger_files`.
#[derive(Debug, Clone, Copy)]
pub struct IngestCompactionConfig {
    /// Compact a CF once its count of below-target ("flush debt") SSTs reaches
    /// this many. Below-target means smaller than
    /// [`DEFAULT_COMPACTION_TARGET_BYTES`]; a full merge collapses those tiny
    /// per-commit files into at-target LSM files, so this bound is achievable
    /// regardless of total CF data volume.
    pub trigger_files: usize,
    /// Hard cap on the number of input SSTs merged in a single bounded round,
    /// so a CF with hundreds of thousands of files compacts over successive
    /// rounds instead of opening every reader at once.
    pub max_round_files: usize,
    /// Hard cap on total input bytes merged in a single bounded round.
    pub max_round_bytes: u64,
}

impl IngestCompactionConfig {
    /// Fails closed on nonsensical knobs so a misconfigured trigger can never
    /// silently disable compaction or request an empty round (issue #1985).
    pub fn validate(&self) -> Result<()> {
        if self.trigger_files == 0 {
            return Err(CalyxError {
                code: "CALYX_INGEST_COMPACTION_CONFIG_INVALID",
                message: "ingest compaction trigger_files must be >= 1".to_string(),
                remediation: "set CALYX_INGEST_COMPACTION_TRIGGER_FILES to a positive integer",
            });
        }
        if self.max_round_files < 2 {
            return Err(CalyxError {
                code: "CALYX_INGEST_COMPACTION_CONFIG_INVALID",
                message: format!(
                    "ingest compaction max_round_files must be >= 2 (got {})",
                    self.max_round_files
                ),
                remediation: "set CALYX_INGEST_COMPACTION_ROUND_MAX_FILES to an integer >= 2 (a round must merge at least two SSTs)",
            });
        }
        if self.max_round_bytes == 0 {
            return Err(CalyxError {
                code: "CALYX_INGEST_COMPACTION_CONFIG_INVALID",
                message: "ingest compaction max_round_bytes must be >= 1".to_string(),
                remediation: "set CALYX_INGEST_COMPACTION_ROUND_MAX_BYTES to a positive integer",
            });
        }
        Ok(())
    }
}

/// Per-CF result of an ingest-lifecycle compaction pass, reported so the CLI
/// can log per-CF before/after file counts (issue #1985 §3).
#[derive(Debug, Clone)]
pub struct CfCompactionOutcome {
    pub cf: ColumnFamily,
    pub files_before: usize,
    pub files_after: usize,
    pub debt_before: usize,
    pub debt_after: usize,
    pub rounds: usize,
    pub reclaimed_input_files: usize,
    pub elapsed_ms: u128,
}

/// One bounded compaction round's before/after accounting.
#[derive(Debug, Clone, Copy)]
struct DebtRound {
    compacted: bool,
    reclaimed_input_files: usize,
    files_before: usize,
    files_after: usize,
    debt_before: usize,
    debt_after: usize,
}

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
        self.with_durable_commit_lock(|| {
            self.flush_locked()?;
            Ok(Some(Arc::new(catalog_from_vault_tiers(
                durable.root(),
                durable.tiering_policy(),
            )?)))
        })
    }

    pub fn compact_cf_once(&self, cf: ColumnFamily) -> Result<Option<CompactionResult>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        self.with_durable_commit_lock(|| {
            self.flush_locked()?;
            let durable_seq = self.verified_durable_coverage_seq(durable)?;
            let catalog = catalog_from_vault_tiers(durable.root(), durable.tiering_policy())?;
            let output = durable.compaction_output_path(cf, durable_seq);
            let mut result = catalog
                .compact_cf(cf, output, CompactionThrottle::unlimited())
                .map(Some)?;
            if let Some(CompactionResult::Compacted(report)) = &mut result
                && cf == ColumnFamily::Recurrence
            {
                ensure_reclaim_outputs_manifest_bounded(&report.output_paths, durable_seq)?;
                report.reclaimed_input_files = reclaim_recurrence_inputs(report)?;
                prune_recurrence_tombstones(report)?;
                // The resident router level still references the deleted
                // input SSTs; rebuild it from disk so live reads stop
                // touching reclaimed files.
                self.rows.reload_router_cf_level(cf)?;
            }
            Ok(result)
        })
    }

    /// Manifest durable coverage after a locked flush; fails closed when the
    /// manifest does not cover the latest committed seq, because naming a
    /// compaction output beyond `durable_seq` makes full-restore readback
    /// skip it while its inputs get reclaimed (issue #1132).
    pub(super) fn verified_durable_coverage_seq(&self, durable: &DurableVault) -> Result<u64> {
        let durable_seq = durable.manifest_durable_seq()?;
        let latest = self.latest_seq();
        if durable_seq < latest {
            return Err(CalyxError {
                code: "CALYX_ASTER_COMPACTION_COVERAGE_GAP",
                message: format!(
                    "manifest durable_seq {durable_seq} does not cover latest committed seq {latest} after flush; compacting now would strand rows invisible to full-restore opens"
                ),
                remediation: "flush the vault and retry; if the gap persists, the WAL tail was not re-staged for checkpointing — report with vault MANIFEST and wal/ listing",
            });
        }
        Ok(durable_seq)
    }

    /// Compacts the listed column families, prunes MVCC tombstone rows from the
    /// compacted SST, and reclaims superseded input SSTs for durable vaults.
    pub fn purge_tombstoned_cfs(&self, cfs: &[ColumnFamily]) -> Result<()> {
        self.with_durable_commit_lock(|| self.purge_tombstoned_cfs_locked(cfs))
    }

    pub(crate) fn purge_tombstoned_cfs_locked(&self, cfs: &[ColumnFamily]) -> Result<()> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        self.flush_locked()?;
        let durable_seq = self.verified_durable_coverage_seq(durable)?;
        let mut unique = Vec::new();
        for cf in cfs {
            if !unique.contains(cf) {
                unique.push(*cf);
            }
        }
        for cf in unique {
            purge_tombstoned_cf_once(durable.root(), durable.tiering_policy(), cf, durable_seq)?;
            // See compact_cf_once: the reclaim above deleted input SSTs the
            // resident router level may still reference.
            self.rows.reload_router_cf_level(cf)?;
        }
        Ok(())
    }

    pub fn start_compaction_scheduler(
        &self,
        mut options: CompactionSchedulerOptions,
    ) -> Result<Option<VaultCompactionScheduler>> {
        if let Some(durable) = &self.durable
            && options.output_root == CompactionSchedulerOptions::default().output_root
        {
            options.output_root = durable.root().join("cf");
        }
        if let Some(durable) = &self.durable {
            options.tiering_policy = options
                .tiering_policy
                .or_else(|| durable.tiering_policy().cloned());
        }
        let Some(catalog) = self.compaction_catalog()? else {
            return Ok(None);
        };
        let scheduler = CompactionScheduler::start(catalog.clone(), options);
        Ok(Some(VaultCompactionScheduler { catalog, scheduler }))
    }

    /// Drives ingest-lifecycle flush-SST compaction across every CF that
    /// currently holds files, returning a per-CF outcome (issue #1985).
    ///
    /// Each CF is compacted in bounded rounds until its below-target flush-SST
    /// debt drops to at most `config.trigger_files`. Used both at chunk-commit
    /// boundaries (to keep debt bounded) and as the final barrier before the
    /// ingest summary (so ingest never exits leaving a CF above the trigger).
    /// Non-durable vaults have no on-disk SSTs and return an empty vector.
    pub fn compact_ingest_flush_debt(
        &self,
        config: IngestCompactionConfig,
    ) -> Result<Vec<CfCompactionOutcome>> {
        config.validate()?;
        let Some(durable) = &self.durable else {
            return Ok(Vec::new());
        };
        // One locked flush + catalog build to learn which CFs have files;
        // each CF is then compacted with its own locked rounds.
        let cfs = self.with_durable_commit_lock(|| {
            self.flush_locked()?;
            let catalog = catalog_from_vault_tiers(durable.root(), durable.tiering_policy())?;
            Ok(catalog.column_families())
        })?;
        let mut outcomes = Vec::with_capacity(cfs.len());
        for cf in cfs {
            outcomes.push(self.compact_cf_flush_debt(cf, config)?);
        }
        Ok(outcomes)
    }

    /// Runs bounded compaction rounds for a single CF until its below-target
    /// flush-SST debt is at most `config.trigger_files`, nothing remains to
    /// merge, or a round fails to make progress (fail-closed, issue #1985).
    pub fn compact_cf_flush_debt(
        &self,
        cf: ColumnFamily,
        config: IngestCompactionConfig,
    ) -> Result<CfCompactionOutcome> {
        config.validate()?;
        let started = Instant::now();
        let mut rounds = 0usize;
        let mut reclaimed_input_files = 0usize;
        let mut files_before = 0usize;
        let mut debt_before = 0usize;
        let mut files_after;
        let mut debt_after;
        let mut first = true;
        loop {
            let round = self.compact_cf_flush_debt_round(cf, config)?;
            if first {
                files_before = round.files_before;
                debt_before = round.debt_before;
                first = false;
            }
            files_after = round.files_after;
            debt_after = round.debt_after;
            if round.compacted {
                rounds += 1;
                reclaimed_input_files =
                    reclaimed_input_files.saturating_add(round.reclaimed_input_files);
            }
            if round.debt_after <= config.trigger_files || !round.compacted {
                break;
            }
            // A round that merged >=2 files but did not shrink debt cannot make
            // further progress; fail loud rather than spin (issue #1985 §2).
            if round.debt_after >= round.debt_before {
                return Err(CalyxError {
                    code: "CALYX_INGEST_COMPACTION_NO_PROGRESS",
                    message: format!(
                        "compaction round for CF {} made no debt progress: below-target files {} -> {} (files {} -> {}); trigger {}",
                        cf.name(),
                        round.debt_before,
                        round.debt_after,
                        round.files_before,
                        round.files_after,
                        config.trigger_files
                    ),
                    remediation: "inspect the CF's SST layout with `calyx ops compact`; a stalled round usually means oversized rows or a corrupt shard preventing the merge",
                });
            }
            // Bound total rounds by the initial file count as a runaway guard.
            if rounds > files_before.saturating_add(2) {
                return Err(CalyxError {
                    code: "CALYX_INGEST_COMPACTION_ROUND_LIMIT",
                    message: format!(
                        "compaction for CF {} exceeded {} rounds without reaching trigger {} (debt still {})",
                        cf.name(),
                        rounds,
                        config.trigger_files,
                        round.debt_after
                    ),
                    remediation: "report this vault/CF; the bounded compaction loop is not converging",
                });
            }
        }
        Ok(CfCompactionOutcome {
            cf,
            files_before,
            files_after,
            debt_before,
            debt_after,
            rounds,
            reclaimed_input_files,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    /// One bounded compaction round for `cf`, holding the durable commit lock
    /// for the whole flush -> merge -> reclaim -> router-reload sequence so it
    /// is atomic against other writers.
    fn compact_cf_flush_debt_round(
        &self,
        cf: ColumnFamily,
        config: IngestCompactionConfig,
    ) -> Result<DebtRound> {
        let Some(durable) = &self.durable else {
            return Ok(DebtRound {
                compacted: false,
                reclaimed_input_files: 0,
                files_before: 0,
                files_after: 0,
                debt_before: 0,
                debt_after: 0,
            });
        };
        self.with_durable_commit_lock(|| {
            self.flush_locked()?;
            let durable_seq = self.verified_durable_coverage_seq(durable)?;
            let catalog = catalog_from_vault_tiers(durable.root(), durable.tiering_policy())?;
            let shards = catalog.shards_for_cf(cf);
            let target = DEFAULT_COMPACTION_TARGET_BYTES;
            let files_before = shards.len();
            let debt_before = flush_debt_count(&shards, target)?;
            let skipped = DebtRound {
                compacted: false,
                reclaimed_input_files: 0,
                files_before,
                files_after: files_before,
                debt_before,
                debt_after: debt_before,
            };
            // Only compact once debt crosses the trigger; below it the CF's
            // per-commit files are left alone (issue #1985 §1).
            if debt_before <= config.trigger_files {
                return Ok(skipped);
            }
            let Some(range) = ingest_debt_round_range(
                &shards,
                target,
                config.max_round_files,
                config.max_round_bytes,
            )?
            else {
                return Ok(skipped);
            };
            let inputs = &shards[range];
            let output_dir =
                ingest_compaction_output_dir(durable.root(), durable.tiering_policy(), cf);
            // Named at the highest commit seq present in the merged inputs so
            // the output sits at the correct read-order position and stays
            // covered by the manifest (durable_seq >= every merged flush's
            // watermark after the locked flush; issues #1132/#1137).
            let output = commit_domain_output_path(&output_dir, inputs)?;
            let mut result = compact_shards(cf, inputs, &output, CompactionThrottle::unlimited())?;
            let CompactionResult::Compacted(report) = &mut result else {
                return Ok(skipped);
            };
            ensure_reclaim_outputs_manifest_bounded(&report.output_paths, durable_seq)?;
            report.reclaimed_input_files = reclaim_compaction_inputs(report)?;
            // The resident router level still references the reclaimed inputs;
            // rebuild it from disk so live reads stop touching deleted files
            // (same invariant compact_cf_once relies on).
            self.rows.reload_router_cf_level(cf)?;
            let inputs_debt = flush_debt_count(inputs, target)?;
            let outputs_debt = flush_debt_count_paths(&report.output_paths, target)?;
            let files_after = files_before
                .saturating_sub(inputs.len())
                .saturating_add(report.output_paths.len());
            let debt_after = debt_before
                .saturating_sub(inputs_debt)
                .saturating_add(outputs_debt);
            Ok(DebtRound {
                compacted: true,
                reclaimed_input_files: report.reclaimed_input_files,
                files_before,
                files_after,
                debt_before,
                debt_after,
            })
        })
    }
}

/// Number of shards whose file is below the compaction target — the
/// accumulating per-commit "flush debt" that the ingest lifecycle compacts.
fn flush_debt_count(shards: &[SstShard], target: u64) -> Result<usize> {
    let mut count = 0;
    for shard in shards {
        if is_flush_debt(&shard.path, shard.bytes, target)? {
            count += 1;
        }
    }
    Ok(count)
}

/// Flush-debt count for freshly written output paths (stat each; there are
/// only a handful of outputs per round). Compaction-adoption outputs are
/// deliberately not debt: rolling outputs are target-sized immutable LSM
/// files even when their exact encoded length is a few bytes below target.
fn flush_debt_count_paths(paths: &[PathBuf], target: u64) -> Result<usize> {
    let mut count = 0;
    for path in paths {
        let bytes = fs::metadata(path)
            .map_err(|error| {
                CalyxError::disk_pressure(format!(
                    "stat compaction output {}: {error}",
                    path.display()
                ))
            })?
            .len();
        if is_flush_debt(path, bytes, target)? {
            count += 1;
        }
    }
    Ok(count)
}

fn is_flush_debt(path: &Path, bytes: u64, target: u64) -> Result<bool> {
    if bytes >= target {
        return Ok(false);
    }
    let name = classify_sst(path)?.ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!(
            "compaction debt input {} has no canonical SST name",
            path.display()
        ))
    })?;
    Ok(match name {
        SstName::RouterLegacy { .. } | SstName::Flush { .. } => true,
        SstName::DurableBatch { index, .. } => {
            !(COMPACTION_ADOPTION_FIRST_INDEX..=COMPACTION_ADOPTION_LAST_INDEX).contains(&index)
        }
        SstName::Compacted { .. } => false,
    })
}

/// Commit-seq group identity for seq-boundary snapping: a `(epoch, seq)` pair.
/// SSTs sharing this key are one commit domain group and must never be split
/// across a round boundary, or the merged output could share a commit seq with
/// a left-behind file and make newest-wins read order ambiguous (issue #1138).
fn seq_group_key(key: &SstOrderKey) -> (u8, u64) {
    (key.epoch, key.seq)
}

/// Chooses the contiguous input range for one bounded compaction round.
///
/// Shards arrive oldest-first (catalog read order). The range starts at the
/// oldest below-target file (the head of accumulated flush debt), is snapped
/// back to a commit-seq-group boundary, extended forward under the file/byte
/// budget, and snapped forward to a commit-seq-group boundary. Snapping to
/// group boundaries guarantees no file left outside the range shares the
/// merged output's commit seq, so read order stays unambiguous. Returns `None`
/// when fewer than two files would be merged (nothing worth doing).
fn ingest_debt_round_range(
    shards: &[SstShard],
    target: u64,
    max_files: usize,
    max_bytes: u64,
) -> Result<Option<std::ops::Range<usize>>> {
    if shards.len() < 2 {
        return Ok(None);
    }
    let keys = shards
        .iter()
        .map(|shard| {
            sst_order_key(&shard.path)?.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "compaction shard {} has no canonical SST order key",
                    shard.path.display()
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut first_debt = None;
    for (index, shard) in shards.iter().enumerate() {
        if is_flush_debt(&shard.path, shard.bytes, target)? {
            first_debt = Some(index);
            break;
        }
    }
    let Some(first_debt) = first_debt else {
        return Ok(None);
    };
    let mut start = first_debt;
    while start > 0 && seq_group_key(&keys[start - 1]) == seq_group_key(&keys[start]) {
        start -= 1;
    }
    let mut end = start;
    let mut files = 0usize;
    let mut bytes = 0u64;
    while end < shards.len() && files < max_files && bytes < max_bytes {
        bytes = bytes.saturating_add(shards[end].bytes);
        files += 1;
        end += 1;
    }
    if end < shards.len() {
        let last = seq_group_key(&keys[end - 1]);
        while end < shards.len() && seq_group_key(&keys[end]) == last {
            end += 1;
        }
    }
    if end - start < 2 {
        return Ok(None);
    }
    Ok(Some(start..end))
}

/// Tier-aware CF directory a compaction output should be written to (mirrors
/// `purge_tombstoned_cf_once` / `DurableVault::compaction_output_path`).
fn ingest_compaction_output_dir(
    root: &Path,
    tiering_policy: Option<&crate::compaction::TieringPolicy>,
    cf: ColumnFamily,
) -> PathBuf {
    tiering_policy.map_or_else(
        || root.join("cf").join(cf.name()),
        |policy| policy.place_current_cf(cf).absolute_dir(),
    )
}

fn purge_tombstoned_cf_once(
    root: &Path,
    tiering_policy: Option<&crate::compaction::TieringPolicy>,
    cf: ColumnFamily,
    seq: u64,
) -> Result<()> {
    let catalog = catalog_from_vault_tiers(root, tiering_policy)?;
    let output = tiering_policy.map_or_else(
        || root.join("cf").join(cf.name()),
        |policy| policy.place_current_cf(cf).absolute_dir(),
    );
    let output = output.join(format!("compacted-{seq:020}.sst"));
    let mut result = catalog.compact_cf(cf, output, CompactionThrottle::unlimited())?;
    let CompactionResult::Compacted(report) = &mut result else {
        return Ok(());
    };
    prune_mvcc_tombstones(report)?;
    ensure_reclaim_outputs_manifest_bounded(&report.output_paths, seq)?;
    report.reclaimed_input_files = reclaim_compaction_inputs(report)?;
    Ok(())
}

/// Fails closed before input reclaim when the compaction output would not be
/// visible to full-restore readback (`seq > manifest durable_seq` is skipped
/// by `read_manifested_batches`), which would silently erase the merged rows
/// from every full-restore open once the inputs are deleted (issue #1132).
pub(super) fn ensure_reclaim_output_manifest_bounded(
    output_path: &Path,
    manifest_durable_seq: u64,
) -> Result<()> {
    let bounded = matches!(
        classify_sst(output_path)?,
        Some(SstName::Compacted { seq } | SstName::DurableBatch { seq, .. })
            if seq <= manifest_durable_seq
    );
    if bounded {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_ASTER_COMPACTION_COVERAGE_GAP",
        message: format!(
            "refusing to reclaim compaction inputs: output {} is not covered by manifest durable_seq {manifest_durable_seq}, so full-restore readback would silently skip the merged rows",
            output_path.display()
        ),
        remediation: "flush the vault so the manifest covers the compaction output seq, then retry; inputs were preserved",
    })
}

pub(super) fn ensure_reclaim_outputs_manifest_bounded(
    output_paths: &[PathBuf],
    manifest_durable_seq: u64,
) -> Result<()> {
    for output_path in output_paths {
        ensure_reclaim_output_manifest_bounded(output_path, manifest_durable_seq)?;
    }
    Ok(())
}

fn reclaim_compaction_inputs(report: &crate::compaction::CompactionReport) -> Result<usize> {
    let outputs = canonical_output_paths(report, "stat compacted SST")?;
    let mut reclaimed = 0;
    for input in &report.input_paths {
        let canonical = match fs::canonicalize(input) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if outputs.contains(&canonical) {
            continue;
        }
        if classify_sst(&canonical)?.is_none() {
            continue;
        }
        // Drop any cached shared reader first. On Windows an active mapping
        // keeps DeleteFile failing (fail-closed, same as before this cache);
        // invalidating shrinks that window to reads still holding a clone.
        // Readers cache by the vault-derived path, so invalidate that exact key
        // as well as the canonicalized form; the reader cache no longer
        // canonicalizes, so the two must be dropped explicitly (#1986).
        invalidate_reader(input);
        invalidate_reader(&canonical);
        fs::remove_file(&canonical).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "reclaim compaction input {}: {error}",
                canonical.display()
            ))
        })?;
        reclaimed += 1;
    }
    Ok(reclaimed)
}

fn reclaim_recurrence_inputs(report: &crate::compaction::CompactionReport) -> Result<usize> {
    let outputs = canonical_output_paths(report, "stat compacted SST")?;
    let parent = fs::canonicalize(&report.staging_parent)
        .map_err(|error| CalyxError::disk_pressure(format!("stat compaction parent: {error}")))?;
    let mut reclaimed = 0;
    for input in &report.input_paths {
        let canonical = match fs::canonicalize(input) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if outputs.contains(&canonical) {
            continue;
        }
        if canonical.parent() != Some(parent.as_path()) {
            continue;
        }
        if canonical.extension().and_then(|value| value.to_str()) != Some("sst") {
            continue;
        }
        // See reclaim_compaction_inputs: invalidate before delete so a live
        // mapping cannot block reclaim beyond in-flight reads. Invalidate both
        // the vault-derived key readers cached under and the canonical form
        // (the reader cache no longer canonicalizes, #1986).
        invalidate_reader(input);
        invalidate_reader(&canonical);
        fs::remove_file(&canonical).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "reclaim recurrence compaction input {}: {error}",
                canonical.display()
            ))
        })?;
        reclaimed += 1;
    }
    Ok(reclaimed)
}

fn canonical_output_paths(
    report: &crate::compaction::CompactionReport,
    context: &str,
) -> Result<Vec<PathBuf>> {
    report
        .output_paths
        .iter()
        .map(|path| {
            fs::canonicalize(path)
                .map_err(|error| CalyxError::disk_pressure(format!("{context}: {error}")))
        })
        .collect()
}

fn prune_mvcc_tombstones(report: &mut crate::compaction::CompactionReport) -> Result<()> {
    rewrite_compacted_without(
        report,
        |value| Ok(is_tombstone_value(value)),
        "mvcc tombstone",
    )
}

fn prune_recurrence_tombstones(report: &mut crate::compaction::CompactionReport) -> Result<()> {
    rewrite_compacted_without(
        report,
        |value| {
            Ok(matches!(
                decode_recurrence_row(value)?,
                StoredRecurrenceRow::Tombstone { .. }
            ))
        },
        "recurrence tombstone",
    )
}

fn rewrite_compacted_without(
    report: &mut crate::compaction::CompactionReport,
    should_prune: impl Fn(&[u8]) -> Result<bool>,
    reason: &str,
) -> Result<()> {
    let mut pruned = 0_u64;
    let original_outputs = report.output_paths.clone();
    for output_path in &original_outputs {
        for entry in shared_reader(output_path)?.iter()? {
            if should_prune(&entry.value)? {
                pruned += 1;
            }
        }
    }
    if pruned == 0 {
        return Ok(());
    }

    let seq = compaction_output_seq(&report.output_path)?;
    let reclaimed_path = durable_compaction_slot_path(&report.staging_parent, seq)?;
    let mut writer = RollingSstWriter::new(&reclaimed_path, DEFAULT_COMPACTION_TARGET_BYTES)?;
    let mut retained = 0_u64;
    let mut logical_bytes = 0_u64;
    for output_path in &original_outputs {
        for entry in shared_reader(output_path)?.iter()? {
            if should_prune(&entry.value)? {
                continue;
            }
            logical_bytes = logical_bytes.saturating_add(entry.value.len() as u64);
            retained = retained.saturating_add(1);
            writer.push(entry.key, entry.value)?;
        }
    }
    let summaries = writer.finish(retained == 0)?;

    for output_path in &original_outputs {
        // These pre-rewrite outputs may have been opened via the shared reader
        // cache (e.g. by a concurrent read); invalidate before deleting so a
        // live mapping cannot block the rewrite's reclaim.
        invalidate_reader(output_path);
        fs::remove_file(output_path).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "remove {reason} compaction file {}: {error}",
                output_path.display()
            ))
        })?;
    }
    report.output_paths = summaries
        .iter()
        .map(|summary| summary.path.clone())
        .collect::<Vec<_>>();
    report.output_path = report
        .output_paths
        .first()
        .cloned()
        .ok_or_else(|| CalyxError::disk_pressure("tombstone rewrite produced no output SST"))?;
    report.output_bytes = summaries
        .iter()
        .map(|summary| summary.bytes)
        .fold(0_u64, u64::saturating_add);
    report.logical_bytes = logical_bytes;
    report.write_amp_milli =
        report.output_bytes.saturating_mul(1_000) / report.logical_bytes.max(1);
    report.debt_after = crate::compaction::CompactionDebt::measure(
        &summaries
            .iter()
            .map(|summary| crate::compaction::SstShard {
                cf: report.cf,
                path: summary.path.clone(),
                level: 0,
                bytes: summary.bytes,
            })
            .collect::<Vec<_>>(),
        DEFAULT_COMPACTION_TARGET_BYTES,
    );
    Ok(())
}

fn compaction_output_seq(path: &Path) -> Result<u64> {
    match classify_sst(path)? {
        Some(SstName::Compacted { seq } | SstName::DurableBatch { seq, .. }) => Ok(seq),
        Some(SstName::RouterLegacy { .. } | SstName::Flush { .. }) | None => {
            Err(CalyxError::aster_corrupt_shard(format!(
                "unexpected compacted SST name {}",
                path.display()
            )))
        }
    }
}
