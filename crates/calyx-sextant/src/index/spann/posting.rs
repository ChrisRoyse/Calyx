//! SPANN posting-list blocks: varint deltas inside zstd-compressed files.

mod codec;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::centroids::SpannCentroidIndex;
use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO,
    CALYX_SEXTANT_INDEX_EMPTY, CALYX_SEXTANT_VECTOR_SHAPE, sextant_error,
};
use crate::index::distance::l2_sq;
use crate::index::{IndexSearchHit, IndexStats, SextantIndex, ranked};
pub use codec::{decode_posting_block, encode_posting_block};

const ZSTD_LEVEL: i32 = 3;
const DEFAULT_BOUNDARY_EPSILON: f32 = 0.10;
const DEFAULT_MAX_REPLICATION: usize = 2;

/// One member of a SPANN posting list: its local id plus the **sparse vector**
/// (idx,val pairs) of the cx it points at.
///
/// #701 root cause: posting members used to carry a single static scalar (the sum
/// of the vector's entries), so search ranked members by a query-independent
/// number and returned the same order for every query. The faithful SPANN design
/// (Microsoft SPANN, NeurIPS'21) stores the member vectors in the posting list so
/// search can compute the *true* query-to-member distance and rank by it. We keep
/// the SPARSE form (idx,val) so memory/disk stay bounded by nnz, not dim.
#[derive(Clone, Debug, PartialEq)]
pub struct PostingMember {
    pub cx_id: u32,
    pub vector: Vec<(u32, f32)>,
}

impl PostingMember {
    pub fn new(cx_id: u32, vector: Vec<(u32, f32)>) -> Self {
        Self { cx_id, vector }
    }
}

#[derive(Clone, Debug)]
pub struct PostingListWriter {
    dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct PostingListReader {
    dir: PathBuf,
}

#[derive(Debug)]
pub struct SpannSearch {
    slot: SlotId,
    dim: u32,
    centroids: SpannCentroidIndex,
    posting_dir: PathBuf,
    local_to_cx: Vec<CxId>,
    cx_to_local: BTreeMap<CxId, u32>,
    vectors: BTreeMap<CxId, SlotVector>,
    default_n_probe: usize,
    boundary_epsilon: f32,
    max_replication: usize,
    built_at_seq: u64,
    base_seq: u64,
}

impl PostingListWriter {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn append(&self, centroid_id: u32, cx_id: u32, vector: &[(u32, f32)]) -> Result<()> {
        validate_sparse_vector(vector)?;
        let reader = PostingListReader::new(self.dir.clone());
        let mut entries = reader.read_list(centroid_id)?;
        if let Some(existing) = entries.iter_mut().find(|m| m.cx_id == cx_id) {
            existing.vector = vector.to_vec();
        } else {
            entries.push(PostingMember::new(cx_id, vector.to_vec()));
        }
        entries.sort_by_key(|m| m.cx_id);
        self.write_list(centroid_id, &entries)
    }

    pub fn write_list(&self, centroid_id: u32, entries: &[PostingMember]) -> Result<()> {
        fs::create_dir_all(&self.dir).map_err(|e| io("create posting dir", e))?;
        let raw = encode_posting_block(entries)?;
        let compressed = zstd::stream::encode_all(Cursor::new(raw), ZSTD_LEVEL).map_err(|e| {
            io(
                "compress posting block",
                std::io::Error::new(std::io::ErrorKind::InvalidData, e),
            )
        })?;
        let path = posting_path(&self.dir, centroid_id);
        let tmp = tmp_path(&path);
        let mut file = File::create(&tmp).map_err(|e| io("create posting tmp", e))?;
        file.write_all(&compressed)
            .map_err(|e| io("write posting tmp", e))?;
        file.sync_all().map_err(|e| io("fsync posting tmp", e))?;
        drop(file);
        fs::rename(&tmp, &path).map_err(|e| io("publish posting block", e))
    }
}

impl PostingListReader {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn read_list(&self, centroid_id: u32) -> Result<Vec<PostingMember>> {
        let path = posting_path(&self.dir, centroid_id);
        let compressed = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io("read posting block", error)),
        };
        let raw = zstd::stream::decode_all(Cursor::new(compressed))
            .map_err(|error| corrupt(format!("zstd decode for centroid {centroid_id}: {error}")))?;
        decode_posting_block(&raw)
    }
}

