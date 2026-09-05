use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, SlotId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Kernel, LodestarError, Result};

const FORMAT_VERSION: u32 = 1;
const PANEL_FORMAT_VERSION: u32 = 2;
pub const PANEL_RRF_K: u32 = 60;

pub trait EmbeddingStore {
    fn embedding(&self, cx_id: CxId) -> Result<Option<Vec<f32>>>;
}

impl EmbeddingStore for BTreeMap<CxId, Vec<f32>> {
    fn embedding(&self, cx_id: CxId) -> Result<Option<Vec<f32>>> {
        Ok(self.get(&cx_id).cloned())
    }
}

pub trait KernelStore {
    fn write_index_bytes(&self, kernel_id: CxId, bytes: &[u8]) -> Result<()>;
    fn read_index_bytes(&self, kernel_id: CxId) -> Result<Option<Vec<u8>>>;

    /// Install the immutable kernel index by streaming its serialized bytes
    /// through `write` straight to durable storage, without ever materializing
    /// the whole artifact (which for panel kernels is hundreds of MB) as one
    /// in-memory buffer. The default implementation buffers and delegates to
    /// [`Self::write_index_bytes`] for stores that have no streaming path; the
    /// on-disk [`FsKernelStore`] overrides it with a temp-file + fsync install.
    fn install_index_streaming(
        &self,
        kernel_id: CxId,
        write: &dyn Fn(&mut dyn Write) -> Result<()>,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        write(&mut buffer)?;
        self.write_index_bytes(kernel_id, &buffer)
    }

    /// Open a streaming reader over the kernel index artifact, or `None` when it
    /// does not exist. The default implementation reads the whole artifact into
    /// memory; [`FsKernelStore`] overrides it with a buffered file reader so a
    /// large index is parsed incrementally instead of copied into a heap buffer
    /// before parsing.
    fn read_index_reader(&self, kernel_id: CxId) -> Result<Option<Box<dyn Read>>> {
        Ok(self
            .read_index_bytes(kernel_id)?
            .map(|bytes| Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>))
    }
}

#[derive(Clone, Debug)]
pub struct FsKernelStore {
    root: PathBuf,
}

impl FsKernelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn index_dir(&self, kernel_id: CxId) -> PathBuf {
        self.root
            .join("idx")
            .join("kernel")
            .join(kernel_id.to_string())
    }

    pub fn index_file_path(&self, kernel_id: CxId) -> PathBuf {
        self.index_dir(kernel_id).join("index.json")
    }

    pub fn kernel_file_path(&self, kernel_id: CxId) -> PathBuf {
        self.index_dir(kernel_id).join("kernel.json")
    }
}

impl KernelStore for FsKernelStore {
    fn write_index_bytes(&self, kernel_id: CxId, bytes: &[u8]) -> Result<()> {
        let dir = self.index_dir(kernel_id);
        let path = dir.join("index.json");
        install_immutable_file(&path, bytes)
    }

    fn read_index_bytes(&self, kernel_id: CxId) -> Result<Option<Vec<u8>>> {
        let path = self.index_file_path(kernel_id);
        if !Path::new(&path).exists() {
            return Ok(None);
        }
        fs::read(path).map(Some).map_err(io_error)
    }

    fn install_index_streaming(
        &self,
        kernel_id: CxId,
        write: &dyn Fn(&mut dyn Write) -> Result<()>,
    ) -> Result<()> {
        let path = self.index_dir(kernel_id).join("index.json");
        install_immutable_streaming(&path, write)
    }

    fn read_index_reader(&self, kernel_id: CxId) -> Result<Option<Box<dyn Read>>> {
        let path = self.index_file_path(kernel_id);
        if !Path::new(&path).exists() {
            return Ok(None);
        }
        let file = fs::File::open(&path).map_err(io_error)?;
        Ok(Some(Box::new(BufReader::new(file)) as Box<dyn Read>))
    }
}

pub(crate) fn install_immutable_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| LodestarError::KernelIndexIo {
        detail: format!("immutable artifact path {} has no parent", path.display()),
    })?;
    fs::create_dir_all(parent).map_err(io_error)?;
    if path.exists() {
        let existing = fs::read(path).map_err(io_error)?;
        if existing == bytes {
            return Ok(());
        }
        return Err(LodestarError::KernelIndexIo {
            detail: format!(
                "refusing to replace immutable kernel artifact {} with different bytes",
                path.display()
            ),
        });
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| LodestarError::KernelIndexIo {
            detail: format!("immutable artifact path {} has no filename", path.display()),
        })?
        .to_string_lossy();
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(io_error)?;
    let publish = (|| {
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        fs::rename(&tmp, path).map_err(io_error)?;
        sync_parent(parent)
    })();
    if publish.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    publish
}

/// Streaming counterpart to [`install_immutable_file`]. A new artifact is
/// streamed to a sibling temp file, fsynced, and atomically installed. When the
/// immutable target already exists, serialization is compared directly against
/// it without a redundant temp-file write; any byte or length drift fails
/// closed.
pub(crate) fn install_immutable_streaming(
    path: &Path,
    write: &dyn Fn(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    let parent = path.parent().ok_or_else(|| LodestarError::KernelIndexIo {
        detail: format!("immutable artifact path {} has no parent", path.display()),
    })?;
    fs::create_dir_all(parent).map_err(io_error)?;
    if path.exists() {
        return compare_immutable_stream(path, write);
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| LodestarError::KernelIndexIo {
            detail: format!("immutable artifact path {} has no filename", path.display()),
        })?
        .to_string_lossy();
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    // Clear any stale temp left by a crashed run in this pid slot so create_new
    // does not spuriously fail.
    let _ = fs::remove_file(&tmp);
    let streamed = (|| -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(io_error)?;
        let mut writer = BufWriter::new(file);
        write(&mut writer)?;
        writer.flush().map_err(io_error)?;
        let file = writer
            .into_inner()
            .map_err(|error| LodestarError::KernelIndexIo {
                detail: format!("flush streamed artifact {}: {error}", tmp.display()),
            })?;
        file.sync_all().map_err(io_error)
    })();
    if let Err(error) = streamed {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_error(error));
    }
    sync_parent(parent)
}

