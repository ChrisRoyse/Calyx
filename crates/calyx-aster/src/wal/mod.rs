//! Write-ahead log storage for Aster.

mod batch;
mod point_read;
mod prune;
mod record;
mod replay;
mod segment;
mod stream_replay;

use calyx_core::{CalyxError, CalyxErrorCode, Result};
use record::SpanStatus;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use batch::GroupCommitBatcher;
pub(crate) use point_read::WalWriteRowPointReader;
pub use prune::{
    CALYX_WAL_PRUNE_CONFIG_INVALID, WalPruneConfig, WalPruneOutcome, prune_covered_segments,
};
pub use replay::replay_dir;
pub use replay::replay_dir_after;
pub(crate) use stream_replay::{
    for_each_record_payload_after, for_each_record_payload_reverse, stream_records_after,
};

pub(crate) const RECORD_HEADER_BYTES: u64 = record::HEADER_LEN as u64;

/// Default group-commit window for PH05.
pub const DEFAULT_GROUP_COMMIT_WINDOW: Duration = Duration::from_millis(2);
/// Maximum encoded payload carried by one physical WAL record. Logical payloads
/// larger than this are fragmented across a chain of physical records.
pub(crate) const MAX_RECORD_BYTES: usize = record::MAX_RECORD_BYTES as usize;

/// Stable error code for a single commit batch whose encoded payload exceeds the
/// per-commit WAL ceiling ([`record::MAX_LOGICAL_PAYLOAD_BYTES`]). This is a
/// capacity limit on one commit, distinct from `CALYX_DISK_PRESSURE` (which
/// signals a full/failing volume): the remediation is to split the batch, not to
/// free disk.
pub(crate) const CALYX_WAL_PAYLOAD_TOO_LARGE: &str = "CALYX_WAL_PAYLOAD_TOO_LARGE";

fn wal_payload_too_large(len: usize) -> CalyxError {
    CalyxError {
        code: CALYX_WAL_PAYLOAD_TOO_LARGE,
        message: format!(
            "WAL logical payload {len} bytes exceeds the single-commit ceiling {} bytes",
            record::MAX_LOGICAL_PAYLOAD_BYTES
        ),
        remediation: "split the commit batch into smaller commits; this is a single-commit capacity limit, not disk pressure",
    }
}

/// WAL writer configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalOptions {
    /// Maximum bytes in one segment before the next append rotates.
    pub max_segment_bytes: u64,
    /// Upper bound for coalescing near-following requests into one fsync.
    pub group_commit_window: Duration,
}

impl Default for WalOptions {
    fn default() -> Self {
        Self {
            max_segment_bytes: 64 * 1024 * 1024,
            group_commit_window: DEFAULT_GROUP_COMMIT_WINDOW,
        }
    }
}

/// Fsync-backed append acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAck {
    pub seq: u64,
    pub segment_path: PathBuf,
    pub start_offset: u64,
    pub end_offset: u64,
}

/// A replayed WAL record. `payload` is the reassembled logical payload; a record
/// split across a `CXW2` fragment chain is presented as one logical record with
/// `fragmented = true` and `start_offset`/`end_offset` spanning the whole chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
    pub segment_path: PathBuf,
    pub start_offset: u64,
    pub end_offset: u64,
    /// True when the logical payload was reassembled from a fragment chain.
    /// Physical-offset consumers (e.g. the Base page index) must not compute
    /// intra-record byte offsets for a fragmented record.
    pub fragmented: bool,
}

/// Torn WAL tail discarded during replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TornTail {
    pub segment_path: PathBuf,
    pub offset: u64,
    pub code: &'static str,
    pub message: String,
}

impl TornTail {
    /// Converts the recovery observation to the catalogued Calyx error.
    pub fn error(&self) -> CalyxError {
        CalyxErrorCode::AsterTornWal.error(format!(
            "{} at byte {}: {}",
            self.segment_path.display(),
            self.offset,
            self.message
        ))
    }
}

/// WAL replay result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub records: Vec<ReplayRecord>,
    pub torn_tail: Option<TornTail>,
}

/// Physical WAL segment readback used by recyclers and FSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalSegmentStatus {
    pub index: u64,
    pub path: PathBuf,
    pub bytes: u64,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub record_count: usize,
    pub active: bool,
}

