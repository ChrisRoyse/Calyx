use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{AnchorKind, CxId};
use calyx_lodestar::{
    AssocStore, CollectionId, GroundednessReport, Kernel, KernelGraphParams, KernelParams,
    RecallReport, Scope, ScopeCache, bridges, build_kernel, build_kernel_index, kernel_answer,
    kernel_answer_scoped, report_all_scopes,
};
use calyx_paths::AssocGraph;
use serde_json::json;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx, ids};

fn bridge_store() -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=13 {
        let weight = match seed {
            2 => 9.0,
            1 => 7.0,
            _ => 1.0,
        };
        builder.add_node(cx(seed), weight).unwrap();
    }
    add_cycle(&mut builder, [1, 3, 4]);
    add_cycle(&mut builder, [2, 5, 6]);
    add_cycle(&mut builder, [1, 7, 8]);
    add_cycle(&mut builder, [2, 9, 10]);
    add_cycle(&mut builder, [11, 12, 13]);

    MemoryAssocStore::with_scope_data(
        builder.build(),
        BTreeMap::from([
            (CollectionId::from("a"), ids([1, 2, 3, 4, 5, 6])),
            (CollectionId::from("b"), ids([1, 2, 7, 8, 9, 10])),
            (CollectionId::from("disjoint"), ids([11, 12, 13])),
            (CollectionId::from("empty"), BTreeSet::new()),
        ]),
        BTreeMap::from([(domain_anchor(), vec![cx(1), cx(2), cx(11)])]),
    )
}

fn naive_union_store() -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=4 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    add_cycle(&mut builder, [1, 2, 3]);
    add_cycle(&mut builder, [2, 3, 4]);
    MemoryAssocStore::with_scope_data(
        builder.build(),
        BTreeMap::from([
            (CollectionId::from("a"), ids([1, 2, 3])),
            (CollectionId::from("b"), ids([2, 3, 4])),
        ]),
        BTreeMap::from([(domain_anchor(), vec![cx(1), cx(2)])]),
    )
}

fn scoped_answer_store() -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in [1, 2, 3, 9] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(9), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(9), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(1), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(2), 1.0)
        .unwrap();
    MemoryAssocStore::with_scope_data(
        builder.build(),
        BTreeMap::new(),
        BTreeMap::from([(domain_anchor(), vec![cx(1)])]),
    )
}

fn add_cycle(builder: &mut calyx_paths::AssocGraphBuilder, cycle: [u8; 3]) {
    builder
        .add_edge(cx(cycle[0]), cx(cycle[1]), 1.0)
        .unwrap()
        .add_edge(cx(cycle[1]), cx(cycle[2]), 1.0)
        .unwrap()
        .add_edge(cx(cycle[2]), cx(cycle[0]), 1.0)
        .unwrap();
}

fn coll(name: &str) -> Scope {
    Scope::Collection {
        id: CollectionId::from(name),
    }
}

