//! DiskANN on-disk graph format: header, compact node blocks, writer,
//! mmap reader (PH68 T01). Construction lives in [`super::build`].
//!
//! Layout: one page-aligned header block (`CLXDA001`), then one fixed-size
//! block per node holding `[vector payload | neighbor_count: u32 | neighbors:
//! [u32; m_max] zero-padded]` so a single offset calculation fetches a node's
//! full search state. v1/v2 payloads are f32; v3 payloads are signed int8
//! directional vectors. Node `id` lives at byte offset
//! `HEADER + id * node_block_size`.
//!
//! Server-only: embedded vaults keep the in-RAM HNSW from PH23.

use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use calyx_core::Result;
use memmap2::Mmap;

use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error,
};

/// File magic at offset 0 of every `graph.cda`.
pub const DISKANN_MAGIC: [u8; 8] = *b"CLXDA001";
/// On-disk format version written into the header for new unit/cosine graphs.
pub const DISKANN_FORMAT_VERSION: u32 = 3;
/// Compact f32 node records written for raw-L2 graphs and kept readable.
pub const DISKANN_F32_FORMAT_VERSION: u32 = 2;
/// Legacy v1 node records were padded to 4 KiB.
pub const DISKANN_LEGACY_FORMAT_VERSION: u32 = 1;
/// The header remains one 4 KiB page for mmap/old-reader stability.
pub const DISKANN_BLOCK_ALIGN: usize = 4096;
/// v2 node records are cache-line aligned instead of page padded.
pub const DISKANN_NODE_ALIGN: usize = 64;
/// Upper bound on vector dimensionality accepted by the format.
pub const DISKANN_MAX_DIM: usize = 8192;
/// Upper bound on `m_max` (graph out-degree capacity) accepted by the format.
pub const DISKANN_MAX_M: usize = 512;

/// Size in bytes of one node block: vector + count + padded neighbor list,
/// rounded up to the v2 compact node alignment.
pub const fn node_block_size(dim: usize, m_max: usize) -> usize {
    compact_i8_node_block_size(dim, m_max)
}

const fn compact_f32_node_block_size(dim: usize, m_max: usize) -> usize {
    (dim * 4 + 4 + m_max * 4).div_ceil(DISKANN_NODE_ALIGN) * DISKANN_NODE_ALIGN
}

const fn compact_i8_node_block_size(dim: usize, m_max: usize) -> usize {
    (i8_payload_len(dim) + 4 + m_max * 4).div_ceil(DISKANN_NODE_ALIGN) * DISKANN_NODE_ALIGN
}

const fn i8_payload_len(dim: usize) -> usize {
    dim.div_ceil(4) * 4
}

const fn legacy_node_block_size(dim: usize, m_max: usize) -> usize {
    (dim * 4 + 4 + m_max * 4).div_ceil(DISKANN_BLOCK_ALIGN) * DISKANN_BLOCK_ALIGN
}

const fn node_block_size_for_header(header: &DiskAnnHeader) -> usize {
    match header.format_version {
        DISKANN_LEGACY_FORMAT_VERSION => {
            legacy_node_block_size(header.dim as usize, header.m_max as usize)
        }
        DISKANN_F32_FORMAT_VERSION => {
            compact_f32_node_block_size(header.dim as usize, header.m_max as usize)
        }
        DISKANN_FORMAT_VERSION => {
            compact_i8_node_block_size(header.dim as usize, header.m_max as usize)
        }
        _ => 0,
    }
}

fn corrupt(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_CORRUPT,
        format!("diskann graph corrupt: {detail}"),
    )
}

pub(super) fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("diskann invalid params: {detail}"),
    )
}

fn io_err(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("diskann {stage}: {error}"))
}

/// Fixed header written as the first `DISKANN_BLOCK_ALIGN` block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskAnnHeader {
    pub format_version: u32,
    pub dim: u32,
    pub m_max: u32,
    pub max_degree: u32,
    pub entry_point_id: u32,
    pub node_count: u64,
}

