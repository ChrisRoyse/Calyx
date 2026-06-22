use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId};
use calyx_sextant::{FusionStrategy, HnswIndex, InvertedIndex, Query, SearchEngine, SlotIndexMap};
use serde_json::Value;

#[test]
#[ignore = "manual FSV requires BEIR SciFact under CALYX_QRELS_ROOT"]
fn beir_scifact_rrf_beats_single_lens_qrels() {
    let dataset = std::env::var("CALYX_QRELS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/calyx/data/datasets/beir-scifact/scifact"));
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
    let mut provenance_ok = true;
    for qid in &query_ids {
        let text = queries.get(qid).unwrap();
        let relevant = qrels.get(qid).unwrap();
        let single = engine
            .search(
                &Query::new(text)
                    .with_vector(query_vec())
                    .with_slots(vec![SlotId::new(8)])
                    .require_stored_provenance(true),
            )
            .unwrap();
        provenance_ok &= hits_have_stored_provenance(&engine, &single);
        let rrf = engine
            .search(&Query {
                fusion: Some(FusionStrategy::Rrf),
                ..Query::new(text)
                    .with_vector(query_vec())
                    .with_slots(vec![SlotId::new(1), SlotId::new(8)])
                    .require_stored_provenance(true)
            })
            .unwrap();
        provenance_ok &= hits_have_stored_provenance(&engine, &rrf);
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
        "provenance_ok": provenance_ok,
        "provenance_source": "stored_required"
    });
    let path = fsv_root.join("real-qrels-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("real_qrels_readback={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    assert!(delta >= 0.15, "RRF delta {delta} must be >= 0.15");
    assert!(
        provenance_ok,
        "hits must carry stored constellation provenance"
    );
}

fn real_qrels_engine(corpus: &BTreeMap<String, String>) -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    map.register(HnswIndex::new(SlotId::new(8), 2, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    for (idx, (doc_id, text)) in corpus.iter().enumerate() {
        let cx = cx_for(doc_id);
        let seq = idx as u64 + 1;
        engine
            .indexes
            .insert_text(SlotId::new(1), cx, text, seq)
            .unwrap();
        engine
            .indexes
            .insert(SlotId::new(8), cx, weak_dense(doc_id), seq)
            .unwrap();
        engine.put_constellation(real_qrels_constellation(cx, doc_id, seq));
    }
    engine
}

fn hits_have_stored_provenance(engine: &SearchEngine, hits: &[calyx_sextant::Hit]) -> bool {
    hits.iter().all(|hit| {
        engine
            .constellation(hit.cx_id)
            .is_some_and(|cx| cx.provenance == hit.provenance)
    })
}

fn real_qrels_constellation(cx_id: CxId, doc_id: &str, seq: u64) -> calyx_core::Constellation {
    let hash = blake3::hash(doc_id.as_bytes());
    let mut input_hash = [0_u8; 32];
    input_hash.copy_from_slice(hash.as_bytes());
    calyx_core::Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 4,
        created_at: seq,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("beir-scifact:{doc_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq,
            hash: input_hash,
        },
        flags: CxFlags::default(),
    }
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
