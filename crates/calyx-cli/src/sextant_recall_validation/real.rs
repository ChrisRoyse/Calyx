use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state, persist_vault_panel_state};
use calyx_sextant::{
    FreshnessTag, FusionContext, FusionStrategy, Hit, ProvenanceSource, RrfProfile, fusion,
};
use std::collections::{BTreeMap, BTreeSet};

use crate::cmd::{
    PersistedSearchIndexes, ingest_text_input, load_search_docs, measure_ingest_constellation,
    measure_text_query_vectors, rebuild_persistent_indexes,
};

use super::data::ValidationData;
use super::engine::{cx_for_doc_id, ensure_ledger_refs};
use super::ir::{IrMetrics, ranking_metrics};
use super::panel::{RealPanelSlot, load_real_panel};
use super::real_output::{PanelMetricEvidence, write_outputs};
use super::real_types::{
    PanelRelevanceReport, QuerySlotEvidence, RealQueryEvidence, SlotMetricReport,
};
use super::request::RecallRequest;
use super::rerank::{doc_texts_by_cx, rerank_hits};

const INGEST_BATCH_ROWS: usize = 128;

#[derive(Clone, Debug)]
struct SlotAccumulator {
    slot: SlotId,
    lens: String,
    metrics: IrMetrics,
    hits_examined: usize,
}

pub(crate) fn run_real_panel(
    request: &RecallRequest,
    data: &ValidationData,
    vault_id: VaultId,
) -> Result<PanelMetricEvidence, String> {
    let panel = load_real_panel(request)?;
    let options = VaultOptions {
        panel: Some(panel.panel.clone()),
        ..VaultOptions::default()
    };
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        options,
    )
    .map_err(|error| error.to_string())?;
    let before = load_search_docs(&vault).map_err(|error| error.to_string())?;
    if !before.is_empty() {
        return Err(format!(
            "CALYX_FSV_SEXTANT_VAULT_NOT_EMPTY: {} stored docs before validation",
            before.len()
        ));
    }
    persist_vault_panel_state(&request.vault, &panel.panel, &panel.registry)
        .map_err(|error| error.to_string())?;
    let state = load_vault_panel_state(&request.vault).map_err(|error| error.to_string())?;
    ingest_corpus(&vault, &state, data)?;
    rebuild_persistent_indexes(&request.vault, &vault).map_err(|error| error.to_string())?;
    let docs = load_search_docs(&vault).map_err(|error| error.to_string())?;
    validate_stored_docs(&docs, data, state.panel.slots.len())?;
    let report = evaluate_panel(&vault, &state, request, data, &docs, panel.slots)?;
    write_outputs(&vault, request, report)
}

fn ingest_corpus(
    vault: &AsterVault,
    state: &VaultPanelState,
    data: &ValidationData,
) -> Result<(), String> {
    let mut rows = Vec::with_capacity(INGEST_BATCH_ROWS);
    for (idx, doc) in data.corpus.iter().enumerate() {
        let pointer = format!("beir-scifact:{}", doc.doc_id);
        let input = ingest_text_input(doc.text.clone()).with_pointer(pointer);
        let mut cx = measure_ingest_constellation(vault, state, input, idx as u64 + 1)
            .map_err(|error| error.to_string())?;
        cx.cx_id = cx_for_doc_id(&doc.doc_id);
        cx.metadata
            .insert("dataset".to_string(), "beir_scifact".to_string());
        cx.metadata.insert("doc_id".to_string(), doc.doc_id.clone());
        cx.metadata.insert(
            "validation".to_string(),
            "issue_727_real_panel_relevance".to_string(),
        );
        cx.anchors.push(Anchor {
            kind: AnchorKind::Label("dataset".to_string()),
            value: AnchorValue::Enum("beir_scifact".to_string()),
            source: "beir_qrels_manifest".to_string(),
            observed_at: idx as u64 + 1,
            confidence: 1.0,
        });
        cx.flags = CxFlags {
            ungrounded: false,
            ..cx.flags
        };
        reject_absent_slots(&doc.doc_id, &cx.slots)?;
        rows.push(cx);
        if rows.len() >= INGEST_BATCH_ROWS {
            flush_ingest_batch(vault, &mut rows, idx + 1, data.corpus.len())?;
        }
    }
    flush_ingest_batch(vault, &mut rows, data.corpus.len(), data.corpus.len())?;
    Ok(())
}

