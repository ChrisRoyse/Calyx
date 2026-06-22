use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{AnchorKind, CxId, Ts};
use calyx_lodestar::{
    AssocStore, CollectionId, FilterExpr, KernelGraphParams, KernelParams, Scope, ScopeCache,
    TenantId, build_kernel,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[derive(Clone)]
struct IdentityStore {
    graph: AssocGraph,
    collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
    anchors: BTreeMap<AnchorKind, Vec<CxId>>,
}

impl AssocStore for IdentityStore {
    fn full_graph(&self) -> calyx_lodestar::Result<AssocGraph> {
        Ok(self.graph.clone())
    }

    fn collection_nodes(
        &self,
        id: &CollectionId,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(self.collections.get(id).cloned())
    }

    fn domain_anchors(&self, kind: &AnchorKind) -> calyx_lodestar::Result<Vec<CxId>> {
        Ok(self.anchors.get(kind).cloned().unwrap_or_default())
    }

    fn time_window_nodes(
        &self,
        _t0: Ts,
        _t1: Ts,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn tenant_nodes(&self, _id: &TenantId) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn filter_nodes(&self, _expr: &FilterExpr) -> calyx_lodestar::Result<BTreeSet<CxId>> {
        Ok(BTreeSet::new())
    }
}

fn store() -> IdentityStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=4 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(4), 1.0)
        .unwrap();

    IdentityStore {
        graph: builder.build(),
        collections: BTreeMap::from([(CollectionId::from("scope-cache-id"), ids([1, 2, 3, 4]))]),
        anchors: BTreeMap::from([(domain_anchor(), vec![cx(1)]), (alt_anchor(), vec![cx(2)])]),
    }
}

fn ids<const N: usize>(values: [u8; N]) -> BTreeSet<CxId> {
    values.into_iter().map(cx).collect()
}

fn params(corpus_seed: u8) -> KernelParams {
    KernelParams {
        panel_version: 7,
        anchor_kind: None,
        corpus_shard_hash: [corpus_seed; 32],
        built_at_millis: 328,
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

fn alt_anchor() -> AnchorKind {
    AnchorKind::Label("alt-domain".to_string())
}

fn collection_scope() -> Scope {
    Scope::Collection {
        id: CollectionId::from("scope-cache-id"),
    }
}

#[test]
fn build_kernel_cache_identity_includes_anchor_and_corpus() {
    let store = store();
    let scope = collection_scope();
    let mut cache = ScopeCache::new(8);

    let domain_first = build_kernel(
        &store,
        scope.clone(),
        Some(domain_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    assert_eq!(cache.stats().misses, 1);

    let domain_hit = build_kernel(
        &store,
        scope.clone(),
        Some(domain_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    assert_eq!(domain_first, domain_hit);
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().current_size, 1);

    let alt_first = build_kernel(
        &store,
        scope.clone(),
        Some(alt_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    assert_eq!(cache.stats().misses, 2);
    assert_eq!(cache.stats().current_size, 2);

    let alt_hit = build_kernel(
        &store,
        scope.clone(),
        Some(alt_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    assert_eq!(alt_first, alt_hit);
    assert_eq!(cache.stats().hits, 2);

    let changed_corpus =
        build_kernel(&store, scope, Some(alt_anchor()), params(35), &mut cache).unwrap();
    let stats = cache.stats();

    assert_ne!(
        alt_first.corpus_shard_hash,
        changed_corpus.corpus_shard_hash
    );
    assert_eq!(stats.hits, 2);
    assert_eq!(stats.misses, 3);
    assert_eq!(stats.current_size, 3);
}

#[test]
#[ignore = "manual FSV writes issue #328 source-of-truth readback bytes"]
fn ph34_scope_cache_identity_manual_fsv() {
    let store = store();
    let scope = collection_scope();
    let mut cache = ScopeCache::new(8);
    let before = cache.stats();

    let domain_first = build_kernel(
        &store,
        scope.clone(),
        Some(domain_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    let after_domain_miss = cache.stats();
    let domain_hit = build_kernel(
        &store,
        scope.clone(),
        Some(domain_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    let after_domain_hit = cache.stats();
    let alt_first = build_kernel(
        &store,
        scope.clone(),
        Some(alt_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    let after_alt_miss = cache.stats();
    let alt_hit = build_kernel(
        &store,
        scope.clone(),
        Some(alt_anchor()),
        params(34),
        &mut cache,
    )
    .unwrap();
    let after_alt_hit = cache.stats();
    let changed_corpus =
        build_kernel(&store, scope, Some(alt_anchor()), params(35), &mut cache).unwrap();
    let final_stats = cache.stats();

    assert_eq!(domain_first, domain_hit);
    assert_eq!(alt_first, alt_hit);
    assert_eq!(final_stats.hits, 2);
    assert_eq!(final_stats.misses, 3);
    assert_eq!(final_stats.current_size, 3);

    write_readback(json!({
        "issue": 328,
        "trigger": "build_kernel real path via ScopeCache",
        "scope": "Collection(scope-cache-id)",
        "expected_final_stats": { "hits": 2, "misses": 3, "current_size": 3 },
        "runs": [
            { "label": "before", "stats": before },
            { "label": "domain corpus34 miss", "stats": after_domain_miss,
              "kernel_id": domain_first.kernel_id, "anchor_kind": domain_first.anchor_kind,
              "corpus_shard_hash": domain_first.corpus_shard_hash },
            { "label": "domain corpus34 hit", "stats": after_domain_hit,
              "kernel_id": domain_hit.kernel_id, "same_kernel_as_miss": domain_hit == domain_first },
            { "label": "alt corpus34 miss", "stats": after_alt_miss,
              "kernel_id": alt_first.kernel_id, "anchor_kind": alt_first.anchor_kind,
              "corpus_shard_hash": alt_first.corpus_shard_hash },
            { "label": "alt corpus34 hit", "stats": after_alt_hit,
              "kernel_id": alt_hit.kernel_id, "same_kernel_as_miss": alt_hit == alt_first },
            { "label": "alt corpus35 miss", "stats": final_stats,
              "kernel_id": changed_corpus.kernel_id,
              "corpus_shard_hash": changed_corpus.corpus_shard_hash }
        ]
    }));

    println!(
        "scope cache identity OK: hits={} misses={} size={}",
        final_stats.hits, final_stats.misses, final_stats.current_size
    );
}

fn write_readback(value: serde_json::Value) {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join("ph34-scope-cache-identity-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_SCOPE_CACHE_IDENTITY_READBACK={}", path.display());
}

fn fsv_root() -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue328-scope-cache-identity"));
    base.join("scope-cache-identity")
}
