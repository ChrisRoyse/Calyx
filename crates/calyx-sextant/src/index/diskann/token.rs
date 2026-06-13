//! Token DiskANN + segmented MaxSim rerank for server-scale multi slots.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};
use memmap2::Mmap;

use super::{DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams};
use crate::error::sextant_error;
use crate::error::{CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, CALYX_SEXTANT_VECTOR_SHAPE};
use crate::index::{IndexSearchHit, IndexStats, MaxSimIndex, SextantIndex, ranked};
use crate::util::top_k;

use super::token_sidecar::{
    DocSegment, docs_path, graph_path, map_tokens, read_docs, read_f32_vec, read_token_docs,
    token_cx, token_docs_path, tokens_path, write_docs, write_token_docs, write_tokens,
};

#[derive(Debug)]
pub struct TokenDiskAnnMaxSim {
    slot: SlotId,
    token_dim: u32,
    root: PathBuf,
    token_graph: DiskAnnSearch,
    docs: Vec<DocSegment>,
    positions: HashMap<CxId, usize>,
    token_docs: Vec<u32>,
    tokens: Mmap,
    default_search: DiskAnnSearchParams,
    candidate_tokens_per_query: usize,
    build_params: DiskAnnBuildParams,
    built_at_seq: u64,
    base_seq: u64,
}