impl SpannSearch {
    pub fn new(
        slot: SlotId,
        centroids: SpannCentroidIndex,
        posting_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            slot,
            dim: centroids.dim(),
            centroids,
            posting_dir: posting_dir.into(),
            local_to_cx: Vec::new(),
            cx_to_local: BTreeMap::new(),
            vectors: BTreeMap::new(),
            default_n_probe: 8,
            boundary_epsilon: DEFAULT_BOUNDARY_EPSILON,
            max_replication: DEFAULT_MAX_REPLICATION,
            built_at_seq: 0,
            base_seq: 0,
        }
    }

    pub fn with_cx_map(mut self, local_to_cx: Vec<CxId>) -> Self {
        self.cx_to_local = local_to_cx
            .iter()
            .enumerate()
            .filter_map(|(idx, cx)| u32::try_from(idx).ok().map(|id| (*cx, id)))
            .collect();
        self.local_to_cx = local_to_cx;
        self
    }

    pub fn with_default_n_probe(mut self, n_probe: usize) -> Self {
        self.default_n_probe = n_probe.max(1);
        self
    }

    pub fn with_boundary_duplication(mut self, epsilon: f32, max_replication: usize) -> Self {
        self.boundary_epsilon = epsilon.max(0.0);
        self.max_replication = max_replication.max(1);
        self
    }

    pub fn open(
        slot: SlotId,
        centroid_dir: impl AsRef<Path>,
        posting_dir: impl Into<PathBuf>,
    ) -> Result<Self> {
        let centroids = SpannCentroidIndex::open(centroid_dir)?;
        Ok(Self::new(slot, centroids, posting_dir))
    }

    /// Region-restricted SPANN search. Probes the `n_probe` nearest centroids,
    /// loads their posting lists (each member carries its sparse vector), and ranks
    /// candidates by their **true distance to the query** — the #701 fix. The
    /// returned f32 is a similarity (higher = closer), consistent with the DiskANN
    /// path's `1.0 - dist`, so the KernelFirst funnel can rank either uniformly.
    pub fn search(&self, query: &[f32], k: usize, n_probe: usize) -> Result<Vec<(u32, f32)>> {
        if k == 0 || self.centroids.centroid_count() == 0 {
            return Ok(Vec::new());
        }
        validate_query(self.dim, query)?;
        // ||q||^2 is constant across all members, so ranking by L2^2 =
        // ||q||^2 - 2*<q,m> + ||m||^2 is order-equivalent to ranking by the
        // similarity below; we still return the real L2 so scores are meaningful.
        let query_norm_sq: f32 = query.iter().map(|v| v * v).sum();
        let reader = PostingListReader::new(self.posting_dir.clone());
        // Keep the best (smallest-distance) sighting of each cx across probed lists.
        let mut best = BTreeMap::<u32, f32>::new();
        for centroid_id in self.centroids.nearest_centroids(query, n_probe) {
            for member in reader.read_list(centroid_id)? {
                let mut dot = 0.0_f32;
                let mut member_norm_sq = 0.0_f32;
                for (idx, val) in &member.vector {
                    let qi = *query.get(*idx as usize).ok_or_else(|| {
                        corrupt(format!(
                            "posting member dim index {idx} >= query dim {}",
                            query.len()
                        ))
                    })?;
                    dot += qi * val;
                    member_norm_sq += val * val;
                }
                let l2_sq = (query_norm_sq - 2.0 * dot + member_norm_sq).max(0.0);
                let similarity = -l2_sq; // higher = closer
                best.entry(member.cx_id)
                    .and_modify(|existing| {
                        if similarity > *existing {
                            *existing = similarity;
                        }
                    })
                    .or_insert(similarity);
            }
        }
        let mut hits: Vec<_> = best.into_iter().collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(k);
        Ok(hits)
    }

    pub fn posting_dir(&self) -> &Path {
        &self.posting_dir
    }

    pub fn centroids(&self) -> &SpannCentroidIndex {
        &self.centroids
    }

    fn local_id_for(&mut self, cx_id: CxId) -> Result<u32> {
        if let Some(id) = self.cx_to_local.get(&cx_id) {
            return Ok(*id);
        }
        let id = u32::try_from(self.local_to_cx.len())
            .map_err(|_| invalid("SPANN local id space exceeds u32"))?;
        self.local_to_cx.push(cx_id);
        self.cx_to_local.insert(cx_id, id);
        Ok(id)
    }

    fn cx_for_local(&self, local: u32) -> CxId {
        self.local_to_cx
            .get(local as usize)
            .copied()
            .unwrap_or_else(|| cx_from_u32(local))
    }
}

