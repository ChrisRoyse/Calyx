use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::AnchorKind;
use calyx_lodestar::{
    CollectionId, KernelGraphParams, KernelParams, LodestarError, Scope, ScopeCache, TenantId,
    anchors_for_scope, build_kernel, report_all_scopes,
};
use calyx_paths::AssocGraph;
use serde_json::json;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx, ids};

fn store(temporal_ready: bool) -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=8 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(5), 1.0)
        .unwrap()
        .add_edge(cx(5), cx(6), 1.0)
        .unwrap()
        .add_edge(cx(6), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(4), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(7), cx(8), 1.0)
        .unwrap();

    let domain = AnchorKind::Label("domain".to_string());
    MemoryAssocStore::with_indexes(
        builder.build(),
        BTreeMap::from([
            (CollectionId::from("cycle-a"), ids([1, 2, 3])),
            (CollectionId::from("cycle-b"), ids([4, 5, 6])),
            (CollectionId::from("empty"), BTreeSet::new()),
        ]),
        BTreeMap::from([(domain, vec![cx(1)])]),
        temporal_ready.then(|| {
            (1..=8)
                .map(|seed| (cx(seed), 1_000_u64 + seed as u64))
                .collect()
        }),
        BTreeMap::from([(TenantId::from("tenant-a"), ids([7, 8]))]),
        BTreeMap::new(),
    )
}

fn params(panel_version: u64) -> KernelParams {
    KernelParams {
        panel_version,
        anchor_kind: None,
        corpus_shard_hash: [34; 32],
        built_at_millis: 34,
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 4,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn domain_anchor() -> AnchorKind {
    AnchorKind::Label("domain".to_string())
}

fn collection_scope() -> Scope {
    Scope::Collection {
        id: CollectionId::from("cycle-a"),
    }
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-t03"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_T03_READBACK={}", path.display());
}

#[test]
fn build_kernel_all_associations_and_collection_subset() {
    let store = store(true);
    let mut cache = ScopeCache::new(8);
    let all = build_kernel(
        &store,
        Scope::AllAssociations,
        Some(domain_anchor()),
        params(7),
        &mut cache,
    )
    .unwrap();
    let collection = build_kernel(
        &store,
        collection_scope(),
        Some(domain_anchor()),
        params(7),
        &mut cache,
    )
    .unwrap();
    let all_members: BTreeSet<_> = all.members.iter().copied().collect();
    let collection_subset = collection
        .members
        .iter()
        .all(|member| all_members.contains(member));

    println!(
        "PH34_MULTI_SCOPE_SUBSET all={} collection={} subset={collection_subset}",
        all.members.len(),
        collection.members.len()
    );
    write_readback(
        "subset",
        "ph34-multi-scope-subset-readback.json",
        json!({
            "all_members": all.members,
            "collection_members": collection.members,
            "collection_subset": collection_subset,
            "all_size_gte_collection": all.members.len() >= collection.members.len(),
        }),
    );

    assert!(!all.members.is_empty());
    assert!(!collection.members.is_empty());
    assert!(all.members.len() >= collection.members.len());
    assert!(collection_subset);
}

#[test]
fn build_kernel_reuses_scope_cache_by_scope_hash_and_panel_version() {
    let store = store(true);
    let mut cache = ScopeCache::new(2);
    let scope = collection_scope();
    build_kernel(
        &store,
        scope.clone(),
        Some(domain_anchor()),
        params(7),
        &mut cache,
    )
    .unwrap();
    build_kernel(&store, scope, Some(domain_anchor()), params(7), &mut cache).unwrap();
    let stats = cache.stats();

    println!(
        "PH34_MULTI_SCOPE_CACHE hits={} misses={} size={}",
        stats.hits, stats.misses, stats.current_size
    );
    write_readback(
        "cache",
        "ph34-multi-scope-cache-readback.json",
        json!({ "stats": stats }),
    );

    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.current_size, 1);
}

