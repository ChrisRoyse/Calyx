//! kNN resolved-neighbor edge materialization into Graph CF (#72).

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Seq};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::exact_knn::{
    EXACT_KNN_MAX_DEVICE_K, EXACT_KNN_RERANK_GUARD_ROWS, ExactKnnExecution, exact_cosine_top_k,
};
use crate::knn_base_rate::ResolvedExemplar;

pub const KNN_GRAPH_SCHEMA_VERSION: &str = "poly.knn_graph_edges.v1";
pub const EDGE_KNN_RESOLVED: &str = "association.knn_resolved";

pub const ERR_KNN_GRAPH_EMPTY_CORPUS: &str = "CALYX_POLY_KNN_GRAPH_EMPTY_CORPUS";
pub const ERR_KNN_GRAPH_INVALID_K: &str = "CALYX_POLY_KNN_GRAPH_INVALID_K";
pub const ERR_KNN_GRAPH_DIM_MISMATCH: &str = "CALYX_POLY_KNN_GRAPH_DIM_MISMATCH";
pub const ERR_KNN_GRAPH_NON_FINITE: &str = "CALYX_POLY_KNN_GRAPH_NON_FINITE";
pub const ERR_KNN_GRAPH_DUPLICATE_CX: &str = "CALYX_POLY_KNN_GRAPH_DUPLICATE_CX";
pub const ERR_KNN_GRAPH_READBACK_MISMATCH: &str = "CALYX_POLY_KNN_GRAPH_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnGraphEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub rank: usize,
    pub similarity: f64,
    pub weight: f64,
    pub domain: String,
    pub k: usize,
    pub corpus_len: usize,
    pub query_outcome_yes: bool,
    pub neighbor_outcome_yes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnGraphEdgeValue {
    pub schema_version: String,
    pub edge_type: String,
    pub source: String,
    pub domain: String,
    pub rank: usize,
    pub similarity: f64,
    pub weight: f64,
    pub k: usize,
    pub corpus_len: usize,
    pub query_outcome_yes: bool,
    pub neighbor_outcome_yes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnGraphReadback {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: KnnGraphEdgeValue,
    pub value_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnGraphRun {
    pub schema_version: String,
    pub collection: String,
    pub domain: String,
    pub ingested_cx_id: CxId,
    pub k: usize,
    pub corpus_len: usize,
    pub edge_count: usize,
    pub snapshot_seq: Seq,
    pub graph_cf_row_count: usize,
    pub edges: Vec<KnnGraphEdge>,
    pub readback_edges: Vec<KnnGraphReadback>,
}

pub fn persist_knn_edges_on_ingest<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    domain: &str,
    ingested: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<KnnGraphRun> {
    persist_knn_edges_on_ingest_with_execution(vault, collection, domain, ingested, corpus, k)
        .map(|result| result.0)
}

pub fn persist_knn_edges_on_ingest_with_execution<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    domain: &str,
    ingested: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<(KnnGraphRun, ExactKnnExecution)> {
    let (hits, execution) = compute_knn_edges_with_execution(domain, ingested, corpus, k)?;
    let graph = PlainGraph::new(vault, collection)?;
    let corpus_by_cx: BTreeMap<CxId, &ResolvedExemplar> =
        corpus.iter().map(|row| (row.cx_id, row)).collect();
    graph.put_node(
        ingested.cx_id,
        &node_value(domain, "ingested_resolved", ingested)?,
    )?;
    for edge in &hits {
        let neighbor = corpus_by_cx
            .get(&edge.dst)
            .copied()
            .expect("edge dst came from corpus");
        graph.put_node(
            edge.dst,
            &node_value(domain, "resolved_neighbor", neighbor)?,
        )?;
    }
    for edge in &hits {
        graph.put_edge(edge.src, &edge.edge_type, edge.dst, &edge_bytes(edge)?)?;
    }
    let snapshot_seq = vault.latest_seq();
    let mut readback_edges = Vec::with_capacity(hits.len());
    for edge in &hits {
        let bytes = graph
            .get_edge(snapshot_seq, edge.src, &edge.edge_type, edge.dst)?
            .ok_or_else(|| {
                readback_error(format!("missing Graph CF kNN edge {}", edge_id(edge)))
            })?;
        let expected = edge_bytes(edge)?;
        if bytes != expected {
            return Err(readback_error(format!(
                "Graph CF kNN edge {} bytes mismatch expected_blake3={} actual_blake3={}",
                edge_id(edge),
                blake3::hash(&expected).to_hex(),
                blake3::hash(&bytes).to_hex()
            )));
        }
        let value: KnnGraphEdgeValue =
            serde_json::from_slice(&bytes).map_err(|err| readback_error(err.to_string()))?;
        readback_edges.push(KnnGraphReadback {
            src: edge.src,
            dst: edge.dst,
            edge_type: edge.edge_type.clone(),
            value,
            value_blake3: blake3::hash(&bytes).to_hex().to_string(),
        });
    }
    Ok((
        KnnGraphRun {
            schema_version: KNN_GRAPH_SCHEMA_VERSION.to_string(),
            collection: collection.to_string(),
            domain: domain.to_string(),
            ingested_cx_id: ingested.cx_id,
            k,
            corpus_len: corpus.len(),
            edge_count: hits.len(),
            snapshot_seq,
            graph_cf_row_count: vault.scan_cf_at(snapshot_seq, ColumnFamily::Graph)?.len(),
            edges: hits,
            readback_edges,
        },
        execution,
    ))
}

pub fn compute_knn_edges(
    domain: &str,
    ingested: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<Vec<KnnGraphEdge>> {
    compute_knn_edges_with_execution(domain, ingested, corpus, k).map(|result| result.0)
}

pub fn compute_knn_edges_with_execution(
    domain: &str,
    ingested: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<(Vec<KnnGraphEdge>, ExactKnnExecution)> {
    let (mut batches, execution) =
        compute_knn_edges_batch_with_execution(domain, std::slice::from_ref(ingested), corpus, k)?;
    Ok((batches.remove(0), execution))
}

pub fn compute_knn_edges_batch(
    domain: &str,
    ingested: &[ResolvedExemplar],
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<Vec<Vec<KnnGraphEdge>>> {
    compute_knn_edges_batch_with_execution(domain, ingested, corpus, k).map(|result| result.0)
}

pub fn compute_knn_edges_batch_with_execution(
    domain: &str,
    ingested: &[ResolvedExemplar],
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<(Vec<Vec<KnnGraphEdge>>, ExactKnnExecution)> {
    validate_batch_request(domain, ingested, corpus, k)?;
    let corpus_by_cx: BTreeMap<CxId, &ResolvedExemplar> =
        corpus.iter().map(|row| (row.cx_id, row)).collect();
    let ordered = corpus_by_cx.values().copied().collect::<Vec<_>>();
    let corpus_vectors = ordered
        .iter()
        .map(|row| row.vector.as_slice())
        .collect::<Vec<_>>();
    let query_vectors = ingested
        .iter()
        .map(|row| row.vector.as_slice())
        .collect::<Vec<_>>();
    let candidate_k = if k > EXACT_KNN_MAX_DEVICE_K {
        k
    } else {
        k.saturating_add(EXACT_KNN_RERANK_GUARD_ROWS)
            .min(corpus.len())
            .min(EXACT_KNN_MAX_DEVICE_K)
    };
    let mut exact = exact_cosine_top_k(&corpus_vectors, &query_vectors, candidate_k, None)?;
    exact.execution.shortlist_cpu_similarity_evaluations = exact
        .rankings
        .iter()
        .map(|ranking| ranking.len() as u64)
        .sum();
    let batches = ingested
        .iter()
        .zip(&exact.rankings)
        .map(|(query, ranking)| {
            let mut hits = ranking
                .iter()
                .map(|idx| {
                    let row = ordered[*idx];
                    (row.cx_id, cosine(&query.vector, &row.vector))
                })
                .collect::<Vec<_>>();
            hits.sort_by(|left, right| {
                right
                    .1
                    .total_cmp(&left.1)
                    .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
            });
            hits.truncate(k);
            hits.into_iter()
                .enumerate()
                .map(|(idx, (dst, raw_similarity))| {
                    let neighbor = corpus_by_cx
                        .get(&dst)
                        .copied()
                        .expect("kNN hit came from corpus");
                    let similarity = canonical_score(raw_similarity as f64);
                    KnnGraphEdge {
                        src: query.cx_id,
                        dst,
                        edge_type: EDGE_KNN_RESOLVED.to_string(),
                        rank: idx + 1,
                        similarity,
                        weight: similarity.clamp(0.0, 1.0),
                        domain: domain.to_string(),
                        k,
                        corpus_len: corpus.len(),
                        query_outcome_yes: query.outcome_yes,
                        neighbor_outcome_yes: neighbor.outcome_yes,
                    }
                })
                .collect()
        })
        .collect();
    Ok((batches, exact.execution))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut an = 0.0;
    let mut bn = 0.0;
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

fn validate_batch_request(
    domain: &str,
    ingested: &[ResolvedExemplar],
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Result<()> {
    if domain.trim().is_empty() {
        return Err(invalid(
            ERR_KNN_GRAPH_DIM_MISMATCH,
            "kNN graph domain must not be empty",
        ));
    }
    if corpus.is_empty() {
        return Err(invalid(
            ERR_KNN_GRAPH_EMPTY_CORPUS,
            "kNN graph materialization requires a non-empty resolved corpus",
        ));
    }
    if k == 0 || k > corpus.len() {
        return Err(invalid(
            ERR_KNN_GRAPH_INVALID_K,
            format!("kNN graph k={k} must be in 1..={}", corpus.len()),
        ));
    }
    if ingested.is_empty() {
        return Err(invalid(
            ERR_KNN_GRAPH_EMPTY_CORPUS,
            "kNN graph materialization requires at least one ingested query",
        ));
    }
    let dim = ingested[0].vector.len();
    if dim == 0 {
        return Err(invalid(
            ERR_KNN_GRAPH_DIM_MISMATCH,
            "ingested vector must not be empty",
        ));
    }
    let mut seen = BTreeSet::new();
    for query in ingested {
        if !seen.insert(query.cx_id) {
            return Err(invalid(
                ERR_KNN_GRAPH_DUPLICATE_CX,
                format!("ingested batch repeats cx_id {}", query.cx_id),
            ));
        }
        if query.vector.len() != dim {
            return Err(invalid(
                ERR_KNN_GRAPH_DIM_MISMATCH,
                format!(
                    "ingested query {} dim {} != {dim}",
                    query.cx_id,
                    query.vector.len()
                ),
            ));
        }
        validate_vector(ERR_KNN_GRAPH_NON_FINITE, query.cx_id, &query.vector)?;
    }
    for row in corpus {
        if !seen.insert(row.cx_id) {
            return Err(invalid(
                ERR_KNN_GRAPH_DUPLICATE_CX,
                format!(
                    "resolved corpus repeats cx_id {} or includes the ingested row",
                    row.cx_id
                ),
            ));
        }
        if row.vector.len() != dim {
            return Err(invalid(
                ERR_KNN_GRAPH_DIM_MISMATCH,
                format!("exemplar {} dim {} != {dim}", row.cx_id, row.vector.len()),
            ));
        }
        validate_vector(ERR_KNN_GRAPH_NON_FINITE, row.cx_id, &row.vector)?;
    }
    Ok(())
}

fn validate_vector(code: &'static str, cx_id: CxId, vector: &[f32]) -> Result<()> {
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            code,
            format!("vector for {cx_id} contains a non-finite value"),
        ));
    }
    Ok(())
}

fn edge_bytes(edge: &KnnGraphEdge) -> Result<Vec<u8>> {
    serde_json::to_vec(&edge_value(edge)).map_err(|err| {
        PolyError::diagnostics(
            ERR_KNN_GRAPH_READBACK_MISMATCH,
            format!("encode kNN graph edge: {err}"),
        )
    })
}

fn edge_value(edge: &KnnGraphEdge) -> KnnGraphEdgeValue {
    KnnGraphEdgeValue {
        schema_version: KNN_GRAPH_SCHEMA_VERSION.to_string(),
        edge_type: edge.edge_type.clone(),
        source: "calyx_poly::direct_cosine_top_k".to_string(),
        domain: edge.domain.clone(),
        rank: edge.rank,
        similarity: edge.similarity,
        weight: edge.weight,
        k: edge.k,
        corpus_len: edge.corpus_len,
        query_outcome_yes: edge.query_outcome_yes,
        neighbor_outcome_yes: edge.neighbor_outcome_yes,
    }
}

fn node_value(domain: &str, role: &str, row: &ResolvedExemplar) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": KNN_GRAPH_SCHEMA_VERSION,
        "domain": domain,
        "role": role,
        "cx_id": row.cx_id,
        "outcome_yes": row.outcome_yes,
        "vector_blake3": vector_hash(&row.vector),
    }))
    .map_err(|err| invalid(ERR_KNN_GRAPH_READBACK_MISMATCH, err.to_string()))
}

fn vector_hash(vector: &[f32]) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in vector {
        hasher.update(&value.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn canonical_score(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn edge_id(edge: &KnnGraphEdge) -> String {
    format!("{} -{}-> {}", edge.src, edge.edge_type, edge.dst)
}

fn invalid(code: &'static str, message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(code, message)
}

fn readback_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_KNN_GRAPH_READBACK_MISMATCH, message)
}
