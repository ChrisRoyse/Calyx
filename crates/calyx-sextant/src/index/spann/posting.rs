//! SPANN posting-list blocks: varint deltas inside zstd-compressed files.

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
use crate::index::{IndexSearchHit, IndexStats, SextantIndex, ranked};

const ZSTD_LEVEL: i32 = 3;

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
    built_at_seq: u64,
    base_seq: u64,
}

impl PostingListWriter {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn append(&self, centroid_id: u32, cx_id: u32, score: f32) -> Result<()> {
        if !score.is_finite() {
            return Err(invalid("posting score is non-finite"));
        }
        let reader = PostingListReader::new(self.dir.clone());
        let mut entries = reader.read_list(centroid_id)?;
        if let Some((_, existing)) = entries.iter_mut().find(|(id, _)| *id == cx_id) {
            *existing = score;
        } else {
            entries.push((cx_id, score));
        }
        entries.sort_by_key(|(id, _)| *id);
        self.write_list(centroid_id, &entries)
    }

    pub fn write_list(&self, centroid_id: u32, entries: &[(u32, f32)]) -> Result<()> {
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

    pub fn read_list(&self, centroid_id: u32) -> Result<Vec<(u32, f32)>> {
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

    pub fn open(
        slot: SlotId,
        centroid_dir: impl AsRef<Path>,
        posting_dir: impl Into<PathBuf>,
    ) -> Result<Self> {
        let centroids = SpannCentroidIndex::open(centroid_dir)?;
        Ok(Self::new(slot, centroids, posting_dir))
    }

    pub fn search(&self, query: &[f32], k: usize, n_probe: usize) -> Result<Vec<(u32, f32)>> {
        if k == 0 || self.centroids.centroid_count() == 0 {
            return Ok(Vec::new());
        }
        validate_query(self.dim, query)?;
        let reader = PostingListReader::new(self.posting_dir.clone());
        let mut scores = BTreeMap::<u32, f32>::new();
        for centroid_id in self.centroids.nearest_centroids(query, n_probe) {
            for (cx_id, score) in reader.read_list(centroid_id)? {
                scores
                    .entry(cx_id)
                    .and_modify(|existing| *existing = existing.max(score))
                    .or_insert(score);
            }
        }
        let mut hits: Vec<_> = scores.into_iter().collect();
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
        let centroid_id = self.centroids.assign(&dense);
        let score = sparse_score(&vector);
        PostingListWriter::new(self.posting_dir.clone()).append(centroid_id, local, score)?;
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

pub fn encode_posting_block(entries: &[(u32, f32)]) -> Result<Vec<u8>> {
    let mut previous = 0_u32;
    let mut raw = Vec::with_capacity(4 + entries.len() * 8);
    raw.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (idx, (cx_id, score)) in entries.iter().enumerate() {
        if !score.is_finite() {
            return Err(invalid(format!("posting {idx} has non-finite score")));
        }
        if idx > 0 && *cx_id <= previous {
            return Err(invalid("posting cx_ids must be strictly ascending"));
        }
        write_varint(cx_id.saturating_sub(previous), &mut raw);
        raw.extend_from_slice(&score.to_le_bytes());
        previous = *cx_id;
    }
    Ok(raw)
}

pub fn decode_posting_block(raw: &[u8]) -> Result<Vec<(u32, f32)>> {
    if raw.len() < 4 {
        return Err(corrupt(format!("raw posting block is {} B", raw.len())));
    }
    let count = u32::from_le_bytes(raw[0..4].try_into().expect("4B")) as usize;
    let mut cursor = 4;
    let mut previous = 0_u32;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let delta = read_varint(raw, &mut cursor)?;
        let cx_id = previous
            .checked_add(delta)
            .ok_or_else(|| corrupt("posting cx_id delta overflow"))?;
        let score_bytes = raw
            .get(cursor..cursor + 4)
            .ok_or_else(|| corrupt("truncated posting score"))?;
        cursor += 4;
        let score = f32::from_le_bytes(score_bytes.try_into().expect("4B"));
        if !score.is_finite() {
            return Err(corrupt(format!("posting {cx_id} has non-finite score")));
        }
        entries.push((cx_id, score));
        previous = cx_id;
    }
    if cursor != raw.len() {
        return Err(corrupt(format!(
            "{} trailing posting bytes",
            raw.len() - cursor
        )));
    }
    Ok(entries)
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

fn sparse_score(vector: &SlotVector) -> f32 {
    match vector {
        SlotVector::Sparse { entries, .. } => entries.iter().map(|entry| entry.val).sum(),
        _ => 0.0,
    }
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

fn write_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(raw: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut value = 0_u32;
    let mut shift = 0;
    loop {
        let byte = *raw
            .get(*cursor)
            .ok_or_else(|| corrupt("truncated posting varint"))?;
        *cursor += 1;
        if shift == 28 && byte > 0x0f {
            return Err(corrupt("posting varint exceeds u32"));
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 28 {
            return Err(corrupt("posting varint exceeds u32"));
        }
    }
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
