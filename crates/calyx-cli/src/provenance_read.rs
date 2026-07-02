//! Exact CF point readback keyed by row provenance (issue #1096).
//!
//! `Constellation.provenance.seq` is a LEDGER sequence assigned when a put is
//! staged; durable-batch SSTs are named by the WAL COMMIT sequence of the
//! batch that persisted them. The two counters advance independently, so the
//! old near-seq heuristic ("read the SST whose file seq is the row's
//! provenance seq or seq+1") structurally misses on group-committed /
//! bulk-ingested vaults (99,498/99,498 misses on the issue #1096 80G vault).
//!
//! This module resolves reads exactly instead of guessing:
//!
//! 1. **Commit-batch resolution.** Every put stages its
//!    `ledger_key(provenance.seq)` row in the SAME WAL batch as its base and
//!    slot rows, and ledger keys are 8-byte big-endian seqs, so each
//!    durable-batch ledger SST covers one contiguous provenance range while
//!    its file name carries the commit seq. Binary search over ledger SST
//!    footers maps provenance seq -> commit seq, then the row is read from
//!    exactly that batch's SST(s). Footer ranges are cached for the lifetime
//!    of the context, so a full-vault scan costs O(files touched), not
//!    O(files x chunks).
//! 2. **Full-level lookup.** Any key stage 1 cannot resolve is read through
//!    the complete SST set of the CF with first/last-key + bloom pruning
//!    (compacted files included). This is the semantic source of truth for
//!    latest-row reads, not a retry heuristic. Stage 1 is bypassed entirely
//!    for CFs that contain compacted files, because compaction rewrites row
//!    history and only a newest-first full-level read is then correct.
//! 3. **WAL tail overlay.** Rows committed after the manifest floor override
//!    both stages (same semantics as the previous readback paths).
//!
//! Every resolved row carries its [`RowSource`] so callers can log exactly
//! which stage produced it, and a key that no stage resolves is reported as
//! `None` for the caller to fail closed on.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::sst::level::SstLevel;
use calyx_aster::storage_names::{SstName, SstOrderKey, classify_sst, sst_order_key};
use calyx_aster::vault::encode::{WriteRow, decode_write_batch};

/// Which resolution stage produced a row value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RowSource {
    /// Read from the durable-batch SST(s) of the row's own commit batch,
    /// located through the ledger provenance index.
    CommitBatch,
    /// Read through the metadata-pruned full SST level of the CF.
    FullSet,
    /// Read from WAL records newer than the manifest floor.
    WalTail,
}

/// One resolved row value plus the stage that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedRow {
    pub value: Vec<u8>,
    pub source: RowSource,
}

/// Per-call / cumulative stage-hit accounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ProvenanceReadStats {
    pub commit_batch_rows: usize,
    pub full_set_rows: usize,
    pub wal_tail_rows: usize,
    pub unresolved_rows: usize,
}

impl ProvenanceReadStats {
    fn record(&mut self, row: Option<&ResolvedRow>) {
        match row.map(|row| row.source) {
            Some(RowSource::CommitBatch) => self.commit_batch_rows += 1,
            Some(RowSource::FullSet) => self.full_set_rows += 1,
            Some(RowSource::WalTail) => self.wal_tail_rows += 1,
            None => self.unresolved_rows += 1,
        }
    }

    pub(crate) fn accumulate(&mut self, other: ProvenanceReadStats) {
        self.commit_batch_rows += other.commit_batch_rows;
        self.full_set_rows += other.full_set_rows;
        self.wal_tail_rows += other.wal_tail_rows;
        self.unresolved_rows += other.unresolved_rows;
    }
}

/// Result of one batched provenance read.
#[derive(Debug)]
pub(crate) struct ProvenanceReadBatch {
    pub rows: BTreeMap<Vec<u8>, Option<ResolvedRow>>,
    pub stats: ProvenanceReadStats,
}

