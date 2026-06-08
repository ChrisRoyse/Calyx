use std::collections::BTreeSet;

use calyx_core::{AnchorKind, CxId};
use calyx_paths::AssocGraph;

use crate::grounding_gaps::CALYX_KERNEL_UNGROUNDED;
use crate::{
    AnswerPath, AssocStore, Kernel, KernelIndex, KernelParams, Result, Scope, ScopeCache,
    ScopeCacheKey, build_kernel_pipeline, kernel_answer, materialize_scope,
    scope_cache_anchor_identity, scope_hash,
};

const UNGROUNDED_EPSILON: f32 = 0.01;

pub fn build_kernel(
    store: &dyn AssocStore,
    scope: Scope,
    anchor_kind: Option<AnchorKind>,
    params: KernelParams,
    cache: &mut ScopeCache,
) -> Result<Kernel> {
    // IMPORTANT: Union kernel != members_a ∪ members_b. Union scopes materialize
    // a graph here, then run the same MFVS pipeline as every other scope.
    let graph = materialize_scope(&scope, store)?;
    let anchor_kinds = anchor_kinds_for_scope(&scope, anchor_kind.as_ref());
    let anchors = anchors_for_graph(&graph, store, &anchor_kinds)?;
    let key = ScopeCacheKey::new(
        scope_hash(&scope),
        params.panel_version,
        scope_cache_anchor_identity(&anchor_kinds, &anchors),
        params.corpus_shard_hash,
    );
    if let Some(kernel) = cache.get(&key) {
        return Ok(kernel.clone());
    }

    let mut scoped_params = params;
    if let Some(kind) = anchor_kind.or_else(|| anchor_kinds.first().cloned()) {
        scoped_params.anchor_kind = Some(anchor_kind_name(&kind));
    }

    let mut kernel = build_kernel_pipeline(&graph, &anchors, &scoped_params)?;
    mark_ungrounded_scope(&mut kernel);
    cache.insert(key, kernel.clone());
    Ok(kernel)
}

pub fn bridges(
    store: &dyn AssocStore,
    scope_a: Scope,
    scope_b: Scope,
    anchor_kind: Option<AnchorKind>,
    params: KernelParams,
    cache: &mut ScopeCache,
) -> Result<Vec<CxId>> {
    let kernel_a = build_kernel(store, scope_a, anchor_kind.clone(), params.clone(), cache)?;
    let kernel_b = build_kernel(store, scope_b, anchor_kind, params, cache)?;
    bridge_members_by_frequency(store, &kernel_a, &kernel_b)
}

pub fn kernel_answer_scoped(
    kernel_index: &KernelIndex,
    store: &dyn AssocStore,
    query_cx: CxId,
    query_vec: &[f32],
    scope: &Scope,
    anchored_kernel_nodes: &[CxId],
    max_hops: usize,
) -> Result<AnswerPath> {
    let scoped_graph = materialize_scope(scope, store)?;
    kernel_answer(
        kernel_index,
        &scoped_graph,
        query_cx,
        query_vec,
        anchored_kernel_nodes,
        max_hops,
    )
}

pub fn anchors_for_scope(
    scope: &Scope,
    store: &dyn AssocStore,
    anchor_kind: Option<AnchorKind>,
) -> Result<Vec<CxId>> {
    let graph = materialize_scope(scope, store)?;
    let anchor_kinds = anchor_kinds_for_scope(scope, anchor_kind.as_ref());
    anchors_for_graph(&graph, store, &anchor_kinds)
}

fn bridge_members_by_frequency(
    store: &dyn AssocStore,
    left: &Kernel,
    right: &Kernel,
) -> Result<Vec<CxId>> {
    let graph = store.full_graph()?;
    let right_members: BTreeSet<_> = right.members.iter().copied().collect();
    let mut weighted = left
        .members
        .iter()
        .copied()
        .filter(|id| right_members.contains(id))
        .map(|id| Ok((id, graph.node_weight(id)?)))
        .collect::<Result<Vec<_>>>()?;
    weighted.sort_by(|(left_id, left_weight), (right_id, right_weight)| {
        right_weight
            .total_cmp(left_weight)
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(weighted.into_iter().map(|(id, _)| id).collect())
}

fn anchors_for_graph(
    graph: &AssocGraph,
    store: &dyn AssocStore,
    anchor_kinds: &[AnchorKind],
) -> Result<Vec<CxId>> {
    let mut anchors = BTreeSet::new();
    for kind in anchor_kinds {
        for anchor in store.domain_anchors(kind)? {
            if graph.node_index(anchor).is_some() {
                anchors.insert(anchor);
            }
        }
    }
    Ok(anchors.into_iter().collect())
}

fn anchor_kinds_for_scope(scope: &Scope, explicit: Option<&AnchorKind>) -> Vec<AnchorKind> {
    if let Some(kind) = explicit {
        return vec![kind.clone()];
    }
    let mut kinds = BTreeSet::new();
    collect_domain_anchor_kinds(scope, &mut kinds);
    kinds.into_iter().collect()
}

fn collect_domain_anchor_kinds(scope: &Scope, kinds: &mut BTreeSet<AnchorKind>) {
    match scope {
        Scope::Domain { anchor_kind } => {
            kinds.insert(anchor_kind.clone());
        }
        Scope::Union { left, right } | Scope::Intersect { left, right } => {
            collect_domain_anchor_kinds(left, kinds);
            collect_domain_anchor_kinds(right, kinds);
        }
        _ => {}
    }
}

fn mark_ungrounded_scope(kernel: &mut Kernel) {
    if kernel.groundedness.reached_anchor >= UNGROUNDED_EPSILON {
        return;
    }
    if !kernel
        .warnings
        .iter()
        .any(|warning| warning.starts_with(CALYX_KERNEL_UNGROUNDED))
    {
        kernel.warnings.push(format!(
            "{CALYX_KERNEL_UNGROUNDED}: scoped kernel is provisional"
        ));
    }
    if !kernel
        .estimator_provenance
        .contains(CALYX_KERNEL_UNGROUNDED)
    {
        kernel
            .estimator_provenance
            .push_str(&format!("; {CALYX_KERNEL_UNGROUNDED}"));
    }
    if !kernel.estimator_provenance.contains("provisional") {
        kernel.estimator_provenance.push_str("; trust=provisional");
    }
}

fn anchor_kind_name(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::Label(value) => format!("label:{value}"),
        other => format!("{other:?}"),
    }
}