/// Result of one bounded WAL recycle pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalRecycleReport {
    pub newest_durable_seq: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub segments_before: usize,
    pub recyclable_segments_before: usize,
    pub segments_recycled: usize,
    pub bytes_recycled: u64,
    pub recycled_paths: Vec<PathBuf>,
}

/// Durable WAL writer.
#[derive(Debug)]
pub struct Wal {
    dir: PathBuf,
    options: WalOptions,
    active_index: u64,
    file: File,
    active_len: u64,
    next_seq: u64,
}

impl Wal {
    /// Opens a WAL directory, replaying and truncating any torn tail first.
    pub fn open(dir: impl AsRef<Path>, options: WalOptions) -> Result<Self> {
        Self::open_after(dir, options, 0)
    }

    /// Opens a WAL directory while skipping payload decode for records already
    /// covered by a durable checkpoint sequence.
    pub(crate) fn open_after(
        dir: impl AsRef<Path>,
        options: WalOptions,
        replay_floor_seq: u64,
    ) -> Result<Self> {
        batch::validate_window(options.group_commit_window)?;
        let dir = dir.as_ref().to_path_buf();
        crate::private_fs::create_private_dir_all(&dir)?;
        let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
        // The open handle only needs the tip seq to resume appending — NOT the
        // decoded post-floor tail. A full `replay_dir_locked_after` here decoded
        // and retained every post-floor record's payload in a Vec just to read
        // `.last().seq`, materializing a multi-GB orphaned WAL tail into the
        // heap on every open and OOM-killing `wal-prune`/reader opens against a
        // 27 GB WAL (issue #1992; the dominant accumulator per on-host dwarf
        // stacks). Recover the tip header-only instead; `recover_tip_seq_locked`
        // also heals a torn physical tail on the newest segment exactly as
        // replay's `resolve_torn_tail` does, so open semantics are preserved.
        let tip_seq = recover_tip_seq_locked(&dir)?;
        let last_replayed_seq = tip_seq.max(replay_floor_seq);
        let next_seq = last_replayed_seq.saturating_add(1).max(1);
        let segments = segment::list_segments(&dir)?;
        let active_index = segments.last().map_or(0, |(index, _)| *index);
        let active_path = segment::segment_path(&dir, active_index);
        let mut file = open_append_file(&active_path)?;
        let active_len = file
            .seek(SeekFrom::End(0))
            .map_err(|error| storage_error("seek WAL segment", error))?;

        Ok(Self {
            dir,
            options,
            active_index,
            file,
            active_len,
            next_seq,
        })
    }

    /// Appends one record and fsyncs it.
    pub fn append(&mut self, payload: &[u8]) -> Result<AppendAck> {
        let mut acks = self.append_batch(&[payload])?;
        Ok(acks.remove(0))
    }