/// Vault-scoped read context. Caches the ledger provenance index, per-CF file
/// listings/levels, and the decoded WAL tail across calls, so batched scans
/// pay each file-system cost once per vault instead of once per chunk.
pub(crate) struct VaultReadContext {
    vault: PathBuf,
    ledger: Option<LedgerProvenanceIndex>,
    cf_files: BTreeMap<String, CfFiles>,
    wal_tail: Option<Vec<WriteRow>>,
    pub totals: ProvenanceReadStats,
}

impl VaultReadContext {
    pub(crate) fn new(vault: &Path) -> Self {
        Self {
            vault: vault.to_path_buf(),
            ledger: None,
            cf_files: BTreeMap::new(),
            wal_tail: None,
            totals: ProvenanceReadStats::default(),
        }
    }

    /// Reads the latest value for each `(key, provenance_seq)` pair in `cf`.
    /// Every requested key is present in the result map; `None` means no
    /// physical row exists in any resolution stage (commit batch, full SST
    /// level, WAL tail) and the caller decides whether that is fatal.
    pub(crate) fn latest_cf_rows_for_provenance(
        &mut self,
        cf: ColumnFamily,
        keys: &[(Vec<u8>, u64)],
    ) -> Result<ProvenanceReadBatch, String> {
        let mut rows: BTreeMap<Vec<u8>, Option<ResolvedRow>> =
            keys.iter().map(|(key, _)| (key.clone(), None)).collect();
        if rows.is_empty() {
            return Ok(ProvenanceReadBatch {
                rows,
                stats: ProvenanceReadStats::default(),
            });
        }
        if !self.cf_listing(&cf)?.has_compacted {
            self.read_from_commit_batches(&cf, keys, &mut rows)?;
        }
        self.read_unresolved_from_full_level(&cf, &mut rows)?;
        self.overlay_wal_tail(&cf, &mut rows)?;

        let mut stats = ProvenanceReadStats::default();
        for row in rows.values() {
            stats.record(row.as_ref());
        }
        self.totals.accumulate(stats);
        Ok(ProvenanceReadBatch { rows, stats })
    }

