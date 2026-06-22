use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, Instant};

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId,
};
use calyx_sextant::fusion::pipeline::summarize_pipeline;
use calyx_sextant::fusion::rrf::rrf_contribution;
use calyx_sextant::index::tokenizer::{decode_varint_deltas, encode_varint_deltas, hex, tokenize};
use calyx_sextant::{
    CALYX_SEXTANT_PLAN_UNBOUNDED, CALYX_SEXTANT_POSTINGS_CORRUPT,
    CALYX_SEXTANT_POSTINGS_NOT_SORTED, DualIndex, FreshnessRequirement, FusionStrategy, HnswIndex,
    IntentLabel, InvertedIndex, MaxSimIndex, PlanLimits, QuantConfig, Query, QueryPlanner,
    RerankRequest, RerankerClient, RrfProfile, SearchEngine, SextantIndex, SlotIndexMap,
    compare_lenses, define, neighbors, weighted_profiles,
};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::cx_u8_fill as cx;

#[test]
fn tokenizer_varint_and_bm25_are_byte_exact() {
    assert_eq!(tokenize("Cat, hat! CAT"), ["cat", "hat", "cat"]);
    let encoded = encode_varint_deltas(&[1, 3, 7]).unwrap();
    assert_eq!(hex(&encoded), "010204");
    assert_eq!(decode_varint_deltas(&encoded).unwrap(), vec![1, 3, 7]);
    assert_eq!(
        encode_varint_deltas(&[7, 3]).unwrap_err().code,
        CALYX_SEXTANT_POSTINGS_NOT_SORTED
    );
    assert_eq!(
        decode_varint_deltas(&[0x80]).unwrap_err().code,
        CALYX_SEXTANT_POSTINGS_CORRUPT
    );

    let (engine, ids) = sample_engine();
    let query = Query::new("cat hat")
        .with_slots(vec![SlotId::new(1)])
        .explain(true);
    let hits = engine.search(&query).unwrap();
    assert_eq!(hits[0].cx_id, ids[1]);
    assert!(hits[0].provenance.hash.iter().any(|byte| *byte != 0));
}

#[test]
fn dense_dual_quant_multi_and_slot_map_work() {
    let mut hnsw = HnswIndex::new(SlotId::new(8), 3, 42).with_quant(QuantConfig::scalar8(0.01));
    for i in 0..6 {
        hnsw.insert(cx(i), dense_vec(i as f32, 3), i as u64 + 1)
            .unwrap();
    }
    assert_eq!(hnsw.layer_histogram().iter().sum::<usize>(), 6);
    assert_eq!(
        hnsw.search(&dense_vec(5.0, 3), 2, Some(4)).unwrap()[0].cx_id,
        cx(5)
    );
    let before = hnsw.neighbor_counts();
    hnsw.rebuild().unwrap();
    assert_eq!(before, hnsw.neighbor_counts());

    let mut dual = DualIndex::new(SlotId::new(4), 3, 42).with_boosts(2.0, 0.5);
    dual.insert(cx(1), dense_vec(1.0, 3), 1).unwrap();
    let score = dual.search(&dense_vec(1.0, 3), 1, None).unwrap()[0].score;
    assert!(score > 1.9);

    let mut multi = MaxSimIndex::new(SlotId::new(10), 2);
    multi
        .insert(cx(2), multi_vec(&[[1.0, 0.0], [0.0, 1.0]]), 1)
        .unwrap();
    multi
        .insert(cx(3), multi_vec(&[[0.2, 0.0], [0.0, 0.1]]), 2)
        .unwrap();
    assert_eq!(
        multi.search(&multi_vec(&[[1.0, 0.0]]), 1, None).unwrap()[0].cx_id,
        cx(2)
    );
}

