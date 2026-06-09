use std::collections::BTreeMap;

use calyx_core::{CxId, SlotId};
use calyx_lodestar::{GroundednessReport, Kernel, RecallReport, build_kernel_index, kernel_search};
use calyx_ward::{
    GuardId, GuardPolicy, GuardProfile, KernelFirstQueryVerdict, NoveltyAction, ProducedSlots,
    RegionSource, TrustedRegion, guard_query_kernel_first,
};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn lodestar_kernel_search_can_feed_kernel_first_guard() {
    let profile = sample_profile();
    let query = unit_query();
    let hits = lodestar_hits();
    let kernel = regions_from_hits(&hits);
    let peripheral = vec![region(cx(20), 0.99, 0.99)];

    let verdict =
        guard_query_kernel_first(&profile, &query, &kernel, &peripheral).expect("kernel first");

    match verdict {
        KernelFirstQueryVerdict::Pass {
            nearest_cx,
            match_source,
            ..
        } => {
            assert_eq!(hits[0].0, cx(10));
            assert_eq!(nearest_cx, cx(10));
            assert_eq!(match_source, RegionSource::KernelNear);
        }
        KernelFirstQueryVerdict::Ood { .. } => panic!("expected kernel pass"),
    }
}

fn sample_profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(1), 0.70);
    tau.insert(slot(2), 0.70);
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-query".to_string(),
        tau,
        required_slots: vec![slot(1), slot(2)],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn lodestar_hits() -> Vec<(CxId, f32)> {
    let index = build_kernel_index(&kernel(vec![cx(10), cx(11)]), &kernel_embeddings())
        .expect("build kernel index");
    kernel_search(&index, &[1.0, 0.0], 2).expect("kernel search")
}

fn regions_from_hits(hits: &[(CxId, f32)]) -> Vec<TrustedRegion> {
    hits.iter()
        .map(|(cx_id, _)| {
            let score = if *cx_id == cx(10) { 0.75 } else { 0.55 };
            region(*cx_id, score, score)
        })
        .collect()
}

fn region(cx_id: CxId, slot1_cos: f32, slot2_cos: f32) -> TrustedRegion {
    TrustedRegion {
        cx_id,
        slots: slot_vectors(&[
            (slot(1), cos_vector(slot1_cos)),
            (slot(2), cos_vector(slot2_cos)),
        ]),
    }
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(99),
        panel_version: 42,
        anchor_kind: Some("synthetic_anchor".to_string()),
        corpus_shard_hash: [7; 32],
        members: members.clone(),
        kernel_graph: members,
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "synthetic-lodestar-index".to_string(),
        warnings: Vec::new(),
    }
}

fn kernel_embeddings() -> BTreeMap<CxId, Vec<f32>> {
    BTreeMap::from([(cx(10), vec![1.0, 0.0]), (cx(11), vec![0.0, 1.0])])
}

fn unit_query() -> ProducedSlots {
    slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])])
}

fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> ProducedSlots {
    entries.iter().cloned().collect()
}

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