    /// Stage 1: resolve provenance seqs to commit seqs through the ledger
    /// provenance index and read each key from its own commit batch SST(s).
    ///
    /// HAZARD (issue #1107): this reads the ORIGINAL commit batch and does not
    /// consult newer batches, which is correct ONLY while slot CF rows are
    /// write-once for live constellations. A future feature that rewrites an
    /// existing slot CF key in a LATER durable batch without compaction would
    /// make this return a stale value. If you add such a feature, land a
    /// guardrail first (tombstone/compact on rewrite, version slot rows, or a
    /// newer-batch overlay here); the tripwire test
    /// `later_batch_slot_rewrite_is_missed_by_stage1_write_once_assumption`
    /// pins the current behavior.
    fn read_from_commit_batches(
        &mut self,
        cf: &ColumnFamily,
        keys: &[(Vec<u8>, u64)],
        rows: &mut BTreeMap<Vec<u8>, Option<ResolvedRow>>,
    ) -> Result<(), String> {
        let mut keys_by_commit_seq = BTreeMap::<u64, Vec<&[u8]>>::new();
        for (key, provenance_seq) in keys {
            let Some(commit_seq) = self.ledger_index()?.resolve(*provenance_seq)? else {
                continue;
            };
            keys_by_commit_seq
                .entry(commit_seq)
                .or_default()
                .push(key.as_slice());
        }
        if keys_by_commit_seq.is_empty() {
            return Ok(());
        }
        let listing = self.cf_listing(cf)?;
        for (commit_seq, seq_keys) in keys_by_commit_seq {
            for path in listing.durable_batch_files_for_seq(commit_seq) {
                let reader = SstReader::open(path).map_err(|error| {
                    format!("open commit-batch SST {}: {error}", path.display())
                })?;
                for key in &seq_keys {
                    if let Some(value) = reader
                        .get(key)
                        .map_err(|error| format!("read {}: {error}", path.display()))?
                    {
                        rows.insert(
                            key.to_vec(),
                            Some(ResolvedRow {
                                value,
                                source: RowSource::CommitBatch,
                            }),
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Stage 2: read every still-unresolved key through the metadata-pruned
    /// full SST level of the CF (newest-first, compacted files included).
    fn read_unresolved_from_full_level(
        &mut self,
        cf: &ColumnFamily,
        rows: &mut BTreeMap<Vec<u8>, Option<ResolvedRow>>,
    ) -> Result<(), String> {
        let unresolved: Vec<Vec<u8>> = rows
            .iter()
            .filter(|(_, row)| row.is_none())
            .map(|(key, _)| key.clone())
            .collect();
        if unresolved.is_empty() {
            return Ok(());
        }
        let level = self.cf_full_level(cf)?;
        for key in unresolved {
            if let Some(value) = level
                .get(&key)
                .map_err(|error| format!("full-level read of CF {}: {error}", cf.name()))?
            {
                rows.insert(
                    key,
                    Some(ResolvedRow {
                        value,
                        source: RowSource::FullSet,
                    }),
                );
            }
        }
        Ok(())
    }

    /// Stage 3: WAL records newer than the manifest floor override everything.
    fn overlay_wal_tail(
        &mut self,
        cf: &ColumnFamily,
        rows: &mut BTreeMap<Vec<u8>, Option<ResolvedRow>>,
    ) -> Result<(), String> {
        if self.wal_tail.is_none() {
            let replay = crate::cf_read::replay_after_manifest(&self.vault)?;
            let mut tail = Vec::new();
            for record in replay.records {
                tail.extend(
                    decode_write_batch(&record.payload).map_err(|error| error.to_string())?,
                );
            }
            self.wal_tail = Some(tail);
        }
        for row in self.wal_tail.as_ref().expect("wal tail cached") {
            if row.cf == *cf && rows.contains_key(&row.key) {
                rows.insert(
                    row.key.clone(),
                    Some(ResolvedRow {
                        value: row.value.clone(),
                        source: RowSource::WalTail,
                    }),
                );
            }
        }
        Ok(())
    }

    fn ledger_index(&mut self) -> Result<&mut LedgerProvenanceIndex, String> {
        if self.ledger.is_none() {
            self.ledger = Some(LedgerProvenanceIndex::open(&self.vault)?);
        }
        Ok(self.ledger.as_mut().expect("ledger index cached"))
    }

    fn cf_listing(&mut self, cf: &ColumnFamily) -> Result<&CfFiles, String> {
        let name = cf.name().to_string();
        if !self.cf_files.contains_key(&name) {
            let listing = CfFiles::list(&self.vault, cf)?;
            self.cf_files.insert(name.clone(), listing);
        }
        Ok(self.cf_files.get(&name).expect("cf listing cached"))
    }

    fn cf_full_level(&mut self, cf: &ColumnFamily) -> Result<&SstLevel, String> {
        let name = cf.name().to_string();
        self.cf_listing(cf)?;
        let needs_level = self
            .cf_files
            .get(&name)
            .expect("cf listing cached")
            .level
            .is_none();
        if needs_level {
            let paths: Vec<PathBuf> = self
                .cf_files
                .get(&name)
                .expect("cf listing cached")
                .files
                .iter()
                .map(|(_, _, path)| path.clone())
                .collect();
            let level = SstLevel::from_oldest_first_with_lookup(paths)
                .map_err(|error| format!("build lookup level for CF {}: {error}", cf.name()))?;
            self.cf_files
                .get_mut(&name)
                .expect("cf listing cached")
                .level = Some(level);
        }
        Ok(self
            .cf_files
            .get(&name)
            .and_then(|listing| listing.level.as_ref())
            .expect("cf level cached"))
    }
}

/// Cached, classified SST listing of one CF directory.
struct CfFiles {
    /// Canonical files sorted oldest-first by `(SstOrderKey, path)`.
    files: Vec<(SstOrderKey, SstName, PathBuf)>,
    has_compacted: bool,
    level: Option<SstLevel>,
}

impl CfFiles {
    fn list(vault: &Path, cf: &ColumnFamily) -> Result<Self, String> {
        let dir = vault.join("cf").join(cf.name());
        let mut files = Vec::new();
        let mut has_compacted = false;
        if dir.exists() {
            for entry in fs::read_dir(&dir)
                .map_err(|error| format!("read CF dir {}: {error}", dir.display()))?
            {
                let path = entry
                    .map_err(|error| format!("read CF dir entry in {}: {error}", dir.display()))?
                    .path();
                let Some(name) = classify_sst(&path).map_err(|error| error.to_string())? else {
                    continue;
                };
                if matches!(name, SstName::Compacted { .. }) {
                    has_compacted = true;
                }
                let order = sst_order_key(&path)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("classified SST {} has no order key", path.display()))?;
                files.push((order, name, path));
            }
        }
        files.sort_by(|(left_order, _, left_path), (right_order, _, right_path)| {
            left_order
                .cmp(right_order)
                .then_with(|| left_path.cmp(right_path))
        });
        Ok(Self {
            files,
            has_compacted,
            level: None,
        })
    }

    fn durable_batch_files_for_seq(&self, commit_seq: u64) -> impl Iterator<Item = &PathBuf> {
        self.files.iter().filter_map(move |(_, name, path)| {
            matches!(name, SstName::DurableBatch { seq, .. } if *seq == commit_seq).then_some(path)
        })
    }
}

/// Binary-searchable provenance-seq -> commit-seq index over the ledger CF's
/// durable-batch SSTs. Footer key ranges are loaded lazily and cached.
struct LedgerProvenanceIndex {
    /// Durable-batch ledger files sorted oldest-first, with their commit seq
    /// and lazily-loaded `(first_provenance_seq, last_provenance_seq)` range.
    files: Vec<LedgerFile>,
}

struct LedgerFile {
    path: PathBuf,
    commit_seq: u64,
    range: Option<(u64, u64)>,
}

impl LedgerProvenanceIndex {
    fn open(vault: &Path) -> Result<Self, String> {
        let listing = CfFiles::list(vault, &ColumnFamily::Ledger)?;
        let files = listing
            .files
            .into_iter()
            .filter_map(|(_, name, path)| match name {
                // Router flushes span many commit batches and compaction
                // outputs are not named by a commit seq: neither can map a
                // provenance seq to the batch that wrote it.
                SstName::DurableBatch { seq, .. } => Some(LedgerFile {
                    path,
                    commit_seq: seq,
                    range: None,
                }),
                SstName::Router { .. } | SstName::Compacted { .. } => None,
            })
            .collect();
        Ok(Self { files })
    }

    /// Maps a provenance seq to the commit seq of the batch that wrote it, or
    /// `None` when no durable-batch ledger SST covers the seq (the caller
    /// then reads through the full level, which stays authoritative).
    fn resolve(&mut self, provenance_seq: u64) -> Result<Option<u64>, String> {
        let mut lo = 0usize;
        let mut hi = self.files.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (first, last) = self.range(mid)?;
            if provenance_seq < first {
                hi = mid;
            } else if provenance_seq > last {
                lo = mid + 1;
            } else {
                return Ok(Some(self.files[mid].commit_seq));
            }
        }
        Ok(None)
    }

    fn range(&mut self, index: usize) -> Result<(u64, u64), String> {
        if let Some(range) = self.files[index].range {
            return Ok(range);
        }
        let path = &self.files[index].path;
        let reader = SstReader::open(path)
            .map_err(|error| format!("open ledger SST {}: {error}", path.display()))?;
        let (first_key, last_key) = reader.key_range().ok_or_else(|| {
            format!(
                "ledger SST {} has no keys; refusing to use an empty ledger file as a provenance index",
                path.display()
            )
        })?;
        let range = (
            decode_ledger_seq(path, first_key)?,
            decode_ledger_seq(path, last_key)?,
        );
        self.files[index].range = Some(range);
        Ok(range)
    }
}

fn decode_ledger_seq(path: &Path, key: &[u8]) -> Result<u64, String> {
    let bytes: [u8; 8] = key.try_into().map_err(|_| {
        format!(
            "ledger SST {} contains a non-canonical ledger key ({} bytes, expected the 8-byte \
             big-endian seq written by ledger_key); the ledger CF cannot serve as a provenance \
             index for this vault",
            path.display(),
            key.len()
        )
    })?;
    Ok(u64::from_be_bytes(bytes))
}