    /// Appends a batch and fsyncs once before acknowledging the records.
    pub fn append_batch(&mut self, payloads: &[&[u8]]) -> Result<Vec<AppendAck>> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }

        for payload in payloads {
            if payload.len() as u64 > record::MAX_LOGICAL_PAYLOAD_BYTES {
                return Err(wal_payload_too_large(payload.len()));
            }
        }

        let _lock = crate::file_lock::FileLockGuard::acquire(&self.dir.join(".append.lock"))?;
        self.refresh_after_external_appends_locked()?;
        let mut acks = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let seq = self.next_seq;
            let (start_offset, end_offset) = if payload.len() <= MAX_RECORD_BYTES {
                self.append_full_record(seq, payload)?
            } else {
                self.append_fragmented_record(seq, payload)?
            };
            acks.push(AppendAck {
                seq,
                segment_path: self.active_path(),
                start_offset,
                end_offset,
            });
            self.next_seq += 1;
            self.active_len = end_offset;
        }

        self.file
            .sync_data()
            .map_err(|error| storage_error("fsync WAL batch", error))?;
        Ok(acks)
    }

    /// Writes one `CXW1` full record (payload fits in a single physical record).
    fn append_full_record(&mut self, seq: u64, payload: &[u8]) -> Result<(u64, u64)> {
        let bytes = record::encode(seq, payload)
            .map_err(|error| storage_error("encode WAL record", error))?;
        self.rotate_if_needed(bytes.len() as u64)?;
        let start_offset = self.seek_end()?;
        self.file
            .write_all(&bytes)
            .map_err(|error| storage_error("append WAL record", error))?;
        Ok((start_offset, start_offset + bytes.len() as u64))
    }

    /// Writes one large logical record as a contiguous `CXW2` fragment chain.
    ///
    /// The whole chain is rotated as a unit so every fragment of one logical
    /// record lands in a single segment file (fragments must never straddle a
    /// segment boundary). A chain is allowed to push the segment past the soft
    /// `max_segment_bytes` so it stays whole. All fragments share the logical
    /// `seq`; `next_seq` still advances by one per logical record.
    fn append_fragmented_record(&mut self, seq: u64, payload: &[u8]) -> Result<(u64, u64)> {
        let frag_count = record::fragment_count(payload.len());
        let frag_count_u32 = u32::try_from(frag_count).map_err(|_| {
            CalyxError::aster_corrupt_shard(format!(
                "WAL logical payload requires {frag_count} fragments, exceeding u32"
            ))
        })?;
        let logical_len = payload.len() as u64;
        let total_bytes = (frag_count as u64) * record::FRAGMENT_HEADER_LEN as u64 + logical_len;
        // Rotate the entire chain as one unit so it cannot straddle segments.
        self.rotate_if_needed(total_bytes)?;
        let start_offset = self.seek_end()?;
        for (frag_index, chunk) in payload.chunks(MAX_RECORD_BYTES).enumerate() {
            let header =
                record::fragment_header(seq, frag_index as u32, frag_count_u32, logical_len, chunk);
            self.file
                .write_all(&header)
                .map_err(|error| storage_error("append WAL fragment header", error))?;
            self.file
                .write_all(chunk)
                .map_err(|error| storage_error("append WAL fragment payload", error))?;
        }
        Ok((start_offset, start_offset + total_bytes))
    }

    pub fn durable_tip_seq(&mut self) -> Result<u64> {
        let _lock = crate::file_lock::FileLockGuard::acquire(&self.dir.join(".append.lock"))?;
        self.refresh_after_external_appends_locked()?;
        Ok(self.next_seq.saturating_sub(1))
    }

    /// Reads the physical WAL segment inventory behind the append lock.
    pub fn segment_inventory(&mut self) -> Result<Vec<WalSegmentStatus>> {
        let _lock = crate::file_lock::FileLockGuard::acquire(&self.dir.join(".append.lock"))?;
        self.refresh_after_external_appends_locked()?;
        segment_inventory_locked(&self.dir, self.active_index)
    }

    /// Returns total bytes in all canonical WAL segment files.
    pub fn total_segment_bytes(&mut self) -> Result<u64> {
        Ok(self
            .segment_inventory()?
            .iter()
            .map(|segment| segment.bytes)
            .sum())
    }

    /// Resets durable, non-active segments without deallocating segment files.
    pub fn recycle_durable_segments(
        &mut self,
        newest_durable_seq: u64,
        max_segments: usize,
        fsync_budget: usize,
    ) -> Result<WalRecycleReport> {
        let _lock = crate::file_lock::FileLockGuard::acquire(&self.dir.join(".append.lock"))?;
        self.refresh_after_external_appends_locked()?;
        let before = segment_inventory_locked(&self.dir, self.active_index)?;
        let bytes_before = total_bytes(&before);
        let budget = max_segments.min(fsync_budget);
        let candidates = recyclable_segments(&before, newest_durable_seq);
        let mut report = WalRecycleReport {
            newest_durable_seq,
            bytes_before,
            bytes_after: bytes_before,
            segments_before: before.len(),
            recyclable_segments_before: candidates.len(),
            ..WalRecycleReport::default()
        };
        if budget == 0 {
            return Ok(report);
        }

        for segment in candidates.into_iter().take(budget) {
            let file = OpenOptions::new()
                .write(true)
                .open(&segment.path)
                .map_err(|error| storage_error("open WAL segment for recycle", error))?;
            file.set_len(0)
                .map_err(|error| storage_error("truncate recycled WAL segment", error))?;
            file.sync_data()
                .map_err(|error| storage_error("fsync recycled WAL segment", error))?;
            report.segments_recycled += 1;
            report.bytes_recycled = report.bytes_recycled.saturating_add(segment.bytes);
            report.recycled_paths.push(segment.path.clone());
        }

        let after = segment_inventory_locked(&self.dir, self.active_index)?;
        report.bytes_after = total_bytes(&after);
        Ok(report)
    }

    fn refresh_after_external_appends_locked(&mut self) -> Result<()> {
        if !self.external_appends_present()? {
            return Ok(());
        }
        // Recover next_seq from the highest committed seq WITHOUT materializing
        // any payload. A full floor-0 replay here decoded every record of the
        // entire WAL into the heap just to read the last seq — O(total WAL
        // bytes) — which OOM-killed `wal-prune` attaching a fresh write handle
        // to a 27 GB / 581-segment WAL (issue #1992). Seqs increase
        // monotonically with segment index, so the highest seq is the last
        // record of the highest-index non-empty segment; a header-seek scan
        // over that segment reads no payload bytes.
        let highest_seq = recover_tip_seq_locked(&self.dir)?;
        self.next_seq = highest_seq.saturating_add(1).max(1);
        let segments = segment::list_segments(&self.dir)?;
        let active_index = segments.last().map_or(0, |(index, _)| *index);
        if active_index != self.active_index {
            self.active_index = active_index;
            self.file = open_append_file(&self.active_path())?;
        }
        self.active_len = self.seek_end()?;
        Ok(())
    }

    fn external_appends_present(&self) -> Result<bool> {
        let segments = segment::list_segments(&self.dir)?;
        let Some((active_index, active_path)) = segments.last() else {
            return Ok(self.active_index != 0 || self.active_len != 0);
        };
        if *active_index != self.active_index {
            return Ok(true);
        }
        let len = fs::metadata(active_path)
            .map_err(|error| storage_error("stat WAL segment", error))?
            .len();
        Ok(len != self.active_len)
    }

    fn rotate_if_needed(&mut self, incoming_bytes: u64) -> Result<()> {
        let offset = self.seek_end()?;
        if offset == 0 || offset + incoming_bytes <= self.options.max_segment_bytes {
            return Ok(());
        }

        self.file
            .sync_all()
            .map_err(|error| storage_error("fsync WAL segment before rotation", error))?;
        self.active_index += 1;
        self.file = open_append_file(&self.active_path())?;
        self.active_len = 0;
        Ok(())
    }

    fn seek_end(&mut self) -> Result<u64> {
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|error| storage_error("seek WAL segment", error))
    }

    fn active_path(&self) -> PathBuf {
        segment::segment_path(&self.dir, self.active_index)
    }
}