/// Prove an idempotent immutable install without writing a second multi-GB
/// artifact: serialize into a buffered comparison writer that consumes the
/// existing file at the same byte offsets. A differing byte or length fails
/// closed and names the first physical mismatch.
fn compare_immutable_stream(
    path: &Path,
    write: &dyn Fn(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    let existing = fs::File::open(path).map_err(io_error)?;
    let mut comparator = ImmutableCompareWriter::new(BufReader::new(existing));
    {
        let mut writer = BufWriter::with_capacity(1024 * 1024, &mut comparator);
        write(&mut writer)?;
        writer.flush().map_err(io_error)?;
    }
    comparator.finish(path)
}

struct ImmutableCompareWriter<R> {
    existing: R,
    scratch: Vec<u8>,
    compared_bytes: u64,
    mismatch: Option<String>,
}

impl<R: Read> ImmutableCompareWriter<R> {
    fn new(existing: R) -> Self {
        Self {
            existing,
            scratch: Vec::new(),
            compared_bytes: 0,
            mismatch: None,
        }
    }

    fn finish(mut self, path: &Path) -> Result<()> {
        if self.mismatch.is_none() {
            let mut trailing = [0_u8; 1];
            match self.existing.read(&mut trailing).map_err(io_error)? {
                0 => return Ok(()),
                _ => {
                    self.mismatch = Some(format!(
                        "serialized bytes ended at offset {} but the existing file has trailing bytes",
                        self.compared_bytes
                    ));
                }
            }
        }
        Err(LodestarError::KernelIndexIo {
            detail: format!(
                "refusing to replace immutable kernel artifact {}: {}",
                path.display(),
                self.mismatch
                    .as_deref()
                    .unwrap_or("streamed bytes differed")
            ),
        })
    }
}

impl<R: Read> Write for ImmutableCompareWriter<R> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.mismatch.is_some() {
            self.compared_bytes = self.compared_bytes.saturating_add(bytes.len() as u64);
            return Ok(bytes.len());
        }
        self.scratch.resize(bytes.len(), 0);
        let mut filled = 0usize;
        while filled < bytes.len() {
            match self.existing.read(&mut self.scratch[filled..]) {
                Ok(0) => {
                    self.mismatch = Some(format!(
                        "existing file ended at offset {} while serialized bytes continued",
                        self.compared_bytes + filled as u64
                    ));
                    break;
                }
                Ok(count) => filled += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        if self.mismatch.is_none()
            && let Some(offset) = bytes
                .iter()
                .zip(&self.scratch)
                .position(|(expected, observed)| expected != observed)
        {
            self.mismatch = Some(format!(
                "first differing byte at offset {}",
                self.compared_bytes + offset as u64
            ));
        }
        self.compared_bytes = self.compared_bytes.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelVectorRow {
    pub cx_id: CxId,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct KernelIndex {
    pub kernel_id: CxId,
    pub dim: usize,
    rows: Vec<KernelVectorRow>,
}

impl KernelIndex {
    pub fn rows(&self) -> &[KernelVectorRow] {
        &self.rows
    }

    pub fn filter_to_nodes(&self, allowed_nodes: &BTreeSet<CxId>) -> Result<Self> {
        let rows = self
            .rows
            .iter()
            .filter(|row| allowed_nodes.contains(&row.cx_id))
            .cloned()
            .collect::<Vec<_>>();
        Self::from_rows(self.kernel_id, rows)
    }

    fn from_rows(kernel_id: CxId, rows: Vec<KernelVectorRow>) -> Result<Self> {
        let dim = validate_rows(&rows)?;
        Ok(Self {
            kernel_id,
            dim,
            rows,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct KernelIndexSnapshot {
    format_version: u32,
    kernel_id: CxId,
    dim: usize,
    rows: Vec<KernelVectorRow>,
}

pub fn build_kernel_index(kernel: &Kernel, embeddings: &dyn EmbeddingStore) -> Result<KernelIndex> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let rows = kernel
        .members
        .iter()
        .map(|cx_id| {
            let vector = embeddings
                .embedding(*cx_id)?
                .ok_or(LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
            Ok(KernelVectorRow {
                cx_id: *cx_id,
                vector,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    KernelIndex::from_rows(kernel.kernel_id, rows)
}

pub fn kernel_search(
    index: &KernelIndex,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Vec<(CxId, f32)>> {
    if query_vec.len() != index.dim {
        return Err(LodestarError::KernelDimMismatch {
            expected: index.dim,
            actual: query_vec.len(),
        });
    }
    if let Some((offset, _)) = query_vec
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!("query vector has non-finite value at offset {offset}"),
        });
    }
    Ok(top_k_by_score(
        index
            .rows
            .par_iter()
            .map(|row| (row.cx_id, cosine(query_vec, &row.vector)))
            .collect(),
        top_k,
    ))
}

pub fn write_kernel_index(index: &KernelIndex, store: &dyn KernelStore) -> Result<()> {
    let snapshot = KernelIndexSnapshot {
        format_version: FORMAT_VERSION,
        kernel_id: index.kernel_id,
        dim: index.dim,
        rows: index.rows.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot).map_err(codec_error)?;
    store.write_index_bytes(index.kernel_id, &bytes)
}

pub fn load_kernel_index(kernel_id: CxId, store: &dyn KernelStore) -> Result<KernelIndex> {
    let Some(bytes) = store.read_index_bytes(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let snapshot: KernelIndexSnapshot = serde_json::from_slice(&bytes).map_err(codec_error)?;
    if snapshot.format_version != FORMAT_VERSION {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!("unsupported format version {}", snapshot.format_version),
        });
    }
    if snapshot.kernel_id != kernel_id {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "snapshot kernel id {} did not match requested {}",
                snapshot.kernel_id, kernel_id
            ),
        });
    }
    let actual_dim = validate_rows(&snapshot.rows)?;
    if snapshot.dim != actual_dim {
        return Err(LodestarError::KernelDimMismatch {
            expected: snapshot.dim,
            actual: actual_dim,
        });
    }
    KernelIndex::from_rows(snapshot.kernel_id, snapshot.rows)
}

fn validate_rows(rows: &[KernelVectorRow]) -> Result<usize> {
    if rows.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let dim = rows[0].vector.len();
    if dim == 0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: "kernel vectors must have non-zero dimension".to_string(),
        });
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.cx_id) {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("duplicate kernel row {}", row.cx_id),
            });
        }
        if row.vector.len() != dim {
            return Err(LodestarError::KernelDimMismatch {
                expected: dim,
                actual: row.vector.len(),
            });
        }
        if let Some((offset, _)) = row
            .vector
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("row {} has non-finite value at offset {offset}", row.cx_id),
            });
        }
    }
    Ok(dim)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut an = 0.0_f32;
    let mut bn = 0.0_f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        0.0
    } else {
        dot / (an.sqrt() * bn.sqrt())
    }
}