impl TokenDiskAnnMaxSim {
    pub fn build(
        slot: SlotId,
        root: impl Into<PathBuf>,
        rows: &[(CxId, Vec<Vec<f32>>)],
        build_params: DiskAnnBuildParams,
        default_search: DiskAnnSearchParams,
    ) -> Result<Self> {
        validate_rows(rows, build_params.dim)?;
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| io("create token index dir", e))?;
        let mut flat = Vec::new();
        let mut docs = Vec::new();
        let mut token_docs = Vec::new();
        let mut next_token = 0_u32;
        for (doc_ordinal, (cx_id, tokens)) in rows.iter().enumerate() {
            let start = next_token;
            for token in tokens {
                flat.push((next_token, token.clone()));
                token_docs.push(u32::try_from(doc_ordinal).map_err(|_| invalid("too many docs"))?);
                next_token = next_token
                    .checked_add(1)
                    .ok_or_else(|| invalid("too many token vectors"))?;
            }
            docs.push(DocSegment {
                cx_id: *cx_id,
                start,
                len: u32::try_from(tokens.len()).map_err(|_| invalid("too many doc tokens"))?,
            });
        }
        let graph_path = graph_path(&root);
        super::build_diskann_graph(&graph_path, &flat, build_params)?;
        write_docs(
            &docs_path(&root),
            build_params.dim as u32,
            next_token,
            &docs,
        )?;
        write_token_docs(&token_docs_path(&root), &token_docs)?;
        write_tokens(&tokens_path(&root), rows)?;
        Self::open(slot, root, default_search, default_search.rescore_k.max(1))
    }

    pub fn open(
        slot: SlotId,
        root: impl Into<PathBuf>,
        default_search: DiskAnnSearchParams,
        candidate_tokens_per_query: usize,
    ) -> Result<Self> {
        let root = root.into();
        let (token_dim, token_count, docs) = read_docs(&docs_path(&root))?;
        let token_docs = read_token_docs(&token_docs_path(&root), token_count)?;
        let tokens = map_tokens(&tokens_path(&root), token_count, token_dim)?;
        let graph_path = graph_path(&root);
        let header = *super::open_diskann_graph(&graph_path)?.header();
        let token_graph = DiskAnnSearch::open(
            slot,
            graph_path,
            (0..token_count).map(token_cx).collect(),
            None,
            default_search,
        )?;
        let positions = docs
            .iter()
            .enumerate()
            .map(|(idx, doc)| (doc.cx_id, idx))
            .collect();
        Ok(Self {
            slot,
            token_dim,
            root,
            token_graph,
            docs,
            positions,
            token_docs,
            tokens,
            default_search,
            candidate_tokens_per_query: candidate_tokens_per_query.max(1),
            build_params: DiskAnnBuildParams {
                dim: header.dim as usize,
                m_max: header.m_max as usize,
                ef_construction: default_search.ef_search.max(header.m_max as usize),
                alpha: 1.2,
            },
            built_at_seq: 0,
            base_seq: 0,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn graph_path(&self) -> PathBuf {
        graph_path(&self.root)
    }

    pub fn search_tokens(
        &self,
        query: &[Vec<f32>],
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<(CxId, f32)>> {
        if k == 0 || self.docs.is_empty() {
            return Ok(Vec::new());
        }
        validate_query(query, self.token_dim as usize)?;
        let mut params = self.default_search;
        if let Some(ef) = ef {
            params.ef_search = ef;
        }
        let token_k = self.candidate_tokens_per_query.min(params.ef_search);
        let mut candidates = BTreeSet::new();
        for token in query {
            for (token_id, _) in self.token_graph.search_ids(token, token_k, &params)? {
                if let Some(&doc) = self.token_docs.get(token_id as usize) {
                    candidates.insert(doc);
                }
            }
        }
        let mut scored = Vec::with_capacity(candidates.len());
        for doc in candidates {
            let tokens = self.doc_tokens(doc as usize)?;
            scored.push((
                self.docs[doc as usize].cx_id,
                MaxSimIndex::maxsim(query, &tokens),
            ));
        }
        Ok(top_k(scored, k))
    }

    fn rows_from_disk(&self) -> Result<Vec<(CxId, Vec<Vec<f32>>)>> {
        (0..self.docs.len())
            .map(|idx| Ok((self.docs[idx].cx_id, self.doc_tokens(idx)?)))
            .collect()
    }

    fn doc_tokens(&self, doc_ordinal: usize) -> Result<Vec<Vec<f32>>> {
        let doc = self
            .docs
            .get(doc_ordinal)
            .ok_or_else(|| invalid(format!("doc ordinal {doc_ordinal} out of range")))?;
        let dim = self.token_dim as usize;
        let bytes_per = dim * 4;
        let start = doc.start as usize * bytes_per;
        let end = start + doc.len as usize * bytes_per;
        if end > self.tokens.len() {
            return Err(sextant_error(
                CALYX_INDEX_IO,
                format!("token sidecar segment {doc_ordinal} exceeds mapped length"),
            ));
        }
        Ok(self.tokens[start..end]
            .chunks_exact(bytes_per)
            .map(read_f32_vec)
            .collect())
    }
}

impl SextantIndex for TokenDiskAnnMaxSim {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Multi {
            token_dim: self.token_dim,
        }
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let SlotVector::Multi { token_dim, tokens } = vector else {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                "token DiskANN received non-multi vector",
            ));
        };
        if token_dim != self.token_dim {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                format!("token dim {token_dim} expected {}", self.token_dim),
            ));
        }
        validate_query(&tokens, self.token_dim as usize)?;
        let mut rows = self.rows_from_disk()?;
        if let Some(&pos) = self.positions.get(&cx_id) {
            rows[pos] = (cx_id, tokens);
        } else {
            rows.push((cx_id, tokens));
        }
        *self = Self::build(
            self.slot,
            self.root.clone(),
            &rows,
            self.build_params,
            self.default_search,
        )?;
        self.built_at_seq = seq;
        self.base_seq = self.base_seq.max(seq);
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let SlotVector::Multi { token_dim, tokens } = query else {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                "multi query required",
            ));
        };
        if *token_dim != self.token_dim {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                "token dim mismatch",
            ));
        }
        Ok(ranked(self.search_tokens(tokens, k, ef)?))
    }

    fn rebuild(&mut self) -> Result<()> {
        let rows = self.rows_from_disk()?;
        *self = Self::build(
            self.slot,
            self.root.clone(),
            &rows,
            self.build_params,
            self.default_search,
        )?;
        self.built_at_seq = self.base_seq;
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        let pos = *self.positions.get(&cx_id)?;
        self.doc_tokens(pos).ok().map(|tokens| SlotVector::Multi {
            token_dim: self.token_dim,
            tokens,
        })
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.base_seq = seq;
    }

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: self.shape(),
            len: self.docs.len(),
            built_at_seq: self.built_at_seq,
            base_seq: self.base_seq,
            kind: "token_diskann_maxsim",
        }
    }
}

fn validate_rows(rows: &[(CxId, Vec<Vec<f32>>)], dim: usize) -> Result<()> {
    if rows.is_empty() {
        return Err(invalid(
            "empty input: at least one multi-vector row is required",
        ));
    }
    for (doc, (_, tokens)) in rows.iter().enumerate() {
        if tokens.is_empty() {
            return Err(invalid(format!("doc {doc} has no tokens")));
        }
        validate_query(tokens, dim)?;
    }
    Ok(())
}

fn validate_query(tokens: &[Vec<f32>], dim: usize) -> Result<()> {
    if tokens.is_empty() {
        return Err(sextant_error(
            CALYX_SEXTANT_VECTOR_SHAPE,
            "multi query has no tokens",
        ));
    }
    for (idx, token) in tokens.iter().enumerate() {
        if token.len() != dim {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                format!("token {idx} dim {} expected {dim}", token.len()),
            ));
        }
        if token.iter().any(|v| !v.is_finite()) {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                format!("token {idx} has non-finite component"),
            ));
        }
    }
    Ok(())
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("token diskann invalid params: {detail}"),
    )
}

fn io(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("token diskann {stage}: {error}"))
}
