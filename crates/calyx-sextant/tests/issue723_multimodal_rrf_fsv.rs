use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::SlotId;
use calyx_sextant::fusion::profiles::lookup;
use calyx_sextant::fusion::weighted_rrf_fuse;
use calyx_sextant::{FusionContext, FusionStrategy, IndexSearchHit, RrfProfile};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::cx_u8_fill as cx;

#[test]
fn issue723_multimodal_weighted_rrf_readback() {
    let profile = lookup(RrfProfile::Multimodal).expect("multimodal profile");
    let mut results = multimodal_results();
    results.insert(SlotId::new(1), vec![hit(0x01, 1, 500.0)]);

    let context = multimodal_context(&profile, 8);
    let hits = weighted_rrf_fuse(&results, &context);
    let top = hits.first().expect("multimodal hits").cx_id;
    let unlisted_present = hits.iter().any(|row| row.cx_id == cx(0x01));
    let edge_empty = edge_case_readback("empty_inputs", BTreeMap::new(), &context);
    let edge_unlisted_only = edge_case_readback(
        "unlisted_slot_only",
        BTreeMap::from([(SlotId::new(1), vec![hit(0x01, 1, 999.0)])]),
        &context,
    );
    let edge_partial = edge_case_readback(
        "partial_multimodal_slots",
        BTreeMap::from([
            (SlotId::new(10), vec![hit(0xcc, 1, 0.91)]),
            (
                SlotId::new(11),
                vec![hit(0xdd, 1, 0.99), hit(0xcc, 2, 0.80)],
            ),
        ]),
        &context,
    );
    let slot_weights = profile
        .weights
        .iter()
        .map(|(slot, weight)| json!({"slot": slot.get(), "weight": weight}))
        .collect::<Vec<_>>();
    let top_lens = hits[0]
        .per_lens
        .iter()
        .map(|row| {
            json!({
                "slot": row.slot.get(),
                "rank": row.rank,
                "weight": row.weight,
                "contribution": row.contribution,
            })
        })
        .collect::<Vec<_>>();

    write_readback(json!({
        "source_of_truth": "stored Sextant issue723 multimodal weighted-RRF readback JSON",
        "profile": "Multimodal",
        "profile_slots": slot_weights,
        "happy_path_input": summarize_inputs(&results),
        "unlisted_slot_1_present": unlisted_present,
        "top_cx": top.to_string(),
        "top_per_lens": top_lens,
        "strategy": context.strategy.name(),
        "hit_count": hits.len(),
        "edge_cases": [edge_empty, edge_unlisted_only, edge_partial],
    }));

    assert_eq!(
        profile
            .weights
            .keys()
            .map(|slot| slot.get())
            .collect::<Vec<_>>(),
        [8, 9, 10, 11]
    );
    assert_eq!(top, cx(0xaa));
    assert!(!unlisted_present);
    assert_eq!(edge_empty["after"]["hit_count"], 0);
    assert_eq!(edge_unlisted_only["after"]["hit_count"], 0);
    assert_eq!(
        edge_partial["after"]["top_cx"],
        cx(0xcc).to_string(),
        "slot-10 + slot-11 agreement must outrank a slot-11-only hit"
    );
}

fn multimodal_context(
    profile: &calyx_sextant::fusion::profiles::WeightedProfile,
    k: usize,
) -> FusionContext {
    FusionContext {
        k,
        explain: true,
        strategy: FusionStrategy::WeightedRrf {
            profile: RrfProfile::Multimodal,
        },
        weights: profile.weights.clone(),
        stage1_slots: Vec::new(),
    }
}

fn multimodal_results() -> BTreeMap<SlotId, Vec<IndexSearchHit>> {
    BTreeMap::from([
        (SlotId::new(8), vec![hit(0xaa, 1, 0.90), hit(0x88, 2, 0.80)]),
        (SlotId::new(9), vec![hit(0xaa, 1, 0.85), hit(0x99, 2, 0.81)]),
        (
            SlotId::new(10),
            vec![hit(0xaa, 1, 0.83), hit(0x10, 2, 0.72)],
        ),
        (
            SlotId::new(11),
            vec![hit(0xbb, 1, 0.99), hit(0xaa, 2, 0.76)],
        ),
    ])
}

fn hit(value: u8, rank: usize, score: f32) -> IndexSearchHit {
    IndexSearchHit {
        cx_id: cx(value),
        score,
        rank,
    }
}

fn edge_case_readback(
    name: &str,
    results: BTreeMap<SlotId, Vec<IndexSearchHit>>,
    context: &FusionContext,
) -> serde_json::Value {
    let before = summarize_inputs(&results);
    let hits = weighted_rrf_fuse(&results, context);
    let after = json!({
        "hit_count": hits.len(),
        "top_cx": hits.first().map(|hit| hit.cx_id.to_string()),
        "outputs": hits
            .iter()
            .map(|hit| json!({
                "cx": hit.cx_id.to_string(),
                "rank": hit.rank,
                "score": hit.score,
                "per_lens": hit.per_lens
                    .iter()
                    .map(|row| json!({
                        "slot": row.slot.get(),
                        "rank": row.rank,
                        "weight": row.weight,
                        "contribution": row.contribution,
                    }))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    });
    json!({ "name": name, "before": before, "after": after })
}

fn summarize_inputs(results: &BTreeMap<SlotId, Vec<IndexSearchHit>>) -> Vec<serde_json::Value> {
    results
        .iter()
        .map(|(slot, hits)| {
            json!({
                "slot": slot.get(),
                "hits": hits
                    .iter()
                    .map(|hit| json!({
                        "cx": hit.cx_id.to_string(),
                        "rank": hit.rank,
                        "score": hit.score,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

fn write_readback(value: serde_json::Value) {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue723-multimodal-rrf"));
    fs::create_dir_all(&root).expect("create FSV root");
    let path = root.join("issue723-multimodal-rrf-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("ISSUE723_MULTIMODAL_RRF_READBACK={}", path.display());
}