fn top_k_by_score(mut scored: Vec<(CxId, f32)>, top_k: usize) -> Vec<(CxId, f32)> {
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.truncate(top_k);
    scored
}

fn io_error(err: std::io::Error) -> LodestarError {
    LodestarError::KernelIndexIo {
        detail: err.to_string(),
    }
}

fn codec_error(err: serde_json::Error) -> LodestarError {
    LodestarError::KernelIndexCodec {
        detail: err.to_string(),
    }
}

/// A no-flatten panel measurement. Every contracted slot remains separately
/// addressable; fusion operates on ranks and never concatenates vectors.
pub type PanelVectors = BTreeMap<SlotId, Vec<f32>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelFusionLane {
    pub slot: SlotId,
    pub cosine: f32,
    pub rank: usize,
    pub rrf_contribution: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelFusionHit {
    pub cx_id: CxId,
    pub score: f32,
    pub lanes: Vec<PanelFusionLane>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelKernelVectorRow {
    pub cx_id: CxId,
    pub vectors: PanelVectors,
}

#[derive(Clone, Debug)]
pub struct PanelKernelIndex {
    pub kernel_id: CxId,
    slots: Vec<SlotId>,
    dims: BTreeMap<SlotId, usize>,
    rows: Vec<PanelKernelVectorRow>,
}

impl PanelKernelIndex {
    pub fn slots(&self) -> &[SlotId] {
        &self.slots
    }

    pub fn dims(&self) -> &BTreeMap<SlotId, usize> {
        &self.dims
    }

    pub fn rows(&self) -> &[PanelKernelVectorRow] {
        &self.rows
    }

    fn from_rows(
        kernel_id: CxId,
        slots: Vec<SlotId>,
        rows: Vec<PanelKernelVectorRow>,
    ) -> Result<Self> {
        // Validate the panel over borrowed row vectors so the index holds each
        // member's per-lane vectors exactly once instead of keeping a second
        // full `candidates` clone co-resident (issues #1863/#1864).
        let candidate_refs = rows
            .iter()
            .map(|row| (row.cx_id, &row.vectors))
            .collect::<BTreeMap<_, _>>();
        if candidate_refs.len() != rows.len() {
            return Err(LodestarError::KernelInvalidParams {
                detail: "panel kernel index contains duplicate constellation ids".to_string(),
            });
        }
        let dims = validate_panel_refs(&slots, &candidate_refs)?;
        Ok(Self {
            kernel_id,
            slots,
            dims,
            rows,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PanelKernelIndexSnapshot {
    format_version: u32,
    kernel_id: CxId,
    slots: Vec<SlotId>,
    dims: BTreeMap<SlotId, usize>,
    fusion: String,
    rrf_k: u32,
    rows: Vec<PanelKernelVectorRow>,
}

/// Borrowing view of a [`PanelKernelIndex`] used only for streamed
/// serialization. It serializes byte-for-byte identically to
/// [`PanelKernelIndexSnapshot`] (same field order and types) but holds the rows
/// by reference, so `write_panel_kernel_index` never clones the full member
/// vectors into an owned snapshot nor into an intermediate `Vec<u8>`.
#[derive(Serialize)]
struct PanelKernelIndexSnapshotRef<'a> {
    format_version: u32,
    kernel_id: CxId,
    slots: &'a [SlotId],
    dims: &'a BTreeMap<SlotId, usize>,
    fusion: &'a str,
    rrf_k: u32,
    rows: &'a [PanelKernelVectorRow],
}

/// Streamed readback summary of a panel kernel index. Produced by
/// [`verify_panel_kernel_index`] without ever materializing the full parsed
/// index in memory — it validates the contract header and every row row-by-row
/// and returns only the small counts/contract fields callers need to prove the
/// on-disk artifact matches the built kernel.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelKernelIndexReadback {
    pub kernel_id: CxId,
    pub slots: Vec<SlotId>,
    pub dims: BTreeMap<SlotId, usize>,
    pub row_count: usize,
}

pub fn build_panel_kernel_index(
    kernel: &Kernel,
    slots: &[SlotId],
    embeddings: &BTreeMap<CxId, PanelVectors>,
) -> Result<PanelKernelIndex> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let rows = kernel
        .members
        .iter()
        .map(|cx_id| {
            let vectors = embeddings
                .get(cx_id)
                .cloned()
                .ok_or(LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
            Ok(PanelKernelVectorRow {
                cx_id: *cx_id,
                vectors,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    PanelKernelIndex::from_rows(kernel.kernel_id, slots.to_vec(), rows)
}

pub fn panel_kernel_search(
    index: &PanelKernelIndex,
    query: &PanelVectors,
    top_k: usize,
) -> Result<Vec<PanelFusionHit>> {
    // Rank over borrowed row vectors — the panel index no longer keeps a second
    // owned `candidates` map, so search reads the members it already holds. The
    // borrowing ranker is bit-for-bit equivalent to the owned one on success:
    // same BTreeMap key order, cosine, lane sort, RRF math, and hit ordering.
    let candidates = index
        .rows
        .iter()
        .map(|row| (row.cx_id, &row.vectors))
        .collect::<BTreeMap<_, _>>();
    let mut hits = rank_panel_candidate_refs(query, &candidates, &index.slots, PANEL_RRF_K)?;
    hits.truncate(top_k);
    Ok(hits)
}

pub fn write_panel_kernel_index(index: &PanelKernelIndex, store: &dyn KernelStore) -> Result<()> {
    // Serialize straight to the durable writer via a borrowing snapshot: no
    // owned row clone and no whole-file `Vec<u8>`. `to_writer_pretty` emits the
    // exact same bytes as the previous `to_vec_pretty`, so on-disk artifacts and
    // their recorded sha256 are unchanged (issue #1863).
    let snapshot = PanelKernelIndexSnapshotRef {
        format_version: PANEL_FORMAT_VERSION,
        kernel_id: index.kernel_id,
        slots: &index.slots,
        dims: &index.dims,
        fusion: "rrf",
        rrf_k: PANEL_RRF_K,
        rows: &index.rows,
    };
    store.install_index_streaming(index.kernel_id, &|writer| {
        serde_json::to_writer_pretty(writer, &snapshot).map_err(codec_error)
    })
}

pub fn load_panel_kernel_index(
    kernel_id: CxId,
    store: &dyn KernelStore,
) -> Result<PanelKernelIndex> {
    // Stream-parse from a buffered reader so the raw index bytes are not first
    // copied into a heap `Vec<u8>`; the parsed rows themselves are the working
    // index the caller needs (issues #1863/#1864).
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let snapshot: PanelKernelIndexSnapshot =
        serde_json::from_reader(reader).map_err(codec_error)?;
    if snapshot.format_version != PANEL_FORMAT_VERSION
        || snapshot.kernel_id != kernel_id
        || snapshot.fusion != "rrf"
        || snapshot.rrf_k != PANEL_RRF_K
    {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "invalid panel kernel index contract format={} kernel={} fusion={} rrf_k={}",
                snapshot.format_version, snapshot.kernel_id, snapshot.fusion, snapshot.rrf_k
            ),
        });
    }
    let index = PanelKernelIndex::from_rows(snapshot.kernel_id, snapshot.slots, snapshot.rows)?;
    if index.dims != snapshot.dims {
        return Err(LodestarError::KernelIndexCodec {
            detail: "panel kernel index dimensions differ from physical row readback".to_string(),
        });
    }
    Ok(index)
}

/// Streamed readback verification of an on-disk panel kernel index.
///
/// This is the low-memory counterpart to [`load_panel_kernel_index`] for the
/// build/publish readback path, which only needs to prove the artifact parses,
/// honours the contract header, and has the expected member count — it does NOT
/// need the whole index resident. The rows are parsed and validated one at a
/// time from a buffered reader (same per-row contract as `from_rows`:
/// no-duplicate ids, exact slot set, declared dims, finite non-zero lanes) and
/// discarded, so peak memory is ~one row plus the parser buffer instead of the
/// full multi-hundred-MB index (issue #1863).
pub fn verify_panel_kernel_index(
    kernel_id: CxId,
    store: &dyn KernelStore,
) -> Result<PanelKernelIndexReadback> {
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let readback = PanelKernelIndexReadback::deserialize(&mut deserializer).map_err(codec_error)?;
    // Reject trailing non-whitespace bytes so a truncated/appended artifact
    // fails closed rather than being silently accepted.
    deserializer.end().map_err(codec_error)?;
    if readback.kernel_id != kernel_id {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "panel kernel index kernel id {} did not match requested {kernel_id}",
                readback.kernel_id
            ),
        });
    }
    Ok(readback)
}

impl<'de> Deserialize<'de> for PanelKernelIndexReadback {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(PanelReadbackVisitor)
    }
}

struct PanelReadbackVisitor;

impl<'de> serde::de::Visitor<'de> for PanelReadbackVisitor {
    type Value = PanelKernelIndexReadback;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a panel kernel index object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        use serde::de::Error as _;
        let mut format_version: Option<u32> = None;
        let mut kernel_id: Option<CxId> = None;
        let mut slots: Option<Vec<SlotId>> = None;
        let mut dims: Option<BTreeMap<SlotId, usize>> = None;
        let mut fusion: Option<String> = None;
        let mut rrf_k: Option<u32> = None;
        let mut row_count: Option<usize> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "format_version" => format_version = Some(map.next_value()?),
                "kernel_id" => kernel_id = Some(map.next_value()?),
                "slots" => slots = Some(map.next_value()?),
                "dims" => dims = Some(map.next_value()?),
                "fusion" => fusion = Some(map.next_value()?),
                "rrf_k" => rrf_k = Some(map.next_value()?),
                "rows" => {
                    // Rows are the last field in the serialized contract, so
                    // slots+dims are already parsed and available to validate
                    // each streamed row against.
                    let slots = slots.as_ref().ok_or_else(|| {
                        A::Error::custom("panel kernel index rows appeared before slots")
                    })?;
                    let dims = dims.as_ref().ok_or_else(|| {
                        A::Error::custom("panel kernel index rows appeared before dims")
                    })?;
                    row_count = Some(map.next_value_seed(RowStreamSeed { slots, dims })?);
                }
                other => {
                    return Err(A::Error::custom(format!(
                        "unexpected panel kernel index field {other}"
                    )));
                }
            }
        }
        let format_version =
            format_version.ok_or_else(|| A::Error::missing_field("format_version"))?;
        if format_version != PANEL_FORMAT_VERSION {
            return Err(A::Error::custom(format!(
                "unsupported panel kernel index format version {format_version}"
            )));
        }
        let fusion = fusion.ok_or_else(|| A::Error::missing_field("fusion"))?;
        if fusion != "rrf" {
            return Err(A::Error::custom(format!(
                "unsupported panel fusion {fusion}"
            )));
        }
        let rrf_k = rrf_k.ok_or_else(|| A::Error::missing_field("rrf_k"))?;
        if rrf_k != PANEL_RRF_K {
            return Err(A::Error::custom(format!("unsupported panel rrf_k {rrf_k}")));
        }
        Ok(PanelKernelIndexReadback {
            kernel_id: kernel_id.ok_or_else(|| A::Error::missing_field("kernel_id"))?,
            slots: slots.ok_or_else(|| A::Error::missing_field("slots"))?,
            dims: dims.ok_or_else(|| A::Error::missing_field("dims"))?,
            row_count: row_count.ok_or_else(|| A::Error::missing_field("rows"))?,
        })
    }
}

