//! Out-of-core per-slot vector scratch store for weave-loom (#1991).
//!
//! The coverage scan used to decode every selected constellation's per-slot
//! vector into a heap `BTreeMap<SlotId, HashMap<CxId, SlotVector>>` and keep the
//! whole decoded corpus resident through within-doc and between-doc. On the
//! canonical 109,133-row / 15-lane law vault that single resident copy measured
//! 38.9 GB (2 chunk-Multi lanes dominate) and the process was OOM-killed at
//! 56.2 GB anon RSS — a host with 121 GB total and ~65 GB permanently
//! co-resident can never hold it (#1991 round-2 evidence).
//!
//! This store moves the decoded corpus **out of core**. The scan streams each
//! measured slot vector's *canonical encoded bytes* — the exact
//! [`calyx_aster::vault::encode`] on-disk encoding, so the residence changes but
//! the bytes do not (#1960) — into a per-slot record file keyed by dense scan
//! ordinal, with a companion offsets sidecar. within-doc and between-doc then
//! read one constellation's typed panel back on demand (per batch / per candidate
//! edge) by `mmap`ing the record file: the vector payload lives in reclaimable
//! page cache the kernel can evict under pressure instead of anonymous heap it
//! OOM-kills. Anon RSS becomes `O(batch + per-node candidate set)`, independent
//! of corpus size.
//!
//! Layout under `<vault>/idx/weave_loom/runs/<run>/scratch_vectors/` (created
//! `0700`; each file `0600` via the #1989 `private_fs` primitives):
//!
//! * `slot-<id>.rec` — variable-length records concatenated in ordinal order.
//!   Record `o` is the canonical [`encode_slot_vector`](calyx_aster::vault::encode::encode_slot_vector)
//!   byte string for the constellation at scan ordinal `o`; a **zero-length**
//!   record marks a constellation with no measured vector in that slot
//!   (absent / tombstoned / base-row-missing), mirroring the old map's
//!   "not present" exactly.
//! * `slot-<id>.off` — a fixed [`OFF_HEADER_SIZE`]-byte header
//!   (`CLXWSCR1` + version + slot id + node_count) followed by `node_count + 1`
//!   little-endian `u64` byte offsets into `.rec`; record `o` occupies
//!   `off[o]..off[o+1]`. This is the offsets sidecar the fixed-record #1990
//!   `PackedRawSidecar` cannot provide for the variable-length Multi /
//!   SegmentedMulti / Sparse lanes, so a single uniform variable-length record
//!   format serves every `SlotVector` shape (documented design choice, #1991).
//!
//! A short, corrupt, dimension-mismatched, or symlinked file fails closed with a
//! typed `CALYX_*` error on open; readers never zero-fill. The store is cleaned
//! up on weave success and left on disk for diagnosis on failure.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use calyx_aster::private_fs;
use calyx_aster::vault::encode::decode_slot_vector;
use calyx_core::{CxId, SlotId, SlotVector};
use calyx_lodestar::TypedPanelVectors;
use memmap2::Mmap;

use crate::error::{CliError, CliResult};

/// Directory (under the weave run dir) holding the per-slot scratch files.
const SCRATCH_DIR_NAME: &str = "scratch_vectors";
/// Magic at offset 0 of a `slot-<id>.off` sidecar.
const OFF_MAGIC: [u8; 8] = *b"CLXWSCR1";
/// On-disk layout version for the offsets sidecar.
const OFF_VERSION: u32 = 1;
/// Fixed header size of the offsets sidecar. The 24-byte prefix (magic +
/// version + slot id + node_count) is zero-padded so the first `u64` offset is
/// 8-byte aligned within the file.
const OFF_HEADER_SIZE: usize = 64;

fn corrupt(detail: impl std::fmt::Display) -> CliError {
    CliError::io(format!("weave scratch store corrupt: {detail}"))
}

fn slot_rec_name(slot: SlotId) -> String {
    format!("slot-{}.rec", slot.get())
}

fn slot_off_name(slot: SlotId) -> String {
    format!("slot-{}.off", slot.get())
}

/// Streaming writer: one open record file per slot, appended in strict ordinal
/// order. All slots must be pushed the same total number of ordinals so the
/// store is dense `0..node_count` for every lane.
pub(super) struct WeaveVectorScratchWriter {
    dir: PathBuf,
    slots: BTreeMap<SlotId, SlotRecordWriter>,
}