fn flush_ingest_batch(
    vault: &AsterVault,
    rows: &mut Vec<calyx_core::Constellation>,
    measured_docs: usize,
    total_docs: usize,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let batch_len = rows.len();
    vault
        .put_batch(std::mem::take(rows))
        .map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())?;
    eprintln!(
        "CALYX_FSV_SEXTANT_INGEST_PROGRESS measured_docs={measured_docs} total_docs={total_docs} stored_batch={batch_len} latest_seq={}",
        vault.latest_seq()
    );
    Ok(())
}

fn reject_absent_slots(doc_id: &str, slots: &BTreeMap<SlotId, SlotVector>) -> Result<(), String> {
    let absent = slots
        .iter()
        .filter_map(|(slot, vector)| vector.is_absent().then_some(slot.to_string()))
        .collect::<Vec<_>>();
    if absent.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "CALYX_FSV_SEXTANT_PANEL_ABSENT_SLOT: doc_id={doc_id} slots={}",
            absent.join(",")
        ))
    }
}

fn validate_stored_docs(
    docs: &BTreeMap<CxId, calyx_core::Constellation>,
    data: &ValidationData,
    expected_slots: usize,
) -> Result<(), String> {
    if docs.len() != data.corpus.len() {
        return Err(format!(
            "CALYX_FSV_SEXTANT_STORED_DOC_COUNT_MISMATCH: stored={} corpus={}",
            docs.len(),
            data.corpus.len()
        ));
    }
    for doc in &data.corpus {
        let cx_id = cx_for_doc_id(&doc.doc_id);
        let stored = docs
            .get(&cx_id)
            .ok_or_else(|| format!("CALYX_FSV_SEXTANT_STORED_DOC_MISSING: {}", doc.doc_id))?;
        if stored.metadata.get("doc_id") != Some(&doc.doc_id) {
            return Err(format!(
                "CALYX_FSV_SEXTANT_DOC_ID_MISMATCH: expected {}",
                doc.doc_id
            ));
        }
        let real_slots = stored
            .slots
            .values()
            .filter(|vector| !vector.is_absent())
            .count();
        if real_slots != expected_slots {
            return Err(format!(
                "CALYX_FSV_SEXTANT_SLOT_COUNT_MISMATCH: doc_id={} slots={real_slots}/{expected_slots}",
                doc.doc_id
            ));
        }
    }
    Ok(())
}