/// Deserialize seed that streams the `rows` array element-by-element, validating
/// each row against the contract slots/dims and counting them without retaining
/// any row.
struct RowStreamSeed<'a> {
    slots: &'a [SlotId],
    dims: &'a BTreeMap<SlotId, usize>,
}

impl<'de> serde::de::DeserializeSeed<'de> for RowStreamSeed<'_> {
    type Value = usize;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<usize, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> serde::de::Visitor<'de> for RowStreamSeed<'_> {
    type Value = usize;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of panel kernel vector rows")
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<usize, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        use serde::de::Error as _;
        // `validate_panel_vector` indexes `dims` by every contract slot, so the
        // declared dims must cover exactly the slot set before we stream rows.
        if self.dims.len() != self.slots.len()
            || self.slots.iter().any(|slot| !self.dims.contains_key(slot))
        {
            return Err(A::Error::custom(
                "panel kernel index dims differ from the declared slot contract",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut count = 0_usize;
        while let Some(row) = seq.next_element::<PanelKernelVectorRow>()? {
            if !seen.insert(row.cx_id) {
                return Err(A::Error::custom(format!(
                    "panel kernel index contains duplicate constellation id {}",
                    row.cx_id
                )));
            }
            validate_panel_vector(
                &format!("row {}", row.cx_id),
                &row.vectors,
                self.slots,
                self.dims,
            )
            .map_err(|error| A::Error::custom(error.to_string()))?;
            count += 1;
        }
        if count == 0 {
            return Err(A::Error::custom("panel kernel index has no rows"));
        }
        Ok(count)
    }
}

/// Deterministically rank a candidate set through every contracted lane using
/// normalized Reciprocal Rank Fusion. Missing, extra, malformed, or non-finite
/// lane values fail closed before any score is emitted.
pub fn rank_panel_candidates(
    query: &PanelVectors,
    candidates: &BTreeMap<CxId, PanelVectors>,
    slots: &[SlotId],
    rrf_k: u32,
) -> Result<Vec<PanelFusionHit>> {
    let dims = validate_panel(slots, candidates)?;
    validate_panel_vector("query", query, slots, &dims)?;
    if rrf_k == 0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: "panel RRF K must be greater than zero".to_string(),
        });
    }
    let mut evidence = candidates
        .keys()
        .map(|id| (*id, Vec::with_capacity(slots.len())))
        .collect::<BTreeMap<_, _>>();
    let normalizer = (rrf_k as f32 + 1.0) / slots.len() as f32;
    for &slot in slots {
        let query_vector = query.get(&slot).expect("validated query slot");
        let mut lane = candidates
            .par_iter()
            .map(|(id, vectors)| {
                let vector = vectors.get(&slot).expect("validated candidate slot");
                (*id, cosine(query_vector, vector))
            })
            .collect::<Vec<_>>();
        lane.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        for (offset, (id, similarity)) in lane.into_iter().enumerate() {
            let rank = offset + 1;
            let rrf_contribution = normalizer / (rrf_k as f32 + rank as f32);
            evidence
                .get_mut(&id)
                .expect("candidate evidence")
                .push(PanelFusionLane {
                    slot,
                    cosine: similarity,
                    rank,
                    rrf_contribution,
                });
        }
    }
    let mut hits = evidence
        .into_iter()
        .map(|(cx_id, lanes)| PanelFusionHit {
            cx_id,
            score: lanes.iter().map(|lane| lane.rrf_contribution).sum(),
            lanes,
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.cx_id.cmp(&right.cx_id))
    });
    Ok(hits)
}