#[test]
fn fusion_planner_freshness_and_navigation_work() {
    let (engine, ids) = sample_engine();
    let query = Query::new("why cat error")
        .with_vector(basis_vec(1))
        .with_slots(vec![SlotId::new(8), SlotId::new(9)])
        .explain(true);
    let hits = engine.search(&query).unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits[0].explain.is_some());
    assert!((rrf_contribution(1.0, 1) - 0.016393442).abs() < 1e-6);
    assert_eq!(weighted_profiles().len(), 14);
    assert!(weighted_profiles()
        .iter()
        .any(|profile| profile.profile == RrfProfile::Lexical && profile.lexical_excludes_dense));

    engine.indexes.set_base_seq(SlotId::new(8), 99).unwrap();
    let stale = engine.search(&query).unwrap_err();
    assert_eq!(stale.code, "CALYX_STALE_DERIVED");
    let stale_ok = Query {
        freshness: FreshnessRequirement::StaleOk { seq_lag: 200 },
        ..query.clone()
    };
    assert!(engine.search(&stale_ok).unwrap()[0].freshness.stale_by > 0);
    engine.indexes.rebuild(SlotId::new(8)).unwrap();

    let planner = QueryPlanner::default();
    let plan = planner.plan(query.clone(), 100).unwrap();
    assert_eq!(plan.intent, IntentLabel::Causal);
    assert!(matches!(plan.strategy, FusionStrategy::WeightedRrf { .. }));
    let mut bad = query.clone();
    bad.k = 10_000;
    assert_eq!(
        planner.plan(bad, 100).unwrap_err().code,
        CALYX_SEXTANT_PLAN_UNBOUNDED
    );

    let near = neighbors(&engine, ids[0], SlotId::new(8), 2).unwrap();
    assert_eq!(near.len(), 2);
    let compare_query = Query::new("compare")
        .with_vector(basis_vec(0))
        .with_slots(vec![SlotId::new(8), SlotId::new(9)]);
    let compared =
        compare_lenses(&engine, &compare_query, &[SlotId::new(8), SlotId::new(9)]).unwrap();
    assert_ne!(compared[0].hits[0].cx_id, compared[1].hits[0].cx_id);
    let definition = define(&engine, ids[0], SlotId::new(8), 2).unwrap();
    assert!(definition.slots.contains_key(&SlotId::new(9)));
}

#[test]
fn pipeline_and_reranker_keep_candidate_text_request_scoped() {
    let stage1 = vec![cx(1), cx(2), cx(3)];
    let final_ids = vec![cx(2), cx(3)];
    let texts = vec!["cat hat".to_string(), "dog log".to_string()];
    let summary = summarize_pipeline(&stage1, &final_ids);
    assert_eq!(summary.stage1_candidates, 3);
    assert!(summary.subset_ok);

    let reranker = RerankerClient::new("http://127.0.0.1:9", Duration::from_millis(5));
    let request = RerankRequest::new("cat", texts);
    let rendered_request = format!("{request:?}");
    assert!(
        !rendered_request.contains("cat hat") && rendered_request.contains("redacted"),
        "rerank request Debug must redact candidate text: {rendered_request}"
    );
    assert_eq!(
        reranker.rerank(&request).unwrap_err().code,
        "CALYX_SEXTANT_RERANKER_TIMEOUT"
    );

    let (engine, _) = sample_engine();
    let sparse_candidates = engine
        .indexes
        .search_text(SlotId::new(1), "cat hat", 3)
        .unwrap()
        .into_iter()
        .map(|hit| hit.cx_id)
        .collect::<Vec<_>>();
    let pipeline_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("cat hat")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(1), SlotId::new(8), SlotId::new(9)])
                .explain(true)
        })
        .unwrap();
    assert!(!pipeline_hits.is_empty());
    assert!(
        pipeline_hits
            .iter()
            .all(|hit| sparse_candidates.contains(&hit.cx_id))
    );
    assert_eq!(
        pipeline_hits[0].explain.as_ref().unwrap().strategy,
        "pipeline"
    );
    let empty_stage1_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("absent-stage-one-token")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(1), SlotId::new(8), SlotId::new(9)])
        })
        .unwrap();
    assert!(empty_stage1_hits.is_empty());
    let no_stage1_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("cat hat")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(8), SlotId::new(9)])
        })
        .unwrap();
    assert!(no_stage1_hits.is_empty());
}

