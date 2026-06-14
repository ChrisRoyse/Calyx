//! Physical retention reclaimer for time-series points and blob rows (issue #591).
//!
//! Time-series writes already maintain rollup rows in the same commit as raw
//! points. This reclaimer tombstones raw point rows past the collection retention
//! horizon while preserving rollups, then can invoke tombstone-purging
//! compaction for durable vaults. Blob retention uses the manifest
//! `created_at_ms` written by the blob layer; legacy manifests without a
//! timestamp are skipped rather than guessed.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use calyx_core::{CalyxError, Clock, Result};
use serde::Serialize;

use crate::cf::ColumnFamily;
use crate::collection::{Collection, CollectionMode, RetentionPolicy};
use crate::layers::blob::{self, BlobId, BlobManifest};
use crate::layers::timeseries;
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;

pub const CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION: &str =
    "CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION";

const TS_DISC: u8 = 0x04;
const TS_KIND_POINT: u8 = 0x00;
const TS_KIND_ROLLUP: u8 = 0x01;
const TS_POINT_KEY_BYTES: usize = 2 + 8 + 8 + 8;
const TS_ROLLUP_KEY_BYTES: usize = 2 + 8 + 8 + 1 + 8;
const BLOB_DISC: u8 = 0x05;
const BLOB_KIND_CHUNK: u8 = 0x00;
const BLOB_KIND_MANIFEST: u8 = 0x01;
const BLOB_ID_BYTES: usize = 16;
const BLOB_MANIFEST_KEY_BYTES: usize = 2 + 8 + BLOB_ID_BYTES;
const BLOB_CHUNK_KEY_BYTES: usize = BLOB_MANIFEST_KEY_BYTES + 4;
const NANOS_PER_MILLI: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionReclaimerConfig {
    pub max_rows_per_sweep: usize,
    pub compact_after_tombstone: bool,
}

impl Default for RetentionReclaimerConfig {
    fn default() -> Self {
        Self {
            max_rows_per_sweep: 10_000,
            compact_after_tombstone: true,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct RetentionReclaimReport {
    pub ts_points_tombstoned: usize,
    pub ts_rollups_preserved: usize,
    pub blob_manifests_tombstoned: usize,
    pub blob_chunks_tombstoned: usize,
    pub orphan_blob_chunks_tombstoned: usize,
    pub legacy_blob_manifests_skipped: usize,
    pub bytes_marked: u64,
    pub rows_tombstoned: usize,
    pub compacted_cfs: Vec<String>,
    pub capped: bool,
}

pub struct RetentionReclaimer<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    config: RetentionReclaimerConfig,
}

impl<'a, C: Clock> RetentionReclaimer<'a, C> {
    pub fn new(vault: &'a AsterVault<C>, config: RetentionReclaimerConfig) -> Result<Self> {
        if config.max_rows_per_sweep == 0 {
            return Err(invalid_collection(
                "retention reclaimer max_rows_per_sweep must be > 0",
            ));
        }
        Ok(Self { vault, config })
    }

    pub fn run_collection(&self, col: &Collection) -> Result<RetentionReclaimReport> {
        match col.mode {
            CollectionMode::TimeSeries => self.reclaim_timeseries(col),
            CollectionMode::Blob => self.reclaim_blob(col),
            _ => Err(invalid_collection(format!(
                "retention reclaimer supports TimeSeries and Blob collections, got {:?}",
                col.mode
            ))),
        }
    }

    fn reclaim_timeseries(&self, col: &Collection) -> Result<RetentionReclaimReport> {
        let mut report = RetentionReclaimReport::default();
        let Some(policy) = TimeSeriesRetention::from_policy(&col.retention, self.vault.clock_now())
        else {
            return Ok(report);
        };
        let cid = timeseries::collection_id(col).to_be_bytes();
        let mut tombstones = Vec::new();
        for (key, value) in self
            .vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::TimeSeries)?
        {
            match parse_ts_key(&key, &cid) {
                Some(TimeSeriesKey::Point { ts }) if policy.should_drop(ts) => {
                    if !push_tombstone(
                        &mut tombstones,
                        &mut report,
                        ColumnFamily::TimeSeries,
                        key,
                        value.len(),
                        self.config.max_rows_per_sweep,
                    ) {
                        break;
                    }
                    report.ts_points_tombstoned += 1;
                }
                Some(TimeSeriesKey::Rollup) => report.ts_rollups_preserved += 1,
                _ => {}
            }
        }
        self.commit_tombstones(ColumnFamily::TimeSeries, tombstones, &mut report)?;
        Ok(report)
    }

    fn reclaim_blob(&self, col: &Collection) -> Result<RetentionReclaimReport> {
        let mut report = RetentionReclaimReport::default();
        let cid = blob::collection_id(col).to_be_bytes();
        let mut manifests = HashMap::<[u8; BLOB_ID_BYTES], BlobManifest>::new();
        let mut chunks = HashMap::<[u8; BLOB_ID_BYTES], BTreeSet<u32>>::new();
        let mut chunk_value_bytes = HashMap::<([u8; BLOB_ID_BYTES], u32), usize>::new();
        for (key, value) in self
            .vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::Blob)?
        {
            match parse_blob_key(&key, &cid) {
                Some(BlobKey::Manifest { blob_id }) => {
                    manifests.insert(blob_id, decode_blob_manifest_for_reclaimer(&value)?);
                }
                Some(BlobKey::Chunk { blob_id, idx }) => {
                    chunks.entry(blob_id).or_default().insert(idx);
                    chunk_value_bytes.insert((blob_id, idx), value.len());
                }
                None => {}
            }
        }

        let mut tombstones = Vec::new();
        for (blob_id, idxs) in &chunks {
            if manifests.contains_key(blob_id) {
                continue;
            }
            for idx in idxs {
                if !push_blob_chunk_tombstone(
                    &mut tombstones,
                    &mut report,
                    col,
                    *blob_id,
                    *idx,
                    *chunk_value_bytes.get(&(*blob_id, *idx)).unwrap_or(&0),
                    self.config.max_rows_per_sweep,
                    true,
                ) {
                    self.commit_tombstones(ColumnFamily::Blob, tombstones, &mut report)?;
                    return Ok(report);
                }
            }
        }

        for (blob_id, manifest) in manifests {
            let decision =
                blob_retention_decision(&col.retention, &manifest, self.vault.clock_now());
            if decision == BlobRetentionDecision::LegacySkip {
                report.legacy_blob_manifests_skipped += 1;
            }
            if decision != BlobRetentionDecision::Drop {
                continue;
            }
            let needed = manifest.chunk_count as usize + 1;
            if report.rows_tombstoned + tombstones.len() + needed > self.config.max_rows_per_sweep {
                report.capped = true;
                break;
            }
            for idx in 0..manifest.chunk_count {
                let len = *chunk_value_bytes.get(&(blob_id, idx)).unwrap_or(&0);
                push_blob_chunk_tombstone(
                    &mut tombstones,
                    &mut report,
                    col,
                    blob_id,
                    idx,
                    len,
                    self.config.max_rows_per_sweep,
                    false,
                );
            }
            let key = blob::manifest_key(col, BlobId::from_bytes(blob_id));
            report.bytes_marked = report.bytes_marked.saturating_add(key.len() as u64);
            tombstones.push((ColumnFamily::Blob, key, tombstone_value()));
            report.blob_manifests_tombstoned += 1;
        }
        self.commit_tombstones(ColumnFamily::Blob, tombstones, &mut report)?;
        Ok(report)
    }