/// Borrowing variant used by graph construction so candidate generation does
/// not clone every high-dimensional panel for every source node.
pub fn rank_panel_candidate_refs(
    query: &PanelVectors,
    candidates: &BTreeMap<CxId, &PanelVectors>,
    slots: &[SlotId],
    rrf_k: u32,
) -> Result<Vec<PanelFusionHit>> {
    if slots.len() < 2 || candidates.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "panel fusion needs at least two slots and one candidate; slots={} candidates={}",
                slots.len(),
                candidates.len()
            ),
        });
    }
    if !slots.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(LodestarError::KernelInvalidParams {
            detail: "panel fusion slots must be strictly increasing and unique".to_string(),
        });
    }
    let first = *candidates.values().next().expect("nonempty candidates");
    let dims = slots
        .iter()
        .map(|slot| (*slot, first.get(slot).map(Vec::len).unwrap_or(0)))
        .collect::<BTreeMap<_, _>>();
    validate_panel_vector("query", query, slots, &dims)?;
    for (id, vectors) in candidates {
        validate_panel_vector(&format!("candidate {id}"), vectors, slots, &dims)?;
    }
    if rrf_k == 0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: "panel RRF K must be greater than zero".to_string(),
        });
    }
    let mut evidence = candidates
        .keys()
        .map(|id| (*id, Vec::with_capacity(slots.len())))
        .collect::<BTreeMap<_, _>>();
    let normalizer = (rrf_k as f32 + 1.0) / slots.len() as f32;
    for &slot in slots {
        let query_vector = query.get(&slot).expect("validated query slot");
        let mut lane = candidates
            .par_iter()
            .map(|(id, vectors)| {
                let vector = vectors.get(&slot).expect("validated candidate slot");
                (*id, cosine(query_vector, vector))
            })
            .collect::<Vec<_>>();
        lane.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        for (offset, (id, similarity)) in lane.into_iter().enumerate() {
            let rank = offset + 1;
            evidence
                .get_mut(&id)
                .expect("candidate evidence")
                .push(PanelFusionLane {
                    slot,
                    cosine: similarity,
                    rank,
                    rrf_contribution: normalizer / (rrf_k as f32 + rank as f32),
                });
        }
    }
    let mut hits = evidence
        .into_iter()
        .map(|(cx_id, lanes)| PanelFusionHit {
            cx_id,
            score: lanes.iter().map(|lane| lane.rrf_contribution).sum(),
            lanes,
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.cx_id.cmp(&right.cx_id))
    });
    Ok(hits)
}

