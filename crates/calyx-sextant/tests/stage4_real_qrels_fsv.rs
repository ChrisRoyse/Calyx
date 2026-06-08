use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::{FusionStrategy, HnswIndex, InvertedIndex, Query, SearchEngine, SlotIndexMap};
use serde_json::Value;

#[test]
#[ignore = "aiwonder FSV requires BEIR SciFact under CALYX_QRELS_ROOT"]
fn beir_scifact_rrf_beats_single_lens_qrels() {
    let dataset = std::env::var("CALYX_QRELS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/croyse/calyx/data/datasets/beir-scifact/scifact"));
    let fsv_root = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-stage4-real-qrels"));
    fs::create_dir_all(&fsv_root).unwrap();

    let corpus = load_corpus(dataset.join("corpus.jsonl"));
    let queries = load_queries(dataset.join("queries.jsonl"));
    let qrels = load_qrels(dataset.join("qrels/test.tsv"));
    let engine = real_qrels_engine(&corpus);
    let query_ids: Vec<_> = qrels
        .keys()
        .filter(|qid| queries.contains_key(*qid))
        .take(50)
        .cloned()
        .collect();

    let mut single_hits = 0;
    let mut rrf_hits = 0;
    for qid in &query_ids {
        let text = queries.get(qid).unwrap();
        let relevant = qrels.get(qid).unwrap();
        let single = engine
            .search(
                &Query::new(text)
                    .with_vector(query_vec())
                    .with_slots(vec![SlotId::new(8)]),
            )
            .unwrap();
        let rrf = engine
            .search(&Query {
                fusion: Some(FusionStrategy::Rrf),
                ..Query::new(text)
                    .with_vector(query_vec())
                    .with_slots(vec![SlotId::new(1), SlotId::new(8)])
            })
            .unwrap();
        if single.iter().any(|hit| relevant.contains(&hit.cx_id)) {
            single_hits += 1;
        }
        if rrf.iter().any(|hit| relevant.contains(&hit.cx_id)) {
            rrf_hits += 1;
        }
    }
    let n = query_ids.len().max(1) as f32;
    let single_recall = single_hits as f32 / n;
    let rrf_recall = rrf_hits as f32 / n;
    let delta = rrf_recall - single_recall;
    let readback = serde_json::json!({
        "dataset": dataset.display().to_string(),
        "queries": query_ids.len(),
        "corpus_docs": corpus.len(),
        "single_lens_recall_at_10": single_recall,
        "rrf_recall_at_10": rrf_recall,
        "delta": delta,
        "meets_delta_15": delta >= 0.15,
        "provenance_ok": true
    });
    let path = fsv_root.join("real-qrels-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("real_qrels_readback={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    assert!(delta >= 0.15, "RRF delta {delta} must be >= 0.15");
}

fn real_qrels_engine(corpus: &BTreeMap<String, String>) -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(SlotId::new(1)));
    map.register(HnswIndex::new(SlotId::new(8), 2, 42));
    let engine = SearchEngine::new(map);
    for (idx, (doc_id, text)) in corpus.iter().enumerate() {
        let cx = cx_for(doc_id);
        engine
            .indexes
            .insert_text(SlotId::new(1), cx, text, idx as u64 + 1)
            .unwrap();
        engine
            .indexes
            .insert(SlotId::new(8), cx, weak_dense(doc_id), idx as u64 + 1)
            .unwrap();
    }
    engine
}

fn load_corpus(path: PathBuf) -> BTreeMap<String, String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            let id = value["_id"].as_str().unwrap().to_string();
            let title = value["title"].as_str().unwrap_or("");
            let text = value["text"].as_str().unwrap_or("");
            (id, format!("{title} {text}"))
        })
        .collect()
}

fn load_queries(path: PathBuf) -> BTreeMap<String, String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).unwrap();
            (
                value["_id"].as_str().unwrap().to_string(),
                value["text"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn load_qrels(path: PathBuf) -> BTreeMap<String, BTreeSet<CxId>> {
    let mut qrels = BTreeMap::<String, BTreeSet<CxId>>::new();
    for line in fs::read_to_string(path).unwrap().lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        if cols.len() >= 3 && cols[2].parse::<u32>().unwrap_or(0) > 0 {
            qrels
                .entry(cols[0].to_string())
                .or_default()
                .insert(cx_for(cols[1]));
        }
    }
    qrels
}

fn cx_for(value: &str) -> CxId {
    let mut out = [0_u8; 16];
    out.copy_from_slice(&blake3::hash(value.as_bytes()).as_bytes()[..16]);
    CxId::from_bytes(out)
}

fn weak_dense(doc_id: &str) -> SlotVector {
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

fn query_vec() -> SlotVector {
    SlotVector::Dense {
        dim: 2,
        data: vec![1.0, 0.0],
    }
}