fn open_append_file(path: &Path) -> Result<File> {
    let existed = path.exists();
    let mut options = OpenOptions::new();
    options.create(true).read(true).append(true);
    crate::private_fs::harden_open_options(&mut options);
    let file = options
        .open(path)
        .map_err(|error| storage_error("open WAL segment", error))?;
    if !existed {
        crate::private_fs::enforce_private_file_mode(&file, path)?;
        sync_parent(path)?;
    }
    Ok(file)
}

fn sync_parent(path: &Path) -> Result<()> {
    crate::fsync::sync_parent(path, "WAL segment")
}

fn segment_inventory_locked(dir: &Path, active_index: u64) -> Result<Vec<WalSegmentStatus>> {
    let mut inventory = Vec::new();
    for (index, path) in segment::list_segments(dir)? {
        let bytes = fs::metadata(&path)
            .map_err(|error| storage_error("stat WAL segment", error))?
            .len();
        let (first_seq, last_seq, record_count) = read_segment_seq_bounds(&path)?;
        inventory.push(WalSegmentStatus {
            index,
            path,
            bytes,
            first_seq,
            last_seq,
            record_count,
            active: index == active_index,
        });
    }
    Ok(inventory)
}

fn read_segment_seq_bounds(path: &Path) -> Result<(Option<u64>, Option<u64>, usize)> {
    let mut file =
        File::open(path).map_err(|error| storage_error("open WAL segment for inventory", error))?;
    let mut offset = 0;
    let mut first = None;
    let mut last = None;
    let mut count = 0;
    loop {
        match record::read_logical_span_at(&mut file, offset)
            .map_err(|error| storage_error("decode WAL inventory", error))?
        {
            SpanStatus::Complete(span) => {
                first.get_or_insert(span.seq);
                last = Some(span.seq);
                count += 1;
                offset = span.end_offset;
            }
            SpanStatus::Eof => return Ok((first, last, count)),
            SpanStatus::Torn { offset, message } => {
                return Err(CalyxError::aster_torn_wal(format!(
                    "{} at byte {offset}: {message}",
                    path.display()
                )));
            }
        }
    }
}