fn validate_panel(
    slots: &[SlotId],
    candidates: &BTreeMap<CxId, PanelVectors>,
) -> Result<BTreeMap<SlotId, usize>> {
    let refs = candidates
        .iter()
        .map(|(id, vectors)| (*id, vectors))
        .collect::<BTreeMap<_, _>>();
    validate_panel_refs(slots, &refs)
}

/// Borrowing panel validation shared by [`PanelKernelIndex::from_rows`] and the
/// owned [`validate_panel`]. Validating over `&PanelVectors` lets the index be
/// built and readback-verified without cloning every member's per-lane vectors.
fn validate_panel_refs(
    slots: &[SlotId],
    candidates: &BTreeMap<CxId, &PanelVectors>,
) -> Result<BTreeMap<SlotId, usize>> {
    if slots.len() < 2 || candidates.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "panel fusion needs at least two slots and one candidate; slots={} candidates={}",
                slots.len(),
                candidates.len()
            ),
        });
    }
    let ordered = slots.windows(2).all(|pair| pair[0] < pair[1]);
    if !ordered {
        return Err(LodestarError::KernelInvalidParams {
            detail: "panel fusion slots must be strictly increasing and unique".to_string(),
        });
    }
    let first = *candidates.values().next().expect("nonempty candidates");
    let dims = slots
        .iter()
        .map(|slot| {
            let dim = first.get(slot).map(Vec::len).unwrap_or(0);
            (*slot, dim)
        })
        .collect::<BTreeMap<_, _>>();
    for (id, vectors) in candidates {
        validate_panel_vector(&format!("candidate {id}"), vectors, slots, &dims)?;
    }
    Ok(dims)
}

fn validate_panel_vector(
    label: &str,
    vectors: &PanelVectors,
    slots: &[SlotId],
    dims: &BTreeMap<SlotId, usize>,
) -> Result<()> {
    if vectors.len() != slots.len() || vectors.keys().copied().ne(slots.iter().copied()) {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "{label} panel slots {:?} differ from contract {:?}",
                vectors.keys().map(|slot| slot.get()).collect::<Vec<_>>(),
                slots.iter().map(|slot| slot.get()).collect::<Vec<_>>()
            ),
        });
    }
    for &slot in slots {
        let vector = vectors.get(&slot).expect("validated slot key");
        let expected = dims[&slot];
        if expected == 0 || vector.len() != expected {
            return Err(LodestarError::KernelDimMismatch {
                expected,
                actual: vector.len(),
            });
        }
        if let Some(offset) = vector.iter().position(|value| !value.is_finite()) {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("{label} slot {slot} has non-finite value at offset {offset}"),
            });
        }
        let norm = vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>();
        if !norm.is_finite() || norm == 0.0 {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("{label} slot {slot} has invalid squared norm {norm}"),
            });
        }
    }
    Ok(())
}

pub fn panel_kernel_recall_test(
    index: &PanelKernelIndex,
    full: &BTreeMap<CxId, PanelVectors>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    validate_panel_recall_params(params, full)?;
    let held_out = panel_held_out(full, params.held_out_fraction, params.rng_seed);
    if held_out.is_empty() {
        return Err(LodestarError::RecallEmptyCorpus);
    }
    let mut total = 0.0_f32;
    for id in &held_out {
        let query = full.get(id).expect("held-out panel row");
        let mut full_hits = rank_panel_candidates(query, full, &index.slots, PANEL_RRF_K)?;
        full_hits.truncate(params.top_k);
        let kernel_hits = panel_kernel_search(index, query, params.top_k)?;
        let kernel_ids = kernel_hits
            .iter()
            .map(|hit| hit.cx_id)
            .collect::<BTreeSet<_>>();
        let overlap = full_hits
            .iter()
            .filter(|hit| kernel_ids.contains(&hit.cx_id))
            .count();
        total += overlap as f32 / full_hits.len() as f32;
    }
    let ratio = total / held_out.len() as f32;
    Ok(crate::RecallReport {
        kernel_only: ratio,
        full: 1.0,
        ratio,
        approx_factor: 1.0,
        tau_star_estimate: 0,
        tau_star_exact: true,
        recall_test_params: Some(params.clone()),
        corpus_name: Some(corpus_name.to_string()),
        n_queries_tested: held_out.len(),
        held_out,
        warning: (ratio < params.min_recall_ratio).then(|| {
            format!(
                "{}: ratio={ratio:.6} min={:.6}",
                crate::recall_test::CALYX_KERNEL_RECALL_BELOW_GATE,
                params.min_recall_ratio
            )
        }),
    })
}