#[test]
fn report_all_scopes_reads_kernel_fields_without_recomputing() {
    let store = store(true);
    let mut cache = ScopeCache::new(8);
    let scopes = [
        Scope::AllAssociations,
        collection_scope(),
        Scope::Collection {
            id: CollectionId::from("cycle-b"),
        },
    ];
    let kernels: Vec<_> = scopes
        .iter()
        .cloned()
        .map(|scope| {
            let kernel = build_kernel(
                &store,
                scope.clone(),
                Some(domain_anchor()),
                params(7),
                &mut cache,
            )
            .unwrap();
            (scope, kernel)
        })
        .collect();
    let reports = report_all_scopes(&kernels);
    let sizes_match = reports
        .iter()
        .zip(kernels.iter())
        .all(|(report, (_, kernel))| report.kernel_size == kernel.members.len());

    println!(
        "PH34_MULTI_SCOPE_REPORTS rows={} sizes_match={sizes_match}",
        reports.len()
    );
    write_readback(
        "reports",
        "ph34-multi-scope-reports-readback.json",
        json!({ "reports": reports, "sizes_match": sizes_match }),
    );

    assert_eq!(reports.len(), 3);
    assert!(sizes_match);
}

#[test]
fn unanchored_scope_is_tagged_provisional_and_panel_bump_misses_cache() {
    let store = store(true);
    let mut cache = ScopeCache::new(4);
    let scope = collection_scope();
    let unanchored = build_kernel(&store, scope.clone(), None, params(7), &mut cache).unwrap();
    build_kernel(&store, scope, None, params(8), &mut cache).unwrap();
    let stats = cache.stats();
    let provisional = unanchored.estimator_provenance.contains("provisional")
        && unanchored
            .estimator_provenance
            .contains("CALYX_KERNEL_UNGROUNDED");

    println!(
        "PH34_MULTI_SCOPE_PROVISIONAL provisional={provisional} hits={} misses={}",
        stats.hits, stats.misses
    );
    write_readback(
        "provisional",
        "ph34-multi-scope-provisional-readback.json",
        json!({
            "provisional": provisional,
            "provenance": unanchored.estimator_provenance,
            "warnings": unanchored.warnings,
            "stats": stats,
        }),
    );

    assert!(provisional);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 2);
    assert_eq!(stats.current_size, 2);
}

#[test]
fn empty_intersect_reports_zero_and_temporal_error_does_not_fill_cache() {
    let ready = store(true);
    let mut cache = ScopeCache::new(4);
    let empty_scope = Scope::Intersect {
        left: Box::new(collection_scope()),
        right: Box::new(Scope::Collection {
            id: CollectionId::from("empty"),
        }),
    };
    let empty = build_kernel(
        &ready,
        empty_scope.clone(),
        Some(domain_anchor()),
        params(7),
        &mut cache,
    )
    .unwrap();
    let report = report_all_scopes(&[(empty_scope, empty.clone())]);

    let not_ready = store(false);
    let err = build_kernel(
        &not_ready,
        Scope::TimeWindow { t0: 1, t1: 2 },
        Some(domain_anchor()),
        params(7),
        &mut cache,
    )
    .unwrap_err();
    let stats = cache.stats();

    println!(
        "PH34_MULTI_SCOPE_EDGES empty_size={} temporal_error={} cache_size={}",
        report[0].kernel_size,
        err.code(),
        stats.current_size
    );
    write_readback(
        "edges",
        "ph34-multi-scope-edges-readback.json",
        json!({
            "empty_kernel_size": empty.members.len(),
            "empty_report_size": report[0].kernel_size,
            "temporal_error": err.code(),
            "cache_size_after_error": stats.current_size,
        }),
    );

    assert!(empty.members.is_empty());
    assert_eq!(report[0].kernel_size, 0);
    assert!(matches!(err, LodestarError::ScopeTemporalNotReady));
    assert_eq!(stats.current_size, 1);
}

#[test]
fn anchors_for_scope_filters_anchor_ids_to_materialized_scope() {
    let store = store(true);
    let collection = anchors_for_scope(&collection_scope(), &store, Some(domain_anchor())).unwrap();
    let tenant = anchors_for_scope(
        &Scope::Tenant {
            id: TenantId::from("tenant-a"),
        },
        &store,
        Some(domain_anchor()),
    )
    .unwrap();

    println!(
        "PH34_MULTI_SCOPE_ANCHORS collection={} tenant={}",
        collection.len(),
        tenant.len()
    );
    write_readback(
        "anchors",
        "ph34-multi-scope-anchors-readback.json",
        json!({ "collection_anchor_count": collection.len(), "tenant_anchor_count": tenant.len() }),
    );

    assert_eq!(collection, vec![cx(1)]);
    assert!(tenant.is_empty());
}