    fn commit_tombstones(
        &self,
        cf: ColumnFamily,
        tombstones: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        report: &mut RetentionReclaimReport,
    ) -> Result<()> {
        if tombstones.is_empty() {
            return Ok(());
        }
        report.rows_tombstoned += tombstones.len();
        self.vault.write_cf_batch(tombstones)?;
        if self.config.compact_after_tombstone {
            self.vault.purge_tombstoned_cfs(&[cf])?;
            report.compacted_cfs.push(cf.name());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum TimeSeriesRetention {
    DropBefore(u64),
    DropAllPoints,
}

impl TimeSeriesRetention {
    fn from_policy(policy: &RetentionPolicy, now_ms: u64) -> Option<Self> {
        match policy {
            RetentionPolicy::Forever => None,
            RetentionPolicy::RollupOnly => Some(Self::DropAllPoints),
            RetentionPolicy::DropAfter(duration) => {
                let span = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
                Some(Self::DropBefore(
                    now_ms.saturating_mul(NANOS_PER_MILLI).saturating_sub(span),
                ))
            }
        }
    }

    fn should_drop(self, ts: u64) -> bool {
        match self {
            Self::DropBefore(floor) => ts < floor,
            Self::DropAllPoints => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlobRetentionDecision {
    Keep,
    Drop,
    LegacySkip,
}

fn blob_retention_decision(
    policy: &RetentionPolicy,
    manifest: &BlobManifest,
    now_ms: u64,
) -> BlobRetentionDecision {
    match policy {
        RetentionPolicy::Forever => BlobRetentionDecision::Keep,
        RetentionPolicy::RollupOnly => BlobRetentionDecision::Drop,
        RetentionPolicy::DropAfter(duration) => match manifest.created_at_ms {
            Some(created_at) if expired_ms(created_at, *duration, now_ms) => {
                BlobRetentionDecision::Drop
            }
            Some(_) => BlobRetentionDecision::Keep,
            None => BlobRetentionDecision::LegacySkip,
        },
    }
}

fn expired_ms(created_at: u64, ttl: Duration, now_ms: u64) -> bool {
    u128::from(now_ms.saturating_sub(created_at)) > ttl.as_millis()
}

fn push_tombstone(
    tombstones: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    report: &mut RetentionReclaimReport,
    cf: ColumnFamily,
    key: Vec<u8>,
    value_len: usize,
    max_rows: usize,
) -> bool {
    if report.rows_tombstoned + tombstones.len() >= max_rows {
        report.capped = true;
        return false;
    }
    report.bytes_marked = report
        .bytes_marked
        .saturating_add(key.len() as u64)
        .saturating_add(value_len as u64);
    tombstones.push((cf, key, tombstone_value()));
    true
}

#[allow(clippy::too_many_arguments)]
fn push_blob_chunk_tombstone(
    tombstones: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    report: &mut RetentionReclaimReport,
    col: &Collection,
    blob_id: [u8; BLOB_ID_BYTES],
    idx: u32,
    value_len: usize,
    max_rows: usize,
    orphan: bool,
) -> bool {
    let pushed = push_tombstone(
        tombstones,
        report,
        ColumnFamily::Blob,
        blob::chunk_key(col, BlobId::from_bytes(blob_id), idx),
        value_len,
        max_rows,
    );
    if pushed {
        report.blob_chunks_tombstoned += 1;
        if orphan {
            report.orphan_blob_chunks_tombstoned += 1;
        }
    }
    pushed
}

enum TimeSeriesKey {
    Point { ts: u64 },
    Rollup,
}

fn parse_ts_key(key: &[u8], collection_id: &[u8; 8]) -> Option<TimeSeriesKey> {
    if key.first().copied()? != TS_DISC || key.get(2..10)? != collection_id {
        return None;
    }
    match key.get(1).copied()? {
        TS_KIND_POINT if key.len() == TS_POINT_KEY_BYTES => Some(TimeSeriesKey::Point {
            ts: u64::from_be_bytes(key[18..26].try_into().ok()?),
        }),
        TS_KIND_ROLLUP if key.len() == TS_ROLLUP_KEY_BYTES => Some(TimeSeriesKey::Rollup),
        _ => None,
    }
}

enum BlobKey {
    Manifest {
        blob_id: [u8; BLOB_ID_BYTES],
    },
    Chunk {
        blob_id: [u8; BLOB_ID_BYTES],
        idx: u32,
    },
}

fn parse_blob_key(key: &[u8], collection_id: &[u8; 8]) -> Option<BlobKey> {
    if key.first().copied()? != BLOB_DISC || key.get(2..10)? != collection_id {
        return None;
    }
    let mut blob_id = [0_u8; BLOB_ID_BYTES];
    blob_id.copy_from_slice(key.get(10..26)?);
    match key.get(1).copied()? {
        BLOB_KIND_MANIFEST if key.len() == BLOB_MANIFEST_KEY_BYTES => {
            Some(BlobKey::Manifest { blob_id })
        }
        BLOB_KIND_CHUNK if key.len() == BLOB_CHUNK_KEY_BYTES => Some(BlobKey::Chunk {
            blob_id,
            idx: u32::from_be_bytes(key[26..30].try_into().ok()?),
        }),
        _ => None,
    }
}

fn decode_blob_manifest_for_reclaimer(bytes: &[u8]) -> Result<BlobManifest> {
    let mut cursor = std::io::Cursor::new(bytes);
    let total_bytes = read_u64(&mut cursor)?;
    let chunk_count = read_u32(&mut cursor)?;
    let mut content_hash = [0_u8; 32];
    std::io::Read::read_exact(&mut cursor, &mut content_hash)
        .map_err(|_| CalyxError::aster_corrupt_shard("blob manifest hash truncated"))?;
    let mut cold = [0_u8; 1];
    std::io::Read::read_exact(&mut cursor, &mut cold)
        .map_err(|_| CalyxError::aster_corrupt_shard("blob manifest cold_tier truncated"))?;
    let cold_tier = match cold[0] {
        0 => false,
        1 => true,
        other => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "blob manifest cold_tier byte {other} is not 0/1"
            )));
        }
    };
    let remaining = bytes.len().saturating_sub(45);
    let created_at_ms = match remaining {
        0 => None,
        8 => Some(read_u64(&mut cursor)?),
        _ => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "blob manifest must be 45 or 53 bytes, got {}",
                bytes.len()
            )));
        }
    };
    Ok(BlobManifest {
        total_bytes,
        chunk_count,
        content_hash,
        cold_tier,
        created_at_ms,
    })
}

fn read_u64(cursor: &mut std::io::Cursor<&[u8]>) -> Result<u64> {
    let mut bytes = [0_u8; 8];
    std::io::Read::read_exact(cursor, &mut bytes)
        .map_err(|_| CalyxError::aster_corrupt_shard("blob manifest u64 truncated"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn read_u32(cursor: &mut std::io::Cursor<&[u8]>) -> Result<u32> {
    let mut bytes = [0_u8; 4];
    std::io::Read::read_exact(cursor, &mut bytes)
        .map_err(|_| CalyxError::aster_corrupt_shard("blob manifest u32 truncated"))?;
    Ok(u32::from_be_bytes(bytes))
}

fn invalid_collection(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_RETENTION_RECLAIMER_INVALID_COLLECTION,
        message: message.into(),
        remediation: "call the reclaimer with a TimeSeries or Blob collection and a nonzero bound",
    }
}

#[cfg(test)]
mod tests;