pub fn panel_kernel_recall_gate(
    index: &PanelKernelIndex,
    full: &BTreeMap<CxId, PanelVectors>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    let report = panel_kernel_recall_test(index, full, params, corpus_name)?;
    if report.ratio < params.min_recall_ratio {
        return Err(LodestarError::RecallBelowGate {
            ratio: report.ratio,
            min: params.min_recall_ratio,
        });
    }
    Ok(report)
}

pub fn panel_full_topk_support_set(
    slots: &[SlotId],
    full: &BTreeMap<CxId, PanelVectors>,
    params: &crate::RecallTestParams,
) -> Result<crate::RecallSupportReport> {
    panel_rank_stabilization_support_set(slots, full, params, 0)
}

/// Extracts exact fused hits plus per-lane rank prefixes for the deterministic
/// held-out recall set. RRF ranks are relative to the candidate population, so
/// retaining only the fused winners can reorder a reduced kernel even when all
/// full-corpus winners are present. The lane prefixes preserve the rank
/// witnesses needed to stabilize that ordering without collapsing the panel.
pub fn panel_rank_stabilization_support_set(
    slots: &[SlotId],
    full: &BTreeMap<CxId, PanelVectors>,
    params: &crate::RecallTestParams,
    lane_depth: usize,
) -> Result<crate::RecallSupportReport> {
    validate_panel_recall_params(params, full)?;
    let held_out = panel_held_out(full, params.held_out_fraction, params.rng_seed);
    if held_out.is_empty() {
        return Err(LodestarError::RecallEmptyCorpus);
    }
    let mut members = BTreeSet::new();
    let mut candidate_hits = 0;
    for id in &held_out {
        let query = full.get(id).expect("held-out panel row");
        let hits = rank_panel_candidates(query, full, slots, PANEL_RRF_K)?;
        let mut query_support = hits
            .iter()
            .take(params.top_k)
            .map(|hit| hit.cx_id)
            .collect::<BTreeSet<_>>();
        if lane_depth > 0 {
            query_support.extend(
                hits.iter()
                    .filter(|hit| hit.lanes.iter().any(|lane| lane.rank <= lane_depth))
                    .map(|hit| hit.cx_id),
            );
        }
        candidate_hits += query_support.len();
        members.extend(query_support);
    }
    Ok(crate::RecallSupportReport {
        members: members.into_iter().collect(),
        n_queries_tested: held_out.len(),
        held_out,
        candidate_hits,
    })
}

fn validate_panel_recall_params(
    params: &crate::RecallTestParams,
    full: &BTreeMap<CxId, PanelVectors>,
) -> Result<()> {
    if full.is_empty() {
        return Err(LodestarError::RecallEmptyCorpus);
    }
    if !params.held_out_fraction.is_finite()
        || !(0.0..=1.0).contains(&params.held_out_fraction)
        || params.top_k == 0
        || !params.min_recall_ratio.is_finite()
        || !(0.0..=1.0).contains(&params.min_recall_ratio)
    {
        return Err(LodestarError::RecallInvalidParams {
            detail: "invalid panel recall parameters".to_string(),
        });
    }
    Ok(())
}

fn panel_held_out(full: &BTreeMap<CxId, PanelVectors>, fraction: f32, seed: u64) -> Vec<CxId> {
    let target = ((full.len() as f32) * fraction).ceil() as usize;
    let mut keyed = full
        .keys()
        .enumerate()
        .map(|(ordinal, id)| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&seed.to_be_bytes());
            hasher.update(&(ordinal as u64).to_be_bytes());
            hasher.update(id.as_bytes());
            (*hasher.finalize().as_bytes(), *id)
        })
        .collect::<Vec<_>>();
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    keyed
        .into_iter()
        .take(target.min(full.len()))
        .map(|(_, id)| id)
        .collect()
}

#[cfg(test)]
mod streaming_tests {
    //! Physical readback tests for the streamed panel-index write/load/verify
    //! path (issues #1863/#1864). Real f32 vectors on a real on-disk store — no
    //! mock data. Proves the streaming serialization is byte-identical to the
    //! previous owned-snapshot serialization, that the candidates double-store
    //! removal preserved search output exactly, and that corrupt bytes fail
    //! closed with the catalog codec code.
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn cx(seed: u8) -> CxId {
        CxId::from_bytes([seed; 16])
    }

