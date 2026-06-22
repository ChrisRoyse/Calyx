use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::AnchorKind;
use calyx_lodestar::{
    CollectionId, FilterExpr, LodestarError, Scope, TenantId, materialize_scope, scope_hash,
};
use calyx_paths::AssocGraph;
use serde_json::json;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx, ids};

const EXPECTED_ALL_HASH: &str = "9bcc9eef3da72eaed03ea54c2b0086368d119cf274516e1fb6706aaf487fe7d5";

fn store(temporal_ready: bool) -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=10 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    for seed in 1..10 {
        builder.add_edge(cx(seed), cx(seed + 1), 1.0).unwrap();
    }
    let c1 = CollectionId::from("c1");
    let c2 = CollectionId::from("c2");
    let tenant = TenantId::from("tenant-a");
    let filter = FilterExpr::Named {
        name: "even".to_string(),
    };
    MemoryAssocStore::with_indexes(
        builder.build(),
        BTreeMap::from([(c1, ids([1, 2, 3, 4])), (c2, ids([4, 5, 6]))]),
        BTreeMap::from([(AnchorKind::Label("domain".to_string()), vec![cx(1)])]),
        temporal_ready.then(|| {
            (1..=10)
                .map(|seed| (cx(seed), 1_000_u64 + seed as u64))
                .collect()
        }),
        BTreeMap::from([(tenant, ids([7, 8]))]),
        BTreeMap::from([(filter, ids([2, 4, 6, 8, 10]))]),
    )
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-t01"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_T01_READBACK={}", path.display());
}

#[test]
fn scope_hash_all_associations_is_fixed_and_stable() {
    let scope = Scope::AllAssociations;
    let first = scope_hash(&scope);
    let second = scope_hash(&scope);
    let hex = hex32(&first);

    println!("PH34_SCOPE_HASH all_associations={hex}");
    write_readback(
        "hash",
        "ph34-scope-hash-readback.json",
        json!({ "all_associations_hash": hex, "stable": first == second }),
    );

    assert_eq!(first, second);
    assert_eq!(hex, EXPECTED_ALL_HASH);
}

#[test]
fn materialize_collection_union_intersect_counts() {
    let store = store(true);
    let c1 = Scope::Collection {
        id: CollectionId::from("c1"),
    };
    let c2 = Scope::Collection {
        id: CollectionId::from("c2"),
    };
    let collection = materialize_scope(&c1, &store).unwrap();
    let union = materialize_scope(
        &Scope::Union {
            left: Box::new(c1.clone()),
            right: Box::new(c2.clone()),
        },
        &store,
    )
    .unwrap();
    let intersect = materialize_scope(
        &Scope::Intersect {
            left: Box::new(c1),
            right: Box::new(c2),
        },
        &store,
    )
    .unwrap();

    println!(
        "PH34_SCOPE_COUNTS collection={} union={} intersect={}",
        collection.node_count(),
        union.node_count(),
        intersect.node_count()
    );
    write_readback(
        "counts",
        "ph34-scope-counts-readback.json",
        json!({
            "collection_nodes": collection.node_count(),
            "union_nodes": union.node_count(),
            "intersect_nodes": intersect.node_count(),
        }),
    );

    assert_eq!(collection.node_count(), 4);
    assert_eq!(union.node_count(), 6);
    assert_eq!(intersect.node_count(), 1);
}

#[test]
fn materialize_all_domain_subgraph_time_tenant_filter() {
    let store = store(true);
    let all = materialize_scope(&Scope::AllAssociations, &store).unwrap();
    let domain = materialize_scope(
        &Scope::Domain {
            anchor_kind: AnchorKind::Label("domain".to_string()),
        },
        &store,
    )
    .unwrap();
    let subgraph = materialize_scope(
        &Scope::Subgraph {
            query: cx(1),
            radius: 2,
        },
        &store,
    )
    .unwrap();
    let time = materialize_scope(
        &Scope::TimeWindow {
            t0: 1_003,
            t1: 1_005,
        },
        &store,
    )
    .unwrap();
    let tenant = materialize_scope(
        &Scope::Tenant {
            id: TenantId::from("tenant-a"),
        },
        &store,
    )
    .unwrap();
    let filter = materialize_scope(
        &Scope::Filter {
            expr: FilterExpr::Named {
                name: "even".to_string(),
            },
        },
        &store,
    )
    .unwrap();

    println!(
        "PH34_SCOPE_VARIANTS all={} domain={} subgraph={} time={} tenant={} filter={}",
        all.node_count(),
        domain.node_count(),
        subgraph.node_count(),
        time.node_count(),
        tenant.node_count(),
        filter.node_count()
    );
    write_readback(
        "variants",
        "ph34-scope-variants-readback.json",
        json!({
            "all": all.node_count(),
            "domain": domain.node_count(),
            "subgraph": subgraph.node_count(),
            "time_window": time.node_count(),
            "tenant": tenant.node_count(),
            "filter": filter.node_count(),
        }),
    );

    assert_eq!(all.node_count(), 10);
    assert_eq!(domain.node_count(), 10);
    assert_eq!(
        subgraph.node_ids().collect::<Vec<_>>(),
        vec![cx(1), cx(2), cx(3)]
    );
    assert_eq!(time.node_count(), 3);
    assert_eq!(tenant.node_count(), 2);
    assert_eq!(filter.node_count(), 5);
}

#[test]
fn scope_fail_closed_edges_report_catalog_codes() {
    let ready = store(true);
    let not_ready = store(false);
    let unknown_collection = materialize_scope(
        &Scope::Collection {
            id: CollectionId::from("missing"),
        },
        &ready,
    )
    .unwrap_err();
    let temporal = materialize_scope(&Scope::TimeWindow { t0: 0, t1: 1 }, &not_ready).unwrap_err();
    let deep = materialize_scope(&nested_union(6), &ready).unwrap_err();
    let tenant = materialize_scope(
        &Scope::Tenant {
            id: TenantId::from("missing"),
        },
        &ready,
    )
    .unwrap_err();

    println!(
        "PH34_SCOPE_ERRORS collection={} temporal={} depth={} tenant={}",
        unknown_collection.code(),
        temporal.code(),
        deep.code(),
        tenant.code()
    );
    write_readback(
        "edges",
        "ph34-scope-edges-readback.json",
        json!({
            "unknown_collection": unknown_collection.code(),
            "temporal_not_ready": temporal.code(),
            "depth_exceeded": deep.code(),
            "unknown_tenant": tenant.code(),
        }),
    );

    assert!(matches!(
        unknown_collection,
        LodestarError::CollectionNotFound { .. }
    ));
    assert!(matches!(temporal, LodestarError::ScopeTemporalNotReady));
    assert!(matches!(deep, LodestarError::ScopeDepthExceeded { .. }));
    assert!(matches!(tenant, LodestarError::ScopeTenantNotFound { .. }));
}

fn nested_union(levels: usize) -> Scope {
    (0..levels).fold(Scope::AllAssociations, |left, _| Scope::Union {
        left: Box::new(left),
        right: Box::new(Scope::AllAssociations),
    })
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