struct SlotRecordWriter {
    rec_path: PathBuf,
    off_path: PathBuf,
    writer: BufWriter<File>,
    /// Byte offsets into `.rec`; `offsets[0] == 0`, one entry per written record
    /// boundary. Length is `next_ordinal + 1`.
    offsets: Vec<u64>,
    cursor: u64,
}

impl WeaveVectorScratchWriter {
    /// Create the scratch directory under `run_dir` and open one record file per
    /// slot. Fails closed if any slot id is duplicated.
    pub(super) fn create(run_dir: &Path, slots: &[SlotId]) -> CliResult<Self> {
        let dir = run_dir.join(SCRATCH_DIR_NAME);
        private_fs::create_private_dir_all(&dir)?;
        let mut writers = BTreeMap::new();
        for &slot in slots {
            let rec_path = dir.join(slot_rec_name(slot));
            let off_path = dir.join(slot_off_name(slot));
            let file = private_fs::open_private_temp_file(&rec_path)?;
            if writers
                .insert(
                    slot,
                    SlotRecordWriter {
                        rec_path,
                        off_path,
                        writer: BufWriter::new(file),
                        offsets: vec![0],
                        cursor: 0,
                    },
                )
                .is_some()
            {
                return Err(CliError::io(format!(
                    "weave scratch writer given duplicate slot {slot}"
                )));
            }
        }
        Ok(Self {
            dir,
            slots: writers,
        })
    }

    /// Append the record for `(slot, ordinal)`. `encoded` is `Some` canonical
    /// slot-vector bytes for a measured vector, or `None` for a constellation
    /// with no measured vector in this slot (written as a zero-length record).
    /// Fails closed if `ordinal` is not the next expected ordinal for the slot,
    /// so a misordered scan can never silently misplace a record.
    pub(super) fn push(&mut self, slot: SlotId, ordinal: u64, encoded: Option<&[u8]>) -> CliResult {
        let writer = self.slots.get_mut(&slot).ok_or_else(|| {
            CliError::io(format!(
                "weave scratch writer has no record file for slot {slot}"
            ))
        })?;
        let expected = writer.offsets.len() as u64 - 1;
        if ordinal != expected {
            return Err(CliError::io(format!(
                "weave scratch writer slot {slot} expected ordinal {expected}, got {ordinal}"
            )));
        }
        if let Some(bytes) = encoded {
            writer
                .writer
                .write_all(bytes)
                .map_err(|error| CliError::io(format!("write weave scratch record: {error}")))?;
            writer.cursor += bytes.len() as u64;
        }
        writer.offsets.push(writer.cursor);
        Ok(())
    }

    /// Flush every record file, publish the offsets sidecars, and return a
    /// mmap-backed reader. Every slot must have been pushed exactly
    /// `node_count` ordinals.
    pub(super) fn finish(self, node_count: u64) -> CliResult<WeaveVectorScratch> {
        let dir = self.dir;
        let mut slots = BTreeMap::new();
        for (slot, writer) in self.slots {
            let written = writer.offsets.len() as u64 - 1;
            if written != node_count {
                return Err(CliError::io(format!(
                    "weave scratch slot {slot} has {written} records, expected {node_count}"
                )));
            }
            let file = writer.writer.into_inner().map_err(|error| {
                CliError::io(format!("flush weave scratch record file: {error}"))
            })?;
            file.sync_all().map_err(|error| {
                CliError::io(format!("fsync weave scratch record file: {error}"))
            })?;
            drop(file);

            let mut off_bytes = Vec::with_capacity(OFF_HEADER_SIZE + writer.offsets.len() * 8);
            off_bytes.extend_from_slice(&OFF_MAGIC);
            off_bytes.extend_from_slice(&OFF_VERSION.to_le_bytes());
            off_bytes.extend_from_slice(&u32::from(slot.get()).to_le_bytes());
            off_bytes.extend_from_slice(&node_count.to_le_bytes());
            off_bytes.resize(OFF_HEADER_SIZE, 0);
            for offset in &writer.offsets {
                off_bytes.extend_from_slice(&offset.to_le_bytes());
            }
            private_fs::write_private_atomic(&writer.off_path, &off_bytes)?;

            let packed = PackedSlot::open(slot, &writer.rec_path, &writer.off_path)?;
            slots.insert(slot, packed);
        }
        Ok(WeaveVectorScratch {
            dir,
            slots,
            #[cfg(test)]
            node_count,
        })
    }
}

/// mmap-backed reader over the finished per-slot record files.
#[derive(Debug)]
pub(super) struct WeaveVectorScratch {
    dir: PathBuf,
    slots: BTreeMap<SlotId, PackedSlot>,
    #[cfg(test)]
    node_count: u64,
}