    fn test_dir(case: &str) -> PathBuf {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-kernel-index-stream-{case}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Deterministic, distinct, finite, non-zero per-slot vectors.
    fn member_vectors(member: u8, slot_dims: &[(SlotId, usize)]) -> PanelVectors {
        slot_dims
            .iter()
            .map(|(slot, dim)| {
                let vector = (0..*dim)
                    .map(|k| {
                        0.5 + 0.1 * f32::from(member)
                            + 0.2 * f32::from(slot.get())
                            + 0.01 * (k as f32)
                    })
                    .collect::<Vec<f32>>();
                (*slot, vector)
            })
            .collect()
    }

    fn sample_index() -> (CxId, Vec<(SlotId, usize)>, PanelKernelIndex) {
        let kernel_id = cx(9);
        // Mixed per-lane dims to exercise the dims map.
        let slot_dims = vec![
            (SlotId::new(1), 3_usize),
            (SlotId::new(2), 4_usize),
            (SlotId::new(3), 2_usize),
        ];
        let slots = slot_dims.iter().map(|(slot, _)| *slot).collect::<Vec<_>>();
        let rows = (1..=5_u8)
            .map(|member| PanelKernelVectorRow {
                cx_id: cx(member),
                vectors: member_vectors(member, &slot_dims),
            })
            .collect::<Vec<_>>();
        let index = PanelKernelIndex::from_rows(kernel_id, slots, rows).unwrap();
        (kernel_id, slot_dims, index)
    }

    #[test]
    fn streamed_write_is_byte_identical_to_owned_snapshot_serialization() {
        let (kernel_id, _slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("byte-identity"));
        write_panel_kernel_index(&index, &store).unwrap();

        // The on-disk bytes must equal exactly what the previous non-streaming
        // `serde_json::to_vec_pretty(&PanelKernelIndexSnapshot { .. })` produced,
        // so artifact sha256 and the immutable contract are unchanged.
        let owned = PanelKernelIndexSnapshot {
            format_version: PANEL_FORMAT_VERSION,
            kernel_id,
            slots: index.slots.clone(),
            dims: index.dims.clone(),
            fusion: "rrf".to_string(),
            rrf_k: PANEL_RRF_K,
            rows: index.rows.clone(),
        };
        let expected = serde_json::to_vec_pretty(&owned).unwrap();
        let on_disk = fs::read(store.index_file_path(kernel_id)).unwrap();
        assert_eq!(
            on_disk, expected,
            "streamed bytes diverged from owned snapshot"
        );
    }

    #[test]
    fn load_roundtrips_and_search_matches_owned_ranker() {
        let (kernel_id, slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("roundtrip"));
        write_panel_kernel_index(&index, &store).unwrap();

        let loaded = load_panel_kernel_index(kernel_id, &store).unwrap();
        assert_eq!(loaded.rows().len(), index.rows().len());
        assert_eq!(loaded.slots(), index.slots());
        assert_eq!(loaded.dims(), index.dims());

        // The candidates double-store was removed; prove `panel_kernel_search`
        // (now over borrowed rows) yields the exact hits the owned ranker did.
        let query = member_vectors(3, &slot_dims);
        let owned_candidates = loaded
            .rows()
            .iter()
            .map(|row| (row.cx_id, row.vectors.clone()))
            .collect::<BTreeMap<_, _>>();
        let reference = {
            let mut hits =
                rank_panel_candidates(&query, &owned_candidates, loaded.slots(), PANEL_RRF_K)
                    .unwrap();
            hits.truncate(3);
            hits
        };
        let searched = panel_kernel_search(&loaded, &query, 3).unwrap();
        assert_eq!(searched, reference);
    }

    #[test]
    fn verify_counts_rows_and_returns_contract() {
        let (kernel_id, _slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("verify"));
        write_panel_kernel_index(&index, &store).unwrap();

        let readback = verify_panel_kernel_index(kernel_id, &store).unwrap();
        assert_eq!(readback.kernel_id, kernel_id);
        assert_eq!(readback.row_count, index.rows().len());
        assert_eq!(readback.slots, *index.slots());
        assert_eq!(readback.dims, *index.dims());
    }

    #[test]
    fn idempotent_rewrite_of_identical_bytes_succeeds() {
        let (kernel_id, _slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("idempotent"));
        write_panel_kernel_index(&index, &store).unwrap();
        let first = fs::read(store.index_file_path(kernel_id)).unwrap();
        // Re-installing the same immutable artifact must be a no-op, not an error.
        write_panel_kernel_index(&index, &store).unwrap();
        let second = fs::read(store.index_file_path(kernel_id)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn corrupt_index_bytes_fail_closed_on_load_and_verify() {
        let (kernel_id, _slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("corrupt"));
        write_panel_kernel_index(&index, &store).unwrap();

        // Truncate the on-disk artifact mid-structure (a real byte-level
        // corruption). Both the full load and the streamed verifier must refuse
        // with the codec catalog error rather than accept partial state.
        let path = store.index_file_path(kernel_id);
        let mut bytes = fs::read(&path).unwrap();
        bytes.truncate(bytes.len() / 2);
        fs::write(&path, &bytes).unwrap();

        let load_err = load_panel_kernel_index(kernel_id, &store).unwrap_err();
        assert!(
            matches!(load_err, LodestarError::KernelIndexCodec { .. }),
            "unexpected load error: {load_err:?}"
        );
        let verify_err = verify_panel_kernel_index(kernel_id, &store).unwrap_err();
        assert!(
            matches!(verify_err, LodestarError::KernelIndexCodec { .. }),
            "unexpected verify error: {verify_err:?}"
        );
    }

    #[test]
    fn trailing_garbage_after_index_fails_closed_on_verify() {
        let (kernel_id, _slot_dims, index) = sample_index();
        let store = FsKernelStore::new(test_dir("trailing"));
        write_panel_kernel_index(&index, &store).unwrap();
        let path = store.index_file_path(kernel_id);
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n{\"garbage\":1}");
        fs::write(&path, &bytes).unwrap();
        let verify_err = verify_panel_kernel_index(kernel_id, &store).unwrap_err();
        assert!(
            matches!(verify_err, LodestarError::KernelIndexCodec { .. }),
            "trailing bytes must fail closed: {verify_err:?}"
        );
    }

    #[test]
    fn duplicate_and_empty_rows_fail_closed() {
        let slots = vec![SlotId::new(1), SlotId::new(2)];
        let slot_dims = vec![(SlotId::new(1), 3_usize), (SlotId::new(2), 2_usize)];
        let dup = vec![
            PanelKernelVectorRow {
                cx_id: cx(1),
                vectors: member_vectors(1, &slot_dims),
            },
            PanelKernelVectorRow {
                cx_id: cx(1),
                vectors: member_vectors(2, &slot_dims),
            },
        ];
        let err = PanelKernelIndex::from_rows(cx(9), slots.clone(), dup).unwrap_err();
        assert!(matches!(err, LodestarError::KernelInvalidParams { .. }));

        let empty = PanelKernelIndex::from_rows(cx(9), slots, Vec::new()).unwrap_err();
        assert!(matches!(empty, LodestarError::KernelInvalidParams { .. }));
    }
}