fn evaluate_panel(
    vault: &AsterVault,
    state: &VaultPanelState,
    request: &RecallRequest,
    data: &ValidationData,
    docs: &BTreeMap<CxId, calyx_core::Constellation>,
    panel_slots: Vec<RealPanelSlot>,
) -> Result<PanelRelevanceReport, String> {
    let eligible = eligible_qids(data);
    if eligible.is_empty() {
        return Err("CALYX_FSV_EMPTY_QRELS".to_string());
    }
    let qids = eligible
        .iter()
        .take(request.query_limit)
        .cloned()
        .collect::<Vec<_>>();
    let indexes =
        PersistedSearchIndexes::open(&request.vault).map_err(|error| error.to_string())?;
    if indexes.max_len() == 0 {
        return Err("CALYX_FSV_SEXTANT_EMPTY_INDEX".to_string());
    }
    let doc_ids = stored_doc_id_map(docs)?;
    let doc_texts = doc_texts_by_cx(docs, data)?;
    let slot_names = panel_slots
        .iter()
        .map(|slot| (slot.slot, slot.lens.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut accum = panel_slots
        .iter()
        .map(|slot| {
            (
                slot.slot,
                SlotAccumulator {
                    slot: slot.slot,
                    lens: slot.lens.clone(),
                    metrics: IrMetrics::default(),
                    hits_examined: 0,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut fused_metrics = IrMetrics::default();
    let mut candidate_fusion_metrics = IrMetrics::default();
    let mut fused_hits_examined = 0usize;
    let mut candidate_hits_examined = 0usize;
    let mut evidence = Vec::new();
    for qid in &qids {
        let query = data.queries.get(qid).expect("eligible qid has query");
        let relevant = relevant_for_qid(data, qid, &doc_ids)?;
        let query_vectors = measure_text_query_vectors(state, query).map_err(|e| e.to_string())?;
        require_query_vectors(&query_vectors, &panel_slots)?;
        let search_k = request.k.max(request.rerank_depth);
        let mut per_slot = BTreeMap::new();
        let mut singles = Vec::new();
        for (slot, vector) in query_vectors {
            let hits = indexes
                .search(slot, &vector, search_k)
                .map_err(|error| error.to_string())?;
            let ranking = hits
                .iter()
                .take(request.k)
                .map(|hit| hit.cx_id)
                .collect::<Vec<_>>();
            let metrics = ranking_metrics(&ranking, &relevant, request.k);
            let accumulator = accum
                .get_mut(&slot)
                .ok_or_else(|| format!("CALYX_FSV_SEXTANT_UNEXPECTED_SLOT: {slot}"))?;
            accumulator.metrics = accumulator.metrics.add(metrics);
            accumulator.hits_examined += hits.len();
            singles.push(QuerySlotEvidence {
                slot,
                lens: slot_names
                    .get(&slot)
                    .cloned()
                    .unwrap_or_else(|| format!("slot_{slot}")),
                metrics,
                top_k: ids(&ranking),
            });
            per_slot.insert(slot, hits);
        }
        let candidate_fused = fuse_slots(&per_slot, &panel_slots, request.rerank_depth);
        let rerank_candidates = candidate_fused.len();
        let candidate_ranking = candidate_fused
            .iter()
            .take(request.k)
            .map(|hit| hit.cx_id)
            .collect::<Vec<_>>();
        let candidate_metrics = ranking_metrics(&candidate_ranking, &relevant, request.k);
        candidate_fusion_metrics = candidate_fusion_metrics.add(candidate_metrics);
        candidate_hits_examined += rerank_candidates;
        let mut fused = rerank_hits(query, candidate_fused, &doc_texts, request)?;
        fused.truncate(request.k);
        attach_provenance(vault, &mut fused, docs)?;
        ensure_ledger_refs(&fused)?;
        let fused_ranking = fused.iter().map(|hit| hit.cx_id).collect::<Vec<_>>();
        let metrics = ranking_metrics(&fused_ranking, &relevant, request.k);
        fused_metrics = fused_metrics.add(metrics);
        fused_hits_examined += rerank_candidates;
        evidence.push(RealQueryEvidence {
            qid: qid.clone(),
            relevant_docs: relevant.len(),
            rerank_candidates,
            candidate_fusion_metrics: candidate_metrics,
            fused_metrics: metrics,
            fused_top_k: ids(&fused_ranking),
            singles,
        });
    }
    let denom = qids.len() as f64;
    let single_reports = accum
        .into_values()
        .map(|row| SlotMetricReport {
            label: format!("single:{}", row.lens),
            slot: Some(row.slot),
            lens: Some(row.lens),
            metrics: row.metrics.div(denom),
            hits_examined: row.hits_examined,
        })
        .collect::<Vec<_>>();
    let best_single = best_single(single_reports)?;
    let candidate_fusion = SlotMetricReport {
        label: "candidate_fusion:weighted_rrf_signal_bits".to_string(),
        slot: None,
        lens: None,
        metrics: candidate_fusion_metrics.div(denom),
        hits_examined: candidate_hits_examined,
    };
    let fused = SlotMetricReport {
        label: "fused:weighted_rrf_signal_bits+tei_rerank".to_string(),
        slot: None,
        lens: None,
        metrics: fused_metrics.div(denom),
        hits_examined: fused_hits_examined,
    };
    let delta = fused.metrics.ndcg_at_k - best_single.metrics.ndcg_at_k;
    let meets_gate = delta + f64::EPSILON >= request.min_fusion_gain;
    if !meets_gate {
        return Err(format!(
            "CALYX_FSV_SEXTANT_FUSION_BELOW_BEST_SINGLE: fused_ndcg={:.6} best_single_ndcg={:.6} min_gain={:.6}",
            fused.metrics.ndcg_at_k, best_single.metrics.ndcg_at_k, request.min_fusion_gain
        ));
    }
    Ok(PanelRelevanceReport {
        dataset: request.corpus_jsonl.display().to_string(),
        corpus_docs: data.corpus.len(),
        stored_docs: docs.len(),
        qrels_rows: data.qrels_rows,
        qrels_queries: data.graded_qrels.len(),
        queries_evaluated: qids.len(),
        skipped_queries_without_relevance: skipped_without_relevance(data),
        queries_truncated_by_limit: eligible.len().saturating_sub(qids.len()),
        k: request.k,
        panel_slots,
        final_snapshot: vault.snapshot(),
        provenance_ok: true,
        strategy: format!(
            "weighted_rrf:signal_bits + tei_rerank:{} depth={}",
            request.reranker_endpoint, request.rerank_depth
        ),
        metric_kind: "ndcg_at_k/true_recall_at_k/mrr_against_graded_qrels".to_string(),
        best_single,
        candidate_fusion,
        fused,
        fusion_minus_best_ndcg: delta,
        min_fusion_gain: request.min_fusion_gain,
        meets_fusion_gate: true,
        query_evidence: evidence,
    })
}

fn eligible_qids(data: &ValidationData) -> Vec<String> {
    data.graded_qrels
        .iter()
        .filter(|(qid, qrels)| !qrels.is_empty() && data.queries.contains_key(*qid))
        .map(|(qid, _)| qid.clone())
        .collect()
}

fn skipped_without_relevance(data: &ValidationData) -> usize {
    data.queries
        .keys()
        .filter(|qid| data.graded_qrels.get(*qid).is_none_or(Vec::is_empty))
        .count()
}

fn stored_doc_id_map(
    docs: &BTreeMap<CxId, calyx_core::Constellation>,
) -> Result<BTreeMap<String, CxId>, String> {
    let mut out = BTreeMap::new();
    for (cx_id, cx) in docs {
        let Some(doc_id) = cx.metadata.get("doc_id") else {
            return Err(format!("CALYX_FSV_SEXTANT_STORED_DOC_ID_MISSING: {cx_id}"));
        };
        out.insert(doc_id.clone(), *cx_id);
    }
    Ok(out)
}

fn relevant_for_qid(
    data: &ValidationData,
    qid: &str,
    doc_ids: &BTreeMap<String, CxId>,
) -> Result<BTreeMap<CxId, u32>, String> {
    let mut relevant = BTreeMap::<CxId, u32>::new();
    for qrel in data
        .graded_qrels
        .get(qid)
        .ok_or_else(|| format!("CALYX_FSV_SEXTANT_QREL_QUERY_MISSING: {qid}"))?
    {
        let cx_id = doc_ids.get(&qrel.doc_id).ok_or_else(|| {
            format!(
                "CALYX_FSV_SEXTANT_QREL_DOC_MISSING: qid={qid} doc_id={}",
                qrel.doc_id
            )
        })?;
        relevant
            .entry(*cx_id)
            .and_modify(|rel| *rel = (*rel).max(qrel.relevance))
            .or_insert(qrel.relevance);
    }
    Ok(relevant)
}

fn require_query_vectors(
    query_vectors: &[(SlotId, SlotVector)],
    panel_slots: &[RealPanelSlot],
) -> Result<(), String> {
    let measured = query_vectors
        .iter()
        .map(|(slot, _)| *slot)
        .collect::<BTreeSet<_>>();
    let missing = panel_slots
        .iter()
        .filter(|slot| !measured.contains(&slot.slot))
        .map(|slot| slot.slot.to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "CALYX_FSV_SEXTANT_QUERY_VECTOR_MISSING: slots={}",
            missing.join(",")
        ))
    }
}

fn fuse_slots(
    per_slot: &BTreeMap<SlotId, Vec<calyx_sextant::IndexSearchHit>>,
    panel_slots: &[RealPanelSlot],
    k: usize,
) -> Vec<Hit> {
    let weights = panel_slots
        .iter()
        .map(|slot| (slot.slot, slot.weight.max(0.0)))
        .collect::<BTreeMap<_, _>>();
    let context = FusionContext {
        k,
        explain: true,
        strategy: FusionStrategy::WeightedRrf {
            profile: RrfProfile::General,
        },
        weights,
        stage1_slots: Vec::new(),
    };
    fusion::fuse(per_slot, &context)
}

fn attach_provenance(
    vault: &AsterVault,
    hits: &mut [Hit],
    docs: &BTreeMap<CxId, calyx_core::Constellation>,
) -> Result<(), String> {
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            format!(
                "CALYX_FSV_SEXTANT_HIT_DOC_MISSING: fused hit {} missing from Aster",
                hit.cx_id
            )
        })?;
        hit.provenance = cx.provenance.clone();
        hit.provenance_source = ProvenanceSource::Stored;
        hit.freshness = FreshnessTag::fresh(vault.latest_seq());
    }
    Ok(())
}

fn best_single(reports: Vec<SlotMetricReport>) -> Result<SlotMetricReport, String> {
    reports
        .into_iter()
        .max_by(|a, b| {
            a.metrics
                .ndcg_at_k
                .total_cmp(&b.metrics.ndcg_at_k)
                .then_with(|| a.metrics.recall_at_k.total_cmp(&b.metrics.recall_at_k))
                .then_with(|| a.metrics.mrr.total_cmp(&b.metrics.mrr))
        })
        .ok_or_else(|| "CALYX_FSV_SEXTANT_NO_SINGLE_LENS_METRICS".to_string())
}

fn ids(ranking: &[CxId]) -> Vec<String> {
    ranking.iter().map(ToString::to_string).collect()
}
