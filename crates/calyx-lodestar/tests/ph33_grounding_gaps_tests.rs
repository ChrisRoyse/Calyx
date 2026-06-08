use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{
    CALYX_KERNEL_UNGROUNDED, GroundednessReport, Kernel, RecallReport, build_kernel_pipeline,
    grounding_gaps,
};
use calyx_paths::AssocGraph;
use proptest::prelude::*;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(77),
        panel_version: 1,
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
        estimator_provenance: "test".to_string(),
        warnings: Vec::new(),
    }
}

fn four_member_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in [1, 2, 3, 4, 5, 6, 10] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(10), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(5), 1.0)
        .unwrap()
        .add_edge(cx(5), cx(10), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(6), 1.0)
        .unwrap()
        .add_edge(cx(6), cx(10), 1.0)
        .unwrap();
    builder.build()
}

fn bounded_pipeline_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in [3, 4, 6, 10] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(3), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(6), 1.0)
        .unwrap()
        .add_edge(cx(6), cx(10), 1.0)
        .unwrap();
    builder.build()
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-t03"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH33_T03_READBACK={}", path.display());
}

#[test]
fn grounding_gaps_four_member_report_names_only_unreachable() {
    let graph = four_member_graph();
    let report = grounding_gaps(
        &kernel(vec![cx(1), cx(2), cx(3), cx(4)]),
        &graph,
        &[cx(10)],
        2,
    )
    .unwrap();

    println!(
        "GROUNDING_GAPS_FOUR gaps={:?} grounded_fraction={}",
        report.gaps, report.grounded_fraction
    );
    write_readback(
        "four-member",
        "grounding-gaps-four-member.json",
        json!({ "report": report }),
    );

    assert_eq!(report.gaps, vec![cx(4)]);
    assert_eq!(report.grounded_count, 3);
    assert!((report.grounded_fraction - 0.75).abs() <= 1e-6);
    assert_eq!(report.warning, None);
}

#[test]
fn grounding_gaps_all_no_anchor_empty_and_zero_distance_edges() {
    let graph = four_member_graph();
    let all = grounding_gaps(&kernel(vec![cx(1), cx(2)]), &graph, &[cx(10)], 2).unwrap();
    let none = grounding_gaps(&kernel(vec![cx(1), cx(2)]), &graph, &[], 2).unwrap();
    let empty = grounding_gaps(&kernel(Vec::new()), &graph, &[cx(10)], 2).unwrap();
    let zero = grounding_gaps(&kernel(vec![cx(1), cx(10)]), &graph, &[cx(10)], 0).unwrap();

    println!(
        "GROUNDING_GAPS_EDGES all={:?} none={:?} empty={:?} zero={:?}",
        all, none, empty, zero
    );
    write_readback(
        "edges",
        "grounding-gaps-edges.json",
        json!({
            "all": all,
            "none": none,
            "empty": empty,
            "zero": zero,
        }),
    );

    assert_eq!(all.gaps, Vec::new());
    assert_eq!(all.grounded_fraction, 1.0);
    assert_eq!(none.gaps, vec![cx(1), cx(2)]);
    assert!(
        none.warning
            .as_deref()
            .unwrap()
            .starts_with(CALYX_KERNEL_UNGROUNDED)
    );
    assert_eq!(empty.grounded_fraction, 1.0);
    assert_eq!(empty.warning, None);
    assert_eq!(zero.gaps, vec![cx(1)]);
    assert_eq!(zero.grounded_count, 1);
}

#[test]
fn grounding_gaps_boundary_and_pipeline_use_same_gap_logic() {
    let graph = bounded_pipeline_graph();
    let member = kernel(vec![cx(3)]);
    let at_two = grounding_gaps(&member, &graph, &[cx(10)], 2).unwrap();
    let at_one = grounding_gaps(&member, &graph, &[cx(10)], 1).unwrap();

    let params = calyx_lodestar::KernelParams {
        kernel_graph: calyx_lodestar::KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 1,
            ..calyx_lodestar::KernelGraphParams::default()
        },
        ..calyx_lodestar::KernelParams::default()
    };
    let pipeline = build_kernel_pipeline(&graph, &[cx(10)], &params).unwrap();

    println!(
        "GROUNDING_GAPS_BOUNDARY at_two={:?} at_one={:?} pipeline_groundedness={:?}",
        at_two, at_one, pipeline.groundedness
    );
    write_readback(
        "boundary",
        "grounding-gaps-boundary.json",
        json!({
            "at_two": at_two,
            "at_one": at_one,
            "pipeline_max_groundedness_distance": params.kernel_graph.max_groundedness_distance,
            "pipeline_kernel": pipeline,
        }),
    );

    assert_eq!(at_two.gaps, Vec::new());
    assert_eq!(at_one.gaps, vec![cx(3)]);
    assert_eq!(pipeline.members, vec![cx(3)]);
    assert_eq!(pipeline.groundedness.unanchored_members, vec![cx(3)]);
    assert_eq!(pipeline.groundedness.reached_anchor, 0.0);
}

proptest! {
    #[test]
    fn grounding_gaps_counts_partition_members(member_count in 0u8..12, grounded_prefix in 0u8..12) {
        let grounded_prefix = grounded_prefix.min(member_count);
        let mut builder = AssocGraph::builder();
        let members: Vec<_> = (1..=member_count).map(cx).collect();
        for member in &members {
            builder.add_node(*member, 1.0).unwrap();
        }
        builder.add_node(cx(250), 1.0).unwrap();
        for member in members.iter().take(grounded_prefix as usize) {
            builder.add_edge(*member, cx(250), 1.0).unwrap();
        }
        let graph = builder.build();
        let report = grounding_gaps(&kernel(members), &graph, &[cx(250)], 1).unwrap();

        prop_assert_eq!(report.gaps.len() + report.grounded_count, report.member_count);
    }
}