#[test]
#[ignore = "manual FSV writes source-of-truth artifacts"]
fn stage4_full_stack_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-stage4-fsv"));
    fs::create_dir_all(&root).unwrap();

    let (engine, ids) = sample_engine();
    let dense_query = Query::new("why cat error")
        .with_vector(basis_vec(1))
        .with_slots(vec![SlotId::new(8), SlotId::new(9)])
        .explain(true);
    let single = engine
        .search(&Query {
            fusion: Some(FusionStrategy::SingleLens {
                slot: SlotId::new(8),
            }),
            ..dense_query.clone()
        })
        .unwrap();
    let rrf = engine.search(&dense_query).unwrap();
    let sparse = engine
        .search(
            &Query::new("cat hat")
                .with_slots(vec![SlotId::new(1)])
                .explain(true),
        )
        .unwrap();
    let multi = engine
        .indexes
        .search(SlotId::new(10), &multi_vec(&[[1.0, 0.0]]), 2, None)
        .unwrap();
    let pipeline_candidates = engine
        .indexes
        .search_text(SlotId::new(1), "cat hat", 3)
        .unwrap()
        .into_iter()
        .map(|hit| hit.cx_id)
        .collect::<Vec<_>>();
    let pipeline_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("cat hat")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(1), SlotId::new(8), SlotId::new(9)])
                .explain(true)
        })
        .unwrap();
    let empty_stage1_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("absent-stage-one-token")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(1), SlotId::new(8), SlotId::new(9)])
        })
        .unwrap();
    let no_stage1_hits = engine
        .search(&Query {
            fusion: Some(FusionStrategy::Pipeline),
            ..Query::new("cat hat")
                .with_vector(basis_vec(2))
                .with_slots(vec![SlotId::new(8), SlotId::new(9)])
        })
        .unwrap();

    engine.indexes.set_base_seq(SlotId::new(8), 500).unwrap();
    let fresh_error = engine.search(&dense_query).unwrap_err().code.to_string();
    let stale_ok = engine
        .search(&Query {
            freshness: FreshnessRequirement::StaleOk { seq_lag: 1_000 },
            ..dense_query.clone()
        })
        .unwrap();
    engine.indexes.rebuild(SlotId::new(8)).unwrap();

    let planner = QueryPlanner::new(PlanLimits::default());
    let plan = planner.plan(dense_query.clone(), 128).unwrap();
    let unbounded = planner
        .plan(
            Query {
                k: 10_000,
                ..dense_query.clone()
            },
            128,
        )
        .unwrap_err()
        .code
        .to_string();
    let compare_query = Query::new("compare")
        .with_vector(basis_vec(0))
        .with_slots(vec![SlotId::new(8), SlotId::new(9)]);
    let compared =
        compare_lenses(&engine, &compare_query, &[SlotId::new(8), SlotId::new(9)]).unwrap();
    let definition = define(&engine, ids[0], SlotId::new(8), 2).unwrap();
    let reranker = RerankerClient::new("http://127.0.0.1:8089", Duration::from_millis(500));
    let rerank = reranker.rerank(&RerankRequest::new(
        "cat",
        vec!["cat hat".to_string(), "dog log".to_string()],
    ));

    let latencies = measure_latencies(&engine, &dense_query);
    let postings_encoded = encode_varint_deltas(&[1, 3, 7]).unwrap();
    let postings_decoded = decode_varint_deltas(&postings_encoded).unwrap();
    let postings_unsorted = encode_varint_deltas(&[7, 3]).unwrap_err();
    let postings_corrupt = decode_varint_deltas(&[0x80]).unwrap_err();
    let readback = serde_json::json!({
        "hnsw_slots": engine.indexes.slots(),
        "single_lens_hits": single.len(),
        "rrf_hits": rrf.len(),
        "rrf_top_differs_from_single": rrf[0].cx_id != single[0].cx_id || rrf[0].per_lens.len() > 1,
        "sparse_top": sparse[0].cx_id.to_string(),
        "multi_top": multi[0].cx_id.to_string(),
        "pipeline_hits": pipeline_hits.len(),
        "pipeline_subset_ok": pipeline_hits
            .iter()
            .all(|hit| pipeline_candidates.contains(&hit.cx_id)),
        "pipeline_empty_stage1_hits": empty_stage1_hits.len(),
        "pipeline_no_stage1_hits": no_stage1_hits.len(),
        "all_provenanced": rrf.iter().all(|hit| hit.provenance.hash.iter().any(|byte| *byte != 0)),
        "fresh_error": fresh_error,
        "stale_ok_stale_by": stale_ok[0].freshness.stale_by,
        "planner_intent": format!("{:?}", plan.intent),
        "planner_strategy": plan.strategy.name(),
        "unbounded": unbounded,
        "compare_lenses_differ": compared[0].hits[0].cx_id != compared[1].hits[0].cx_id,
        "define_slot_count": definition.slots.len(),
        "rerank": rerank
            .as_ref()
            .map(|response| {
                json!({"scores": response.scores.clone()})
            })
            .unwrap_or_else(|error| json!({"error": error.code})),
        "rrf6_p99_us": latencies.0,
        "pipeline_p99_us": latencies.1,
        "explain_delta_us": latencies.2,
        "varint_hex": hex(&postings_encoded),
        "varint_decoded": postings_decoded,
        "postings_unsorted_error": postings_unsorted.code,
        "postings_corrupt_error": postings_corrupt.code,
    });
    let path = root.join("stage4-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("stage4_fsv_readback={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(rerank.is_ok(), "real reranker response required for FSV");
    assert_eq!(readback["pipeline_subset_ok"], true);
    assert_eq!(readback["pipeline_empty_stage1_hits"], 0);
    assert_eq!(readback["pipeline_no_stage1_hits"], 0);
    assert_eq!(readback["fresh_error"], "CALYX_STALE_DERIVED");
    assert_eq!(readback["unbounded"], CALYX_SEXTANT_PLAN_UNBOUNDED);
    assert_eq!(readback["varint_hex"], "010204");
    assert_eq!(
        readback["postings_unsorted_error"],
        CALYX_SEXTANT_POSTINGS_NOT_SORTED
    );
    assert_eq!(
        readback["postings_corrupt_error"],
        CALYX_SEXTANT_POSTINGS_CORRUPT
    );
}

fn sample_engine() -> (SearchEngine, Vec<CxId>) {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    map.register(HnswIndex::new(SlotId::new(9), 3, 43)).unwrap();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    map.register(MaxSimIndex::new(SlotId::new(10), 2)).unwrap();
    let mut engine = SearchEngine::new(map);
    let ids = vec![cx(1), cx(2), cx(3)];
    let texts = ["dog log", "cat hat", "cat error cause"];
    for (idx, id) in ids.iter().copied().enumerate() {
        let seq = idx as u64 + 1;
        engine
            .indexes
            .insert(SlotId::new(8), id, basis_vec(idx), seq)
            .unwrap();
        engine
            .indexes
            .insert(SlotId::new(9), id, basis_vec(2 - idx), seq)
            .unwrap();
        engine
            .indexes
            .insert_text(SlotId::new(1), id, texts[idx], seq)
            .unwrap();
        engine
            .indexes
            .insert(
                SlotId::new(10),
                id,
                multi_vec(&[[idx as f32 + 1.0, 0.0], [0.0, 1.0]]),
                seq,
            )
            .unwrap();
        engine.put_constellation(sample_constellation(id, seq));
    }
    (engine, ids)
}

fn measure_latencies(engine: &SearchEngine, query: &Query) -> (u128, u128, u128) {
    let mut rrf = Vec::new();
    let mut pipeline = Vec::new();
    let mut explain = Vec::new();
    for _ in 0..16 {
        let start = Instant::now();
        let _ = engine.search(query).unwrap();
        rrf.push(start.elapsed().as_micros());
        let start = Instant::now();
        let _ = summarize_pipeline(&[cx(1), cx(2), cx(3)], &[cx(2), cx(3)]);
        pipeline.push(start.elapsed().as_micros());
        let start = Instant::now();
        let _ = engine
            .search(&Query {
                explain: true,
                ..query.clone()
            })
            .unwrap();
        explain.push(start.elapsed().as_micros());
    }
    (p99(&mut rrf), p99(&mut pipeline), p99(&mut explain))
}

fn p99(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[((values.len() as f32 * 0.99).ceil() as usize).saturating_sub(1)]
}

fn dense_vec(base: f32, dim: u32) -> SlotVector {
    SlotVector::Dense {
        dim,
        data: (0..dim).map(|idx| base + idx as f32 * 0.01).collect(),
    }
}

fn basis_vec(index: usize) -> SlotVector {
    let mut data = vec![0.0; 3];
    data[index % 3] = 1.0;
    SlotVector::Dense { dim: 3, data }
}

fn multi_vec(tokens: &[[f32; 2]]) -> SlotVector {
    SlotVector::Multi {
        token_dim: 2,
        tokens: tokens.iter().map(|t| t.to_vec()).collect(),
    }
}

fn sample_constellation(cx_id: CxId, seq: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id,
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/stage4/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("stage4".to_string()),
            value: AnchorValue::Text("ok".to_string()),
            source: "stage4-fsv".to_string(),
            observed_at: seq,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags::default(),
    }
}