impl SextantIndex for SpannSearch {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Sparse(self.dim)
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        if self.centroids.centroid_count() == 0 {
            return Err(sextant_error(
                CALYX_SEXTANT_INDEX_EMPTY,
                "spann insert requires at least one centroid",
            ));
        }
        let dense = dense_sparse(self.dim, &vector)?;
        let local = self.local_id_for(cx_id)?;
        let sparse = sparse_pairs(&vector)?;
        let writer = PostingListWriter::new(self.posting_dir.clone());
        for centroid_id in boundary_centroids(
            &self.centroids,
            &dense,
            self.boundary_epsilon,
            self.max_replication,
        ) {
            writer.append(centroid_id, local, &sparse)?;
        }
        self.vectors.insert(cx_id, vector);
        self.built_at_seq = self.built_at_seq.max(seq);
        self.base_seq = self.base_seq.max(seq);
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let dense = dense_sparse(self.dim, query)?;
        let n_probe = ef.unwrap_or(self.default_n_probe);
        let hits = SpannSearch::search(self, &dense, k, n_probe)?
            .into_iter()
            .map(|(local, score)| (self.cx_for_local(local), score))
            .collect();
        Ok(ranked(hits))
    }

    fn rebuild(&mut self) -> Result<()> {
        let writer = PostingListWriter::new(self.posting_dir.clone());
        for centroid_id in 0..self.centroids.centroid_count() as u32 {
            writer.write_list(centroid_id, &[])?;
        }
        let rows: Vec<_> = self
            .vectors
            .iter()
            .map(|(cx, v)| (*cx, v.clone()))
            .collect();
        for (idx, (cx_id, vector)) in rows.into_iter().enumerate() {
            self.insert(cx_id, vector, idx as u64)?;
        }
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        self.vectors.get(&cx_id).cloned()
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.base_seq = seq;
    }

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: self.shape(),
            len: self.local_to_cx.len(),
            built_at_seq: self.built_at_seq,
            base_seq: self.base_seq,
            kind: "SPANN",
        }
    }
}

fn dense_sparse(dim: u32, vector: &SlotVector) -> Result<Vec<f32>> {
    let SlotVector::Sparse { dim: vdim, entries } = vector else {
        return Err(sextant_error(
            CALYX_SEXTANT_VECTOR_SHAPE,
            "spann requires sparse vectors",
        ));
    };
    if *vdim != dim {
        return Err(sextant_error(
            CALYX_INDEX_DIM_MISMATCH,
            format!("sparse dim {vdim} expected {dim}"),
        ));
    }
    let mut dense = vec![0.0_f32; dim as usize];
    for entry in entries {
        if entry.idx >= dim || !entry.val.is_finite() {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                "sparse entry outside dim or non-finite",
            ));
        }
        dense[entry.idx as usize] = entry.val;
    }
    Ok(dense)
}

/// Extract the (idx,val) pairs of a sparse slot vector for storage in a posting.
fn sparse_pairs(vector: &SlotVector) -> Result<Vec<(u32, f32)>> {
    match vector {
        SlotVector::Sparse { entries, .. } => Ok(entries.iter().map(|e| (e.idx, e.val)).collect()),
        _ => Err(sextant_error(
            CALYX_SEXTANT_VECTOR_SHAPE,
            "spann requires sparse vectors",
        )),
    }
}

/// A posting member's stored vector must have finite values (so distances stay
/// well-defined). Indices are bounds-checked at search time against the query dim.
fn validate_sparse_vector(vector: &[(u32, f32)]) -> Result<()> {
    if vector.iter().any(|(_, val)| !val.is_finite()) {
        return Err(invalid("posting member vector has non-finite value"));
    }
    Ok(())
}

fn validate_query(dim: u32, query: &[f32]) -> Result<()> {
    if query.len() != dim as usize {
        return Err(sextant_error(
            CALYX_INDEX_DIM_MISMATCH,
            format!("query dim {} expected {dim}", query.len()),
        ));
    }
    if query.iter().any(|value| !value.is_finite()) {
        return Err(invalid("query has non-finite component"));
    }
    Ok(())
}

fn boundary_centroids(
    centroids: &SpannCentroidIndex,
    vector: &[f32],
    epsilon: f32,
    max_replication: usize,
) -> Vec<u32> {
    if centroids.centroid_count() == 0 || vector.len() != centroids.dim() as usize {
        return Vec::new();
    }
    let mut scored: Vec<(u32, f32)> = centroids
        .centroids()
        .iter()
        .enumerate()
        .map(|(idx, centroid)| (idx as u32, l2_sq(centroid, vector)))
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let nearest = scored[0].1;
    let threshold = nearest * (1.0 + epsilon.max(0.0));
    let mut selected = scored
        .iter()
        .copied()
        .filter(|(_, distance)| *distance <= threshold)
        .take(max_replication.max(1))
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        selected.push(scored[0].0);
    }
    selected
}

fn posting_path(dir: &Path, centroid_id: u32) -> PathBuf {
    dir.join(format!("pl_{centroid_id:04}.spb"))
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

fn cx_from_u32(id: u32) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[0..8].copy_from_slice(b"CLXSPANN");
    bytes[12..16].copy_from_slice(&id.to_be_bytes());
    CxId::from_bytes(bytes)
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("spann postings: {detail}"),
    )
}

fn corrupt(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_CORRUPT,
        format!("spann posting block corrupt: {detail}"),
    )
}

fn io(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("spann postings {stage}: {error}"))
}
