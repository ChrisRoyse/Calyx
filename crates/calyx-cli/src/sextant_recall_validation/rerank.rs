use std::collections::BTreeMap;
use std::time::Duration;

use calyx_core::{Constellation, CxId};
use calyx_sextant::{Hit, RerankCandidateText, RerankRequest, RerankerClient};

use super::data::ValidationData;
use super::request::RecallRequest;

pub(crate) fn doc_texts_by_cx(
    docs: &BTreeMap<CxId, Constellation>,
    data: &ValidationData,
) -> Result<BTreeMap<CxId, String>, String> {
    let corpus = data
        .corpus
        .iter()
        .map(|doc| (doc.doc_id.as_str(), doc.text.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut out = BTreeMap::new();
    for (cx_id, cx) in docs {
        let doc_id = cx.metadata.get("doc_id").ok_or_else(|| {
            format!("CALYX_FSV_SEXTANT_STORED_DOC_ID_MISSING_FOR_RERANK: {cx_id}")
        })?;
        let text = corpus
            .get(doc_id.as_str())
            .ok_or_else(|| format!("CALYX_FSV_SEXTANT_RERANK_TEXT_MISSING: doc_id={doc_id}"))?;
        out.insert(*cx_id, (*text).to_string());
    }
    Ok(out)
}

pub(crate) fn rerank_hits(
    query: &str,
    hits: Vec<Hit>,
    doc_texts: &BTreeMap<CxId, String>,
    request: &RecallRequest,
) -> Result<Vec<Hit>, String> {
    if hits.is_empty() {
        return Ok(hits);
    }
    let candidates = hits
        .iter()
        .map(|hit| {
            doc_texts
                .get(&hit.cx_id)
                .cloned()
                .map(RerankCandidateText::new)
                .ok_or_else(|| format!("CALYX_FSV_SEXTANT_RERANK_TEXT_MISSING: {}", hit.cx_id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let client = RerankerClient::new(
        request.reranker_endpoint.clone(),
        Duration::from_millis(request.reranker_timeout_ms),
    );
    let response = client
        .rerank(&RerankRequest::from_candidate_texts(
            query.to_string(),
            candidates,
        ))
        .map_err(|error| error.to_string())?;
    let mut scored = hits
        .into_iter()
        .zip(response.scores)
        .enumerate()
        .collect::<Vec<_>>();
    scored.sort_by(
        |(left_order, (_, left_score)), (right_order, (_, right_score))| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left_order.cmp(right_order))
        },
    );
    Ok(scored
        .into_iter()
        .enumerate()
        .map(|(idx, (_, (mut hit, score)))| {
            hit.score = score;
            hit.rank = idx + 1;
            if let Some(explain) = &mut hit.explain {
                explain.strategy = "weighted_rrf_signal_bits+tei_rerank".to_string();
                explain.per_lens_count = hit.per_lens.len();
            }
            hit
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::LedgerRef;
    use calyx_sextant::{FreshnessTag, ProvenanceSource};

    #[test]
    fn missing_candidate_text_fails_closed() {
        let request = RecallRequest {
            corpus_jsonl: "corpus.jsonl".into(),
            queries_jsonl: "queries.jsonl".into(),
            qrels_tsv: "qrels.tsv".into(),
            packed_panel_json: None,
            lens_catalog: None,
            metrics_dir: "metrics".into(),
            vault: "vault".into(),
            query_limit: 1,
            k: 10,
            min_delta: 0.15,
            min_fusion_gain: 0.0,
            reranker_endpoint: "http://127.0.0.1:9".to_string(),
            reranker_timeout_ms: 10,
            rerank_depth: 64,
            vault_id: super::super::request::DEFAULT_VAULT_ID.to_string(),
            vault_salt: "test".to_string(),
        };
        let err = rerank_hits(
            "query",
            vec![hit(CxId::from_bytes([1; 16]))],
            &BTreeMap::new(),
            &request,
        )
        .unwrap_err();

        assert!(err.contains("CALYX_FSV_SEXTANT_RERANK_TEXT_MISSING"));
    }

    fn hit(cx_id: CxId) -> Hit {
        Hit {
            cx_id,
            score: 1.0,
            rank: 1,
            event_time_secs: None,
            temporal_scores: None,
            causal_confidence: calyx_sextant::CausalConfidence::Absent,
            causal_gate: None,
            per_lens: Vec::new(),
            cross_terms_used: false,
            guard: None,
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            provenance_source: ProvenanceSource::Stub,
            freshness: FreshnessTag::fresh(0),
            explain: None,
        }
    }
}