impl DiskAnnHeader {
    fn encode(&self) -> [u8; DISKANN_BLOCK_ALIGN] {
        let mut block = [0_u8; DISKANN_BLOCK_ALIGN];
        block[0..8].copy_from_slice(&DISKANN_MAGIC);
        block[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        block[12..16].copy_from_slice(&self.dim.to_le_bytes());
        block[16..20].copy_from_slice(&self.m_max.to_le_bytes());
        block[20..24].copy_from_slice(&self.max_degree.to_le_bytes());
        block[24..28].copy_from_slice(&self.entry_point_id.to_le_bytes());
        block[28..36].copy_from_slice(&self.node_count.to_le_bytes());
        block
    }

    fn decode(block: &[u8]) -> Result<Self> {
        if block.len() < 36 {
            return Err(corrupt("header block shorter than 36 bytes"));
        }
        if block[0..8] != DISKANN_MAGIC {
            return Err(corrupt(format!("bad magic {:02x?}", &block[0..8])));
        }
        let le_u32 = |at: usize| u32::from_le_bytes(block[at..at + 4].try_into().expect("4B"));
        let header = Self {
            format_version: le_u32(8),
            dim: le_u32(12),
            m_max: le_u32(16),
            max_degree: le_u32(20),
            entry_point_id: le_u32(24),
            node_count: u64::from_le_bytes(block[28..36].try_into().expect("8B")),
        };
        if !matches!(
            header.format_version,
            DISKANN_LEGACY_FORMAT_VERSION | DISKANN_F32_FORMAT_VERSION | DISKANN_FORMAT_VERSION
        ) {
            return Err(corrupt(format!("format_version {}", header.format_version)));
        }
        if header.dim == 0 || header.dim as usize > DISKANN_MAX_DIM {
            return Err(corrupt(format!(
                "dim {} out of 1..={DISKANN_MAX_DIM}",
                header.dim
            )));
        }
        if header.m_max == 0 || header.m_max as usize > DISKANN_MAX_M {
            return Err(corrupt(format!(
                "m_max {} out of 1..={DISKANN_MAX_M}",
                header.m_max
            )));
        }
        if header.max_degree > header.m_max {
            return Err(corrupt(format!("max_degree {} > m_max", header.max_degree)));
        }
        if header.node_count == 0 {
            return Err(corrupt("node_count is zero"));
        }
        if u64::from(header.entry_point_id) >= header.node_count {
            return Err(corrupt(format!(
                "entry_point_id {} >= node_count",
                header.entry_point_id
            )));
        }
        Ok(header)
    }
}

/// Sequential page-aligned writer. Stages into `<final>.tmp` in the same
/// directory (same filesystem — no `EXDEV`) and publishes atomically on
/// `finish()`; a crash never leaves a partial `graph.cda` behind.
pub struct DiskAnnGraphWriter {
    out: Option<BufWriter<File>>,
    tmp_path: PathBuf,
    final_path: PathBuf,
    header: DiskAnnHeader,
    block: usize,
    next_id: u32,
}

impl DiskAnnGraphWriter {
    pub fn create(path: &Path, header: DiskAnnHeader) -> Result<Self> {
        // Re-validate through the same gate readers use: a header we would
        // refuse to read back must never be written.
        DiskAnnHeader::decode(&header.encode())?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| io_err("create index dir", e))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp_path = PathBuf::from(tmp);
        let file = File::create(&tmp_path).map_err(|e| io_err("create tmp graph file", e))?;
        let mut out = BufWriter::new(file);
        out.write_all(&header.encode())
            .map_err(|e| io_err("write header block", e))?;
        Ok(Self {
            out: Some(out),
            tmp_path,
            final_path: path.to_path_buf(),
            header,
            block: node_block_size_for_header(&header),
            next_id: 0,
        })
    }