/// Highest committed seq across the WAL, recovered with header-seek scans only
/// (no payload materialization). Seqs increase monotonically with segment
/// index, so it scans from the newest segment backwards and returns the last
/// span's seq of the first non-empty segment. A torn physical tail on the
/// newest segment is truncated in place (mirroring replay's `resolve_torn_tail`
/// self-heal); a torn record in any earlier segment fails closed. Returns `0`
/// for an empty WAL.
fn recover_tip_seq_locked(dir: &Path) -> Result<u64> {
    Ok(recover_tip_locked(dir)?.0)
}

/// Highest committed seq plus any healed torn-tail observation, recovered with
/// header-seek scans only. See [`recover_tip_seq_locked`]; this variant also
/// surfaces the truncated torn tail so streaming recovery can report it in the
/// recovery outcome without decoding the tail.
fn recover_tip_locked(dir: &Path) -> Result<(u64, Option<TornTail>)> {
    let segments = segment::list_segments(dir)?;
    for (position, (_, path)) in segments.iter().enumerate().rev() {
        let has_later_segments = position + 1 < segments.len();
        let (last_seq, torn) = scan_segment_last_seq(path, has_later_segments)?;
        if let Some(seq) = last_seq {
            return Ok((seq, torn));
        }
        if torn.is_some() {
            // Newest segment was entirely a torn tail (now truncated to empty);
            // continue to the previous segment for the tip.
            continue;
        }
    }
    Ok((0, None))
}

/// Acquires the append lock and recovers the WAL tip seq header-only, healing a
/// torn newest tail. Returns the tip and any torn-tail observation for the
/// streaming recovery path (issue #1992), which restores the post-floor tail in
/// bounded chunks rather than decoding it all into the heap.
pub(crate) fn recover_tip_and_torn(dir: impl AsRef<Path>) -> Result<(u64, Option<TornTail>)> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    recover_tip_locked(dir)
}

/// Header-seek scan of one segment returning its last record's seq (or `None`
/// when the segment holds no complete record) and any torn tail it healed.
/// Reads only record headers — never a payload — so cost is independent of
/// logical payload size. Heals a torn physical tail only when this is the
/// newest segment (`!has_later_segments`), exactly as WAL replay does; a torn
/// record with later segments present is a missing-write hazard and fails
/// closed.
fn scan_segment_last_seq(
    path: &Path,
    has_later_segments: bool,
) -> Result<(Option<u64>, Option<TornTail>)> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| storage_error("open WAL segment for tip scan", error))?;
    let mut offset = 0;
    let mut last_seq = None;
    loop {
        match record::read_logical_span_at(&mut file, offset)
            .map_err(|error| storage_error("scan WAL span", error))?
        {
            SpanStatus::Complete(span) => {
                last_seq = Some(span.seq);
                offset = span.end_offset;
            }
            SpanStatus::Eof => return Ok((last_seq, None)),
            SpanStatus::Torn {
                offset: torn_offset,
                message,
            } => {
                if has_later_segments {
                    return Err(CalyxError::aster_torn_wal(format!(
                        "{} at byte {torn_offset}: {message}",
                        path.display()
                    )));
                }
                let torn = TornTail {
                    segment_path: path.to_path_buf(),
                    offset: torn_offset,
                    code: CalyxErrorCode::AsterTornWal.code(),
                    message,
                };
                file.set_len(torn_offset)
                    .map_err(|error| storage_error("truncate torn WAL tail", error))?;
                file.sync_data()
                    .map_err(|error| storage_error("fsync truncated WAL tail", error))?;
                return Ok((last_seq, Some(torn)));
            }
        }
    }
}

fn recyclable_segments(
    inventory: &[WalSegmentStatus],
    newest_durable_seq: u64,
) -> Vec<&WalSegmentStatus> {
    inventory
        .iter()
        .filter(|segment| {
            !segment.active
                && segment.bytes > 0
                && segment
                    .last_seq
                    .is_some_and(|last_seq| last_seq <= newest_durable_seq)
        })
        .collect()
}

fn total_bytes(inventory: &[WalSegmentStatus]) -> u64 {
    inventory.iter().map(|segment| segment.bytes).sum()
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests;