#[derive(Debug)]
struct PackedSlot {
    rec: Mmap,
    /// `node_count + 1` byte offsets into `rec`.
    offsets: Vec<u64>,
}

impl PackedSlot {
    fn open(slot: SlotId, rec_path: &Path, off_path: &Path) -> CliResult<Self> {
        let off_bytes = fs::read(off_path)
            .map_err(|error| corrupt(format!("read {}: {error}", off_path.display())))?;
        if off_bytes.len() < OFF_HEADER_SIZE {
            return Err(corrupt(format!(
                "{} is {} B, shorter than one {OFF_HEADER_SIZE} B header",
                off_path.display(),
                off_bytes.len()
            )));
        }
        if off_bytes[0..8] != OFF_MAGIC {
            return Err(corrupt(format!("{} bad magic", off_path.display())));
        }
        let version = u32::from_le_bytes(off_bytes[8..12].try_into().expect("4B"));
        if version != OFF_VERSION {
            return Err(corrupt(format!(
                "{} version {version} != {OFF_VERSION}",
                off_path.display()
            )));
        }
        let stored_slot = u32::from_le_bytes(off_bytes[12..16].try_into().expect("4B"));
        if stored_slot != u32::from(slot.get()) {
            return Err(corrupt(format!(
                "{} slot {stored_slot} != {}",
                off_path.display(),
                slot.get()
            )));
        }
        let node_count = u64::from_le_bytes(off_bytes[16..24].try_into().expect("8B"));
        let table_len = off_bytes.len() - OFF_HEADER_SIZE;
        if !table_len.is_multiple_of(8) {
            return Err(corrupt(format!(
                "{} offset table {table_len} B is not a multiple of 8",
                off_path.display()
            )));
        }
        let entries = (table_len / 8) as u64;
        if entries != node_count + 1 {
            return Err(corrupt(format!(
                "{} has {entries} offset entries, expected node_count+1 = {}",
                off_path.display(),
                node_count + 1
            )));
        }
        let mut offsets = Vec::with_capacity(entries as usize);
        for chunk in off_bytes[OFF_HEADER_SIZE..].chunks_exact(8) {
            offsets.push(u64::from_le_bytes(chunk.try_into().expect("8B")));
        }
        if offsets.first() != Some(&0) {
            return Err(corrupt(format!("{} first offset != 0", off_path.display())));
        }
        if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(corrupt(format!(
                "{} offsets are not monotonic",
                off_path.display()
            )));
        }

        let file = File::open(rec_path)
            .map_err(|error| corrupt(format!("open {}: {error}", rec_path.display())))?;
        let len = file
            .metadata()
            .map_err(|error| corrupt(format!("stat {}: {error}", rec_path.display())))?
            .len();
        let expected = *offsets.last().expect("offsets has at least one entry");
        if len != expected {
            return Err(corrupt(format!(
                "{} is {len} B, expected {expected} B from the offsets sidecar",
                rec_path.display()
            )));
        }
        // SAFETY: a read-only map of a scratch file this process created and no
        // longer writes; the file lives under the run's private 0700 scratch dir
        // and is not shared, so no external truncation can occur mid-read.
        let rec = unsafe {
            Mmap::map(&file)
                .map_err(|error| corrupt(format!("mmap {}: {error}", rec_path.display())))?
        };
        Ok(Self { rec, offsets })
    }

    fn record(&self, ordinal: u64) -> CliResult<&[u8]> {
        let index = usize::try_from(ordinal)
            .ok()
            .filter(|&i| i + 1 < self.offsets.len())
            .ok_or_else(|| corrupt(format!("ordinal {ordinal} out of range")))?;
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        Ok(&self.rec[start..end])
    }
}

impl WeaveVectorScratch {
    /// Number of dense scan ordinals the store was built over.
    #[cfg(test)]
    pub(super) fn node_count(&self) -> u64 {
        self.node_count
    }

    /// Whether the constellation at `ordinal` has a measured (non-empty) record
    /// in `slot` — the out-of-core equivalent of the old
    /// `slot_maps[slot].contains_key(cx_id)` membership test. The scan drives its
    /// complete-panel intersection off the cheaper `present_slot_counts`, so this
    /// is exercised by the store's own roundtrip tests.
    #[cfg(test)]
    pub(super) fn is_present(&self, slot: SlotId, ordinal: u64) -> CliResult<bool> {
        let packed = self
            .slots
            .get(&slot)
            .ok_or_else(|| CliError::io(format!("weave scratch store has no slot {slot}")))?;
        Ok(!packed.record(ordinal)?.is_empty())
    }