fn union(left: Scope, right: Scope) -> Scope {
    Scope::Union {
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn domain_anchor() -> AnchorKind {
    AnchorKind::Label("domain".to_string())
}

fn params(panel_version: u64) -> KernelParams {
    KernelParams {
        panel_version,
        anchor_kind: Some("label:domain".to_string()),
        corpus_shard_hash: [55; 32],
        built_at_millis: 55,
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 4,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(88),
        panel_version: 1,
        anchor_kind: Some("synthetic_anchor".to_string()),
        corpus_shard_hash: [8; 32],
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

fn embeddings() -> BTreeMap<CxId, Vec<f32>> {
    BTreeMap::from([(cx(1), vec![1.0, 0.0])])
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-t05"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_T05_READBACK={}", path.display());
}

#[test]
fn bridges_shared_members_are_sorted_by_frequency() {
    let store = bridge_store();
    let mut cache = ScopeCache::new(8);
    let result = bridges(
        &store,
        coll("a"),
        coll("b"),
        Some(domain_anchor()),
        params(21),
        &mut cache,
    )
    .unwrap();

    println!("PH34_BRIDGES_SHARED bridges={result:?}");
    write_readback(
        "bridges",
        "ph34-bridges-shared-readback.json",
        json!({ "bridges": result, "expected_order": [cx(2), cx(1)] }),
    );

    assert_eq!(result, vec![cx(2), cx(1)]);
}

#[test]
fn bridges_disjoint_and_empty_scopes_are_empty() {
    let store = bridge_store();
    let mut cache = ScopeCache::new(8);
    let disjoint = bridges(
        &store,
        coll("a"),
        coll("disjoint"),
        Some(domain_anchor()),
        params(22),
        &mut cache,
    )
    .unwrap();
    let empty = bridges(
        &store,
        coll("empty"),
        coll("empty"),
        Some(domain_anchor()),
        params(23),
        &mut cache,
    )
    .unwrap();

    println!(
        "PH34_BRIDGES_EMPTY disjoint={} empty={}",
        disjoint.len(),
        empty.len()
    );
    write_readback(
        "empty",
        "ph34-bridges-empty-readback.json",
        json!({ "disjoint": disjoint, "empty": empty }),
    );

    assert!(disjoint.is_empty());
    assert!(empty.is_empty());
}

#[test]
fn kernel_answer_scoped_blocks_out_of_scope_path_and_no_anchor() {
    let store = scoped_answer_store();
    let graph = store.full_graph().unwrap();
    let index = build_kernel_index(&kernel(vec![cx(1)]), &embeddings()).unwrap();
    let full = kernel_answer(&index, &graph, cx(2), &[1.0, 0.0], &[cx(1)], 3).unwrap();
    let scope = Scope::Subgraph {
        query: cx(9),
        radius: 1,
    };
    let scoped =
        kernel_answer_scoped(&index, &store, cx(2), &[1.0, 0.0], &scope, &[cx(1)], 3).unwrap_err();
    let no_anchor =
        kernel_answer_scoped(&index, &store, cx(2), &[1.0, 0.0], &scope, &[], 3).unwrap_err();

    println!(
        "PH34_SCOPED_ANSWER full_hops={} scoped_error={} no_anchor={}",
        full.hops.len(),
        scoped.code(),
        no_anchor.code()
    );
    write_readback(
        "answer",
        "ph34-scoped-answer-readback.json",
        json!({
            "full_hops": full.hops.len(),
            "scoped_error": scoped.code(),
            "no_anchor": no_anchor.code(),
        }),
    );

    assert_eq!(full.hops.len(), 2);
    assert_eq!(scoped.code(), "CALYX_KERNEL_ANSWER_NO_PATH");
    assert_eq!(no_anchor.code(), "CALYX_KERNEL_NO_ANCHORED_NODE");
}

#[test]
fn scope_reports_count_union_bridges() {
    let store = bridge_store();
    let mut cache = ScopeCache::new(8);
    let scope_a = coll("a");
    let scope_b = coll("b");
    let scope_union = union(scope_a.clone(), scope_b.clone());
    let kernels = vec![
        (
            scope_a.clone(),
            build_kernel(
                &store,
                scope_a,
                Some(domain_anchor()),
                params(24),
                &mut cache,
            )
            .unwrap(),
        ),
        (
            scope_b.clone(),
            build_kernel(
                &store,
                scope_b,
                Some(domain_anchor()),
                params(24),
                &mut cache,
            )
            .unwrap(),
        ),
        (
            scope_union.clone(),
            build_kernel(
                &store,
                scope_union,
                Some(domain_anchor()),
                params(24),
                &mut cache,
            )
            .unwrap(),
        ),
    ];
    let reports = report_all_scopes(&kernels);
    let union_report = reports.last().expect("union report");

    println!(
        "PH34_BRIDGE_REPORT rows={} union_bridge_count={}",
        reports.len(),
        union_report.bridge_count
    );
    write_readback(
        "reports",
        "ph34-bridge-report-readback.json",
        json!({ "reports": reports, "union_bridge_count": union_report.bridge_count }),
    );

    assert_eq!(union_report.bridge_count, 2);
}

#[test]
fn bridges_all_associations_returns_all_kernel_members() {
    let store = bridge_store();
    let mut cache = ScopeCache::new(8);
    let all_kernel = build_kernel(
        &store,
        Scope::AllAssociations,
        Some(domain_anchor()),
        params(25),
        &mut cache,
    )
    .unwrap();
    let all_bridges = bridges(
        &store,
        Scope::AllAssociations,
        Scope::AllAssociations,
        Some(domain_anchor()),
        params(25),
        &mut cache,
    )
    .unwrap();
    let kernel_set: BTreeSet<_> = all_kernel.members.iter().copied().collect();
    let bridge_set: BTreeSet<_> = all_bridges.iter().copied().collect();

    println!(
        "PH34_BRIDGES_ALL kernel={} bridges={}",
        all_kernel.members.len(),
        all_bridges.len()
    );
    write_readback(
        "all",
        "ph34-bridges-all-readback.json",
        json!({
            "kernel_members": all_kernel.members,
            "bridges": all_bridges,
            "same_members": kernel_set == bridge_set,
        }),
    );

    assert_eq!(kernel_set, bridge_set);
    assert_eq!(all_bridges.first().copied(), Some(cx(2)));
}

#[test]
fn union_kernel_runs_mfvs_not_naive_member_union() {
    let store = naive_union_store();
    let mut cache = ScopeCache::new(8);
    let scope_a = coll("a");
    let scope_b = coll("b");
    let kernel_a = build_kernel(
        &store,
        scope_a.clone(),
        Some(domain_anchor()),
        params(26),
        &mut cache,
    )
    .unwrap();
    let kernel_b = build_kernel(
        &store,
        scope_b.clone(),
        Some(domain_anchor()),
        params(26),
        &mut cache,
    )
    .unwrap();
    let union_kernel = build_kernel(
        &store,
        union(scope_a, scope_b),
        Some(domain_anchor()),
        params(26),
        &mut cache,
    )
    .unwrap();
    let naive: BTreeSet<_> = kernel_a
        .members
        .iter()
        .chain(kernel_b.members.iter())
        .copied()
        .collect();
    let union_members: BTreeSet<_> = union_kernel.members.iter().copied().collect();

    println!(
        "PH34_UNION_MFVS kernel_a={:?} kernel_b={:?} union={:?}",
        kernel_a.members, kernel_b.members, union_kernel.members
    );
    write_readback(
        "union",
        "ph34-union-mfvs-readback.json",
        json!({
            "kernel_a": kernel_a.members,
            "kernel_b": kernel_b.members,
            "naive_union_size": naive.len(),
            "union_kernel": union_kernel.members,
            "mfvs_not_naive_union": union_members != naive,
        }),
    );

    assert_eq!(kernel_a.members, vec![cx(1)]);
    assert_eq!(kernel_b.members, vec![cx(2)]);
    assert_eq!(union_kernel.members, vec![cx(2)]);
    assert_ne!(union_members, naive);
}
