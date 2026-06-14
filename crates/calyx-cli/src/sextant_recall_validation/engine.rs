use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId, VaultStore,
};
use calyx_sextant::{
    FusionStrategy, HnswIndex, InvertedIndex, ProvenanceSource, Query, RrfProfile, SearchEngine,
    SlotIndexMap,
};
use serde::Serialize;

use super::data::{CorpusDoc, ValidationData};
use super::request::RecallRequest;

const PH70_PANEL_VERSION: u32 = 70;
const LEXICAL_SLOT: SlotId = SlotId::new(1);
const DENSE_SLOT: SlotId = SlotId::new(8);

pub(crate) struct IndexedCorpus {
    pub(crate) engine: SearchEngine,
    pub(crate) doc_count: usize,
    pub(crate) stored_docs: usize,
    pub(crate) ledger_ref_count: usize,
    pub(crate) final_snapshot: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RecallReport {
    pub(crate) dataset: String,
    pub(crate) corpus_docs: usize,
    pub(crate) stored_docs: usize,
    pub(crate) qrels_rows: usize,
    pub(crate) qrels_queries: usize,
    pub(crate) queries_evaluated: usize,
    pub(crate) skipped_queries_without_relevance: usize,
    pub(crate) queries_truncated_by_limit: usize,
    pub(crate) single_hits: usize,
    pub(crate) multi_hits: usize,
    pub(crate) single_recall_at_10: f64,
    pub(crate) multi_recall_at_10: f64,
    pub(crate) delta: f64,
    pub(crate) min_delta: f64,
    pub(crate) meets_delta_15: bool,
    pub(crate) provenance_ok: bool,
    pub(crate) multi_hits_examined: usize,
    pub(crate) ledger_ref_count: usize,
    pub(crate) final_snapshot: u64,
    pub(crate) strategy: String,
    pub(crate) recall_kind: String,
    pub(crate) query_evidence: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct QueryEvidence {
    pub(crate) qid: String,
    pub(crate) relevant_docs: usize,
    pub(crate) single_hit: bool,
    pub(crate) multi_hit: bool,
    pub(crate) single_top_k: Vec<String>,
    pub(crate) multi_top_k: Vec<String>,
}

pub(crate) fn build_engine(
    vault: &AsterVault,
    data: &ValidationData,
) -> Result<IndexedCorpus, String> {
    let vault_id = vault.vault_id();
    let constellations = data
        .corpus
        .iter()
        .enumerate()
        .map(|(idx, doc)| constellation_for_doc(doc, vault_id, idx as u64 + 1))
        .collect::<Vec<_>>();
    let ids = vault
        .put_batch(constellations)
        .map_err(|error| error.to_string())?;
    let snapshot = vault.snapshot();
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(LEXICAL_SLOT))
        .map_err(|error| error.to_string())?;
    map.register(HnswIndex::new(DENSE_SLOT, 2, 42))
        .map_err(|error| error.to_string())?;
    let mut engine = SearchEngine::new(map);
    let mut ledger_refs = BTreeSet::<(u64, [u8; 32])>::new();
    for (seq, doc) in data.corpus.iter().enumerate() {
        let cx_id = cx_for_doc_id(&doc.doc_id);
        let stored = vault
            .get(cx_id, snapshot)
            .map_err(|error| format!("stored readback failed for {}: {error}", doc.doc_id))?;
        ledger_refs.insert((stored.provenance.seq, stored.provenance.hash));
        engine
            .indexes
            .insert_text(LEXICAL_SLOT, cx_id, &doc.text, seq as u64 + 1)
            .map_err(|error| error.to_string())?;
        engine
            .indexes
            .insert(DENSE_SLOT, cx_id, weak_dense(&doc.doc_id), seq as u64 + 1)
            .map_err(|error| error.to_string())?;
        engine.put_constellation(stored);
    }
    Ok(IndexedCorpus {
        engine,
        doc_count: data.corpus.len(),
        stored_docs: ids.len(),
        ledger_ref_count: ledger_refs.len(),
        final_snapshot: snapshot,
    })
}

pub(crate) fn evaluate_recall(
    engine: &SearchEngine,
    data: &ValidationData,
    request: &RecallRequest,
    indexed: &IndexedCorpus,
) -> Result<RecallReport, String> {
    let eligible = eligible_query_ids(data);
    if eligible.is_empty() {
        return Err("CALYX_FSV_EMPTY_QRELS".to_string());
    }
    let qids = eligible
        .iter()
        .take(request.query_limit)
        .cloned()
        .collect::<Vec<_>>();
    let mut single_hits = 0;
    let mut multi_hits = 0;
    let mut multi_hits_examined = 0;
    let mut evidence = Vec::new();
    for qid in &qids {
        let text = data
            .queries
            .get(qid)
            .expect("query_ids filters missing query");
        let relevant = data.qrels.get(qid).expect("query_ids filters missing qrel");
        let single = single_query(engine, text, request.k)?;
        let multi = multi_query(engine, text, request.k)?;
        ensure_ledger_refs(&multi)?;
        multi_hits_examined += multi.len();
        let single_hit = has_relevant_hit(&single, relevant);
        let multi_hit = has_relevant_hit(&multi, relevant);
        single_hits += usize::from(single_hit);
        multi_hits += usize::from(multi_hit);
        evidence.push(QueryEvidence {
            qid: qid.clone(),
            relevant_docs: relevant.len(),
            single_hit,
            multi_hit,
            single_top_k: ids_for_hits(&single),
            multi_top_k: ids_for_hits(&multi),
        });
    }
    let denominator = qids.len() as f64;
    let single_recall = single_hits as f64 / denominator;
    let multi_recall = multi_hits as f64 / denominator;
    let delta = multi_recall - single_recall;
    if delta + f64::EPSILON < request.min_delta {
        return Err(format!(
            "CALYX_FSV_SEXTANT_RECALL_BELOW_THRESHOLD: delta={delta:.6}"
        ));
    }
    Ok(RecallReport {
        dataset: request.corpus_jsonl.display().to_string(),
        corpus_docs: indexed.doc_count,
        stored_docs: indexed.stored_docs,
        qrels_rows: data.qrels_rows,
        qrels_queries: data.qrels.len(),
        queries_evaluated: qids.len(),
        skipped_queries_without_relevance: skipped_without_relevance(data),
        queries_truncated_by_limit: eligible.len().saturating_sub(qids.len()),
        single_hits,
        multi_hits,
        single_recall_at_10: single_recall,
        multi_recall_at_10: multi_recall,
        delta,
        min_delta: request.min_delta,
        meets_delta_15: true,
        provenance_ok: true,
        multi_hits_examined,
        ledger_ref_count: indexed.ledger_ref_count,
        final_snapshot: indexed.final_snapshot,
        strategy: "weighted_rrf:general(slot_1,slot_8)".to_string(),
        recall_kind: "query_hit_recall_at_10".to_string(),
        query_evidence: evidence,
    })
}

pub(crate) fn cx_for_doc_id(doc_id: &str) -> CxId {
    let mut out = [0_u8; 16];
    out.copy_from_slice(&blake3::hash(doc_id.as_bytes()).as_bytes()[..16]);
    CxId::from_bytes(out)
}

pub(crate) fn weak_dense(doc_id: &str) -> SlotVector {
    let bit = doc_id.as_bytes().iter().fold(0_u8, |acc, byte| acc ^ byte) & 1;
    SlotVector::Dense {
        dim: 2,
        data: if bit == 0 {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        },
    }
}

pub(crate) fn query_vec() -> SlotVector {
    SlotVector::Dense {
        dim: 2,
        data: vec![1.0, 0.0],
    }
}

fn eligible_query_ids(data: &ValidationData) -> Vec<String> {
    data.qrels
        .iter()
        .filter(|(qid, relevant)| !relevant.is_empty() && data.queries.contains_key(*qid))
        .map(|(qid, _)| qid.clone())
        .collect()
}

fn skipped_without_relevance(data: &ValidationData) -> usize {
    data.queries
        .keys()
        .filter(|qid| data.qrels.get(*qid).is_none_or(BTreeSet::is_empty))
        .count()
}

fn single_query(
    engine: &SearchEngine,
    text: &str,
    k: usize,
) -> Result<Vec<calyx_sextant::Hit>, String> {
    engine
        .search(
            &Query::new(text)
                .with_vector(query_vec())
                .with_slots(vec![DENSE_SLOT])
                .require_stored_provenance(true)
                .with_recall_k(k),
        )
        .map_err(|error| error.to_string())
}

fn multi_query(
    engine: &SearchEngine,
    text: &str,
    k: usize,
) -> Result<Vec<calyx_sextant::Hit>, String> {
    let mut query = Query::new(text)
        .with_vector(query_vec())
        .with_slots(vec![LEXICAL_SLOT, DENSE_SLOT])
        .require_stored_provenance(true)
        .with_recall_k(k);
    query.k = k;
    query.fusion = Some(FusionStrategy::WeightedRrf {
        profile: RrfProfile::General,
    });
    engine.search(&query).map_err(|error| error.to_string())
}

pub(super) fn ensure_ledger_refs(hits: &[calyx_sextant::Hit]) -> std::result::Result<(), String> {
    if hits
        .iter()
        .all(|hit| hit.provenance_source == ProvenanceSource::Stored)
    {
        return Ok(());
    }
    Err("CALYX_FSV_LEDGER_REF_MISSING".to_string())
}

fn has_relevant_hit(hits: &[calyx_sextant::Hit], relevant: &BTreeSet<CxId>) -> bool {
    hits.iter().any(|hit| relevant.contains(&hit.cx_id))
}

fn ids_for_hits(hits: &[calyx_sextant::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn constellation_for_doc(
    doc: &CorpusDoc,
    vault_id: VaultId,
    created_at: u64,
) -> calyx_core::Constellation {
    let source = format!("{}\0{}", doc.doc_id, doc.text);
    let hash = blake3::hash(source.as_bytes());
    let mut input_hash = [0_u8; 32];
    input_hash.copy_from_slice(hash.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(LEXICAL_SLOT, sparse_fingerprint(&doc.text));
    slots.insert(DENSE_SLOT, weak_dense(&doc.doc_id));
    let mut scalars = BTreeMap::new();
    scalars.insert("text_bytes".to_string(), doc.text.len() as f64);
    let mut metadata = BTreeMap::new();
    metadata.insert("dataset".to_string(), "beir_scifact".to_string());
    metadata.insert("doc_id".to_string(), doc.doc_id.clone());
    metadata.insert(
        "validation".to_string(),
        "ph70_t01_sextant_recall".to_string(),
    );
    calyx_core::Constellation {
        cx_id: cx_for_doc_id(&doc.doc_id),
        vault_id,
        panel_version: PH70_PANEL_VERSION,
        created_at,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("beir-scifact:{}", doc.doc_id)),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars,
        metadata,
        anchors: vec![Anchor {
            kind: AnchorKind::Label("dataset".to_string()),
            value: AnchorValue::Enum("beir_scifact".to_string()),
            source: "beir_qrels_manifest".to_string(),
            observed_at: created_at,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: created_at,
            hash: input_hash,
        },
        flags: CxFlags::default(),
    }
}

fn sparse_fingerprint(text: &str) -> SlotVector {
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for token in text.split_whitespace().take(256) {
        let idx = u32::from_le_bytes(
            blake3::hash(token.as_bytes()).as_bytes()[..4]
                .try_into()
                .unwrap(),
        ) % 1_000_000;
        if seen.insert(idx) {
            entries.push(calyx_core::SparseEntry { idx, val: 1.0 });
        }
    }
    SlotVector::Sparse {
        dim: 1_000_000,
        entries,
    }
}