    pub fn write_node(&mut self, id: u32, vector: &[f32], neighbors: &[u32]) -> Result<()> {
        if id != self.next_id {
            return Err(invalid(format!(
                "node id {id} out of order (expected {})",
                self.next_id
            )));
        }
        if u64::from(id) >= self.header.node_count {
            return Err(invalid(format!(
                "node id {id} >= node_count {}",
                self.header.node_count
            )));
        }
        if vector.len() != self.header.dim as usize {
            return Err(invalid(format!("node {id} vector len {}", vector.len())));
        }
        if vector.iter().any(|v| !v.is_finite()) {
            return Err(invalid(format!(
                "node {id} vector has non-finite component"
            )));
        }
        if neighbors.len() > self.header.m_max as usize {
            return Err(invalid(format!(
                "node {id} degree {} > m_max",
                neighbors.len()
            )));
        }
        for &n in neighbors {
            if u64::from(n) >= self.header.node_count || n == id {
                return Err(invalid(format!("node {id} has invalid neighbor id {n}")));
            }
        }
        let out = self
            .out
            .as_mut()
            .ok_or_else(|| invalid("writer already finished"))?;
        let payload_len = write_vector_payload(out, self.header.format_version, vector)?;
        let count = u32::try_from(neighbors.len()).expect("<= m_max <= 512");
        out.write_all(&count.to_le_bytes())
            .map_err(|e| io_err("write count", e))?;
        for n in neighbors {
            out.write_all(&n.to_le_bytes())
                .map_err(|e| io_err("write neighbors", e))?;
        }
        let pad = self.block - (payload_len + 4 + neighbors.len() * 4);
        out.write_all(&vec![0_u8; pad])
            .map_err(|e| io_err("write pad", e))?;
        self.next_id += 1;
        Ok(())
    }

    /// Flush + fsync the staged file, then atomically rename it into place.
    pub fn finish(mut self) -> Result<()> {
        if u64::from(self.next_id) != self.header.node_count {
            return Err(invalid(format!(
                "finish after {} nodes; header promised {}",
                self.next_id, self.header.node_count
            )));
        }
        let out = self
            .out
            .take()
            .ok_or_else(|| invalid("writer already finished"))?;
        let file = out
            .into_inner()
            .map_err(|e| io_err("flush graph", e.into_error()))?;
        file.sync_all().map_err(|e| io_err("fsync graph", e))?;
        drop(file);
        fs::rename(&self.tmp_path, &self.final_path)
            .map_err(|e| io_err("publish graph (rename tmp)", e))
    }
}

impl Drop for DiskAnnGraphWriter {
    fn drop(&mut self) {
        if self.out.is_some() {
            self.out = None; // close handle before unlink (Windows)
            let _ = fs::remove_file(&self.tmp_path);
        }
    }
}

fn write_vector_payload(
    out: &mut BufWriter<File>,
    format_version: u32,
    vector: &[f32],
) -> Result<usize> {
    match format_version {
        DISKANN_LEGACY_FORMAT_VERSION | DISKANN_F32_FORMAT_VERSION => {
            for v in vector {
                out.write_all(&v.to_le_bytes())
                    .map_err(|e| io_err("write vector", e))?;
            }
            Ok(vector.len() * 4)
        }
        DISKANN_FORMAT_VERSION => {
            let row = quantize_direction_i8(vector);
            let bytes = row.iter().map(|value| *value as u8).collect::<Vec<_>>();
            out.write_all(&bytes)
                .map_err(|e| io_err("write i8 vector", e))?;
            let payload = i8_payload_len(vector.len());
            let pad = payload - vector.len();
            if pad > 0 {
                out.write_all(&vec![0_u8; pad])
                    .map_err(|e| io_err("write i8 vector pad", e))?;
            }
            Ok(payload)
        }
        other => Err(invalid(format!("unsupported graph format {other}"))),
    }
}