    /// Decode `slot`'s vector for the constellation at `ordinal`, or `None` if
    /// the record is empty (no measured vector). Byte-for-byte the inverse of
    /// the canonical bytes streamed in by the scan.
    pub(super) fn read_vector(&self, slot: SlotId, ordinal: u64) -> CliResult<Option<SlotVector>> {
        let packed = self
            .slots
            .get(&slot)
            .ok_or_else(|| CliError::io(format!("weave scratch store has no slot {slot}")))?;
        let bytes = packed.record(ordinal)?;
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(decode_slot_vector(bytes)?))
    }

    /// Canonical encoded bytes occupied by one slot record. Candidate batching
    /// uses this before decoding to enforce a byte bound as well as a row bound,
    /// so large SegmentedMulti queries cannot create an unbounded heap batch.
    pub(super) fn vector_record_bytes(&self, slot: SlotId, ordinal: u64) -> CliResult<u64> {
        let packed = self
            .slots
            .get(&slot)
            .ok_or_else(|| CliError::io(format!("weave scratch store has no slot {slot}")))?;
        Ok(packed.record(ordinal)?.len() as u64)
    }

    /// Canonical encoded bytes occupied by a complete typed panel.
    pub(super) fn panel_record_bytes(&self, ordinal: u64, slots: &[SlotId]) -> CliResult<u64> {
        slots.iter().try_fold(0_u64, |total, &slot| {
            total
                .checked_add(self.vector_record_bytes(slot, ordinal)?)
                .ok_or_else(|| CliError::io("weave scratch panel byte count overflow"))
        })
    }

    /// Reconstruct the complete typed panel for the constellation at `ordinal`
    /// over `slots`. Fails closed if any contracted slot has no measured vector
    /// — the panel graph refuses partial constellations, exactly as the
    /// in-heap drain path did.
    pub(super) fn read_panel(
        &self,
        ordinal: u64,
        slots: &[SlotId],
    ) -> CliResult<TypedPanelVectors> {
        let mut panel = TypedPanelVectors::new();
        for &slot in slots {
            let vector = self.read_vector(slot, ordinal)?.ok_or_else(|| {
                CliError::io(format!(
                    "weave scratch ordinal {ordinal} has no measured vector in contracted slot {slot}"
                ))
            })?;
            panel.insert(slot, vector);
        }
        Ok(panel)
    }

    /// Total record bytes across every slot — the on-disk (page-cache) footprint,
    /// surfaced in the CALYX_WEAVE_RUNTIME `scratch_bytes` detail.
    pub(super) fn total_record_bytes(&self) -> u64 {
        self.slots
            .values()
            .map(|packed| packed.offsets.last().copied().unwrap_or(0))
            .sum()
    }

    /// Conservative upper bound on the resident bytes one node's typed panel
    /// (all slots) plus a candidate reconstruction costs — the largest single
    /// record summed across slots. Used only to size the fail-closed budget
    /// preflight; the live-RSS guard is the authoritative bound.
    pub(super) fn max_panel_record_bytes(&self) -> u64 {
        self.slots
            .values()
            .map(|packed| {
                packed
                    .offsets
                    .windows(2)
                    .map(|pair| pair[1] - pair[0])
                    .max()
                    .unwrap_or(0)
            })
            .sum()
    }

    /// Remove the scratch tree. Called on weave success; on failure the store is
    /// left on disk for diagnosis (never auto-removed on the error path). Open
    /// mmaps keep working after the unlink until the reader is dropped.
    pub(super) fn cleanup(&self) -> CliResult {
        fs::remove_dir_all(&self.dir)
            .map_err(|error| CliError::io(format!("remove weave scratch dir: {error}")))
    }
}

/// The ordinal↔cx_id map for the selected complete-panel candidate set: the
/// dense scan ordinal each surviving constellation was streamed into the scratch
/// store at. Small (109k × 16 B ≈ 1.7 MB), so it stays resident.
#[derive(Default)]
pub(super) struct OrdinalMap {
    to_ordinal: std::collections::HashMap<CxId, u64>,
}

impl OrdinalMap {
    pub(super) fn insert(&mut self, cx_id: CxId, ordinal: u64) {
        self.to_ordinal.insert(cx_id, ordinal);
    }

    pub(super) fn ordinal(&self, cx_id: &CxId) -> Option<u64> {
        self.to_ordinal.get(cx_id).copied()
    }
}

#[cfg(test)]
mod tests;
