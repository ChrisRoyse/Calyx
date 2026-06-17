use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::LedgerRef;
use calyx_core::VaultStore;
use calyx_sextant::{FreshnessTag, Hit, ProvenanceSource};
use proptest::prelude::*;

use super::data::{CorpusDoc, ValidationData};
use super::engine::{build_engine, cx_for_doc_id, ensure_ledger_refs, evaluate_recall, weak_dense};
use super::request::{DEFAULT_VAULT_ID, RecallRequest};

#[test]
fn synthetic_recall_delta_uses_known_query_hits() {
    let root = temp_root("sextant-recall-known");
    let request = request_for(&root, 2, 0.15);
    let data = synthetic_data();
    let vault = AsterVault::new_durable(
        &request.vault,
        DEFAULT_VAULT_ID.parse().unwrap(),
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let indexed = build_engine(&vault, &data).unwrap();
    let report = evaluate_recall(&indexed.engine, &data, &request, &indexed).unwrap();

    assert_eq!(report.queries_evaluated, 2);
    assert_eq!(report.single_recall_at_10, 0.0);
    assert_eq!(report.multi_recall_at_10, 1.0);
    assert!(report.delta >= 0.15);
    assert!(vault.snapshot() > 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn qrel_query_with_no_relevant_docs_is_skipped() {
    let root = temp_root("sextant-recall-skip-empty");
    let request = request_for(&root, 3, 0.15);
    let mut data = synthetic_data();
    data.queries
        .insert("q-empty".to_string(), "none".to_string());
    data.qrels.insert("q-empty".to_string(), BTreeSet::new());
    let vault = AsterVault::new_durable(
        &request.vault,
        DEFAULT_VAULT_ID.parse().unwrap(),
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let indexed = build_engine(&vault, &data).unwrap();
    let report = evaluate_recall(&indexed.engine, &data, &request, &indexed).unwrap();

    assert_eq!(report.queries_evaluated, 2);
    assert!(report.query_evidence.iter().all(|row| row.qid != "q-empty"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn perfect_single_lens_fails_delta_gate() {
    let root = temp_root("sextant-recall-perfect-single");
    let request = request_for(&root, 1, 0.15);
    let data = ValidationData {
        corpus: vec![CorpusDoc {
            doc_id: "even2".to_string(),
            text: "alpha exact".to_string(),
        }],
        queries: BTreeMap::from([("q1".to_string(), "alpha".to_string())]),
        qrels: BTreeMap::from([("q1".to_string(), BTreeSet::from([cx_for_doc_id("even2")]))]),
        graded_qrels: BTreeMap::new(),
        qrels_rows: 1,
    };
    let vault = AsterVault::new_durable(
        &request.vault,
        DEFAULT_VAULT_ID.parse().unwrap(),
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let indexed = build_engine(&vault, &data).unwrap();
    let error = evaluate_recall(&indexed.engine, &data, &request, &indexed).unwrap_err();

    assert!(error.contains("CALYX_FSV_SEXTANT_RECALL_BELOW_THRESHOLD"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_ledger_ref_fails_closed() {
    let mut hit = stub_hit();
    assert_eq!(
        ensure_ledger_refs(&[hit.clone()]).unwrap_err(),
        "CALYX_FSV_LEDGER_REF_MISSING"
    );
    hit.provenance_source = ProvenanceSource::Stored;
    ensure_ledger_refs(&[hit]).unwrap();
}

#[test]
fn empty_qrels_fail_closed() {
    let root = temp_root("sextant-recall-empty-qrels");
    let request = request_for(&root, 1, 0.15);
    let data = ValidationData {
        corpus: vec![CorpusDoc {
            doc_id: "d1".to_string(),
            text: "alpha".to_string(),
        }],
        queries: BTreeMap::from([("q1".to_string(), "alpha".to_string())]),
        qrels: BTreeMap::new(),
        graded_qrels: BTreeMap::new(),
        qrels_rows: 0,
    };
    let vault = AsterVault::new_durable(
        &request.vault,
        DEFAULT_VAULT_ID.parse().unwrap(),
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let indexed = build_engine(&vault, &data).unwrap();
    let error = evaluate_recall(&indexed.engine, &data, &request, &indexed).unwrap_err();

    assert_eq!(error, "CALYX_FSV_EMPTY_QRELS");
    let _ = fs::remove_dir_all(root);
}

proptest! {
    #[test]
    fn delta_gate_matches_threshold(single in 0usize..100, extra in 0usize..100) {
        let n = 100.0;
        let single_recall = single as f64 / n;
        let multi_recall = (single + extra).min(100) as f64 / n;
        let delta = multi_recall - single_recall;
        let gate = delta + f64::EPSILON >= 0.15;
        prop_assert_eq!(gate, multi_recall + f64::EPSILON >= single_recall + 0.15);
    }
}

fn synthetic_data() -> ValidationData {
    let alpha_id = doc_id_with_dense_bit("alpha-rel", 1);
    let beta_id = doc_id_with_dense_bit("beta-rel", 1);
    let mut corpus = vec![
        CorpusDoc {
            doc_id: alpha_id.clone(),
            text: "alpha grounded qrel".to_string(),
        },
        CorpusDoc {
            doc_id: beta_id.clone(),
            text: "beta grounded qrel".to_string(),
        },
    ];
    for idx in 0..12 {
        corpus.push(CorpusDoc {
            doc_id: doc_id_with_dense_bit(&format!("dense-decoy-{idx}"), 0),
            text: format!("unrelated decoy {idx}"),
        });
    }
    ValidationData {
        corpus,
        queries: BTreeMap::from([
            ("q1".to_string(), "alpha".to_string()),
            ("q2".to_string(), "beta".to_string()),
        ]),
        qrels: BTreeMap::from([
            ("q1".to_string(), BTreeSet::from([cx_for_doc_id(&alpha_id)])),
            ("q2".to_string(), BTreeSet::from([cx_for_doc_id(&beta_id)])),
        ]),
        graded_qrels: BTreeMap::new(),
        qrels_rows: 2,
    }
}

fn doc_id_with_dense_bit(prefix: &str, bit: u8) -> String {
    for ordinal in 0..100 {
        let candidate = format!("{prefix}-{ordinal}");
        let data = match weak_dense(&candidate) {
            calyx_core::SlotVector::Dense { data, .. } => data,
            _ => unreachable!("weak_dense is dense"),
        };
        if (data[0] == 1.0) == (bit == 0) {
            return candidate;
        }
    }
    panic!("could not find doc id with requested dense bit");
}

fn request_for(root: &std::path::Path, query_limit: usize, min_delta: f64) -> RecallRequest {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).unwrap();
    RecallRequest {
        corpus_jsonl: root.join("corpus.jsonl"),
        queries_jsonl: root.join("queries.jsonl"),
        qrels_tsv: root.join("qrels.tsv"),
        packed_panel_json: None,
        lens_catalog: None,
        metrics_dir: root.join("metrics"),
        vault: root.join("vault"),
        query_limit,
        k: 10,
        min_delta,
        min_fusion_gain: 0.0,
        reranker_endpoint: "http://127.0.0.1:8089".to_string(),
        reranker_timeout_ms: 30_000,
        rerank_depth: 64,
        vault_id: DEFAULT_VAULT_ID.to_string(),
        vault_salt: "calyx-test-sextant-recall".to_string(),
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}

fn stub_hit() -> Hit {
    Hit {
        cx_id: cx_for_doc_id("stub"),
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