fn quantize_direction_i8(vector: &[f32]) -> Vec<i8> {
    let max_abs = vector
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    if max_abs == 0.0 {
        return vec![0; vector.len()];
    }
    let scale = 127.0 / max_abs;
    vector
        .iter()
        .map(|value| (value * scale).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

/// Zero-copy view of one node inside the mapped graph file.
#[derive(Debug)]
pub struct DiskAnnNodeRef<'a> {
    pub vector: DiskAnnVectorRef<'a>,
    pub neighbors: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub enum DiskAnnVectorRef<'a> {
    F32(&'a [f32]),
    I8(&'a [i8]),
}

impl DiskAnnVectorRef<'_> {
    pub fn to_vec(self) -> Vec<f32> {
        match self {
            Self::F32(values) => values.to_vec(),
            Self::I8(values) => values
                .iter()
                .map(|value| f32::from(*value))
                .collect::<Vec<_>>(),
        }
    }
}

/// mmap-backed reader. The file is published atomically by the writer and
/// never mutated afterwards; the map is read-only.
#[derive(Debug)]
pub struct DiskAnnGraphReader {
    mmap: Mmap,
    header: DiskAnnHeader,
    block: usize,
}

impl DiskAnnGraphReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| io_err("open graph file", e))?;
        let len = file
            .metadata()
            .map_err(|e| io_err("stat graph file", e))?
            .len();
        if len < DISKANN_BLOCK_ALIGN as u64 {
            return Err(corrupt(format!(
                "file is {len} B, smaller than one header block"
            )));
        }
        // SAFETY: read-only map of a file Calyx publishes atomically via
        // tmp+rename and never mutates in place; truncation mid-read would be
        // an external violation of the vault's exclusive index ownership.
        let mmap = unsafe { Mmap::map(&file).map_err(|e| io_err("mmap graph file", e))? };
        let header = DiskAnnHeader::decode(&mmap[..DISKANN_BLOCK_ALIGN])?;
        let block = node_block_size_for_header(&header);
        let expected = DISKANN_BLOCK_ALIGN as u64 + header.node_count * block as u64;
        if len != expected {
            return Err(corrupt(format!(
                "file len {len} != expected {expected} ({} x {block} B node blocks)",
                header.node_count
            )));
        }
        Ok(Self {
            mmap,
            header,
            block,
        })
    }

    pub fn header(&self) -> &DiskAnnHeader {
        &self.header
    }

    pub fn node_count(&self) -> u64 {
        self.header.node_count
    }

    pub fn node_block_size(&self) -> usize {
        self.block
    }

    pub fn node_block_offset(&self, id: u32) -> Result<u64> {
        if u64::from(id) >= self.header.node_count {
            return Err(invalid(format!(
                "node id {id} >= node_count {}",
                self.header.node_count
            )));
        }
        Ok((DISKANN_BLOCK_ALIGN + id as usize * self.block) as u64)
    }

    pub fn read_node(&self, id: u32) -> Result<DiskAnnNodeRef<'_>> {
        if u64::from(id) >= self.header.node_count {
            return Err(invalid(format!(
                "node id {id} >= node_count {}",
                self.header.node_count
            )));
        }
        let dim = self.header.dim as usize;
        let start = DISKANN_BLOCK_ALIGN + id as usize * self.block;
        let bytes = &self.mmap[start..start + self.block];
        let (vector, count_at) = match self.header.format_version {
            DISKANN_LEGACY_FORMAT_VERSION | DISKANN_F32_FORMAT_VERSION => {
                let vector = cast_le_slice::<f32>(&bytes[..dim * 4], "vector")?;
                (DiskAnnVectorRef::F32(vector), dim * 4)
            }
            DISKANN_FORMAT_VERSION => {
                let vector = cast_le_slice::<i8>(&bytes[..dim], "i8 vector")?;
                (DiskAnnVectorRef::I8(vector), i8_payload_len(dim))
            }
            other => return Err(corrupt(format!("format_version {other}"))),
        };
        let count =
            u32::from_le_bytes(bytes[count_at..count_at + 4].try_into().expect("4B")) as usize;
        if count > self.header.m_max as usize {
            return Err(corrupt(format!("node {id} neighbor_count {count} > m_max")));
        }
        let nb_at = count_at + 4;
        let neighbors = cast_le_slice::<u32>(&bytes[nb_at..nb_at + count * 4], "neighbors")?;
        Ok(DiskAnnNodeRef { vector, neighbors })
    }
}

/// Reinterpret little-endian on-disk bytes as `&[T]` without copying. Fails
/// closed (`CALYX_INDEX_CORRUPT`) if the region is misaligned rather than
/// panicking; blocks are 4 KiB-aligned within a page-aligned map, so a
/// misalignment can only mean a corrupt/foreign file.
fn cast_le_slice<'a, T>(bytes: &'a [u8], what: &str) -> Result<&'a [T]> {
    debug_assert_eq!(bytes.len() % size_of::<T>(), 0);
    if bytes.as_ptr().align_offset(align_of::<T>()) != 0 {
        return Err(corrupt(format!(
            "{what} region misaligned for zero-copy read"
        )));
    }
    // SAFETY: alignment checked above; length is an exact multiple of the
    // element size; f32/u32 accept any bit pattern; lifetime tied to the map.
    Ok(unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast(), bytes.len() / size_of::<T>()) })
}

/// Open an existing `graph.cda` for zero-copy reads.
pub fn open_diskann_graph(path: &Path) -> Result<DiskAnnGraphReader> {
    DiskAnnGraphReader::open(path)
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod graph_tests;
