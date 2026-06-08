use std::collections::BTreeSet;

use calyx_core::{AnchorKind, CxId};
use calyx_paths::AssocGraph;

use crate::grounding_gaps::CALYX_KERNEL_UNGROUNDED;
use crate::{
    AssocStore, Kernel, KernelParams, Result, Scope, ScopeCache, ScopeCacheKey,
    build_kernel_pipeline, materialize_scope, scope_hash,
};

const UNGROUNDED_EPSILON: f32 = 0.01;

pub fn build_kernel(
    store: &dyn AssocStore,
    scope: Scope,
    anchor_kind: Option<AnchorKind>,
    params: KernelParams,
    cache: &mut ScopeCache,
) -> Result<Kernel> {
    let key = ScopeCacheKey {
        scope_hash: scope_hash(&scope),
        panel_version: params.panel_version,
    };
    if let Some(kernel) = cache.get(&key) {
        return Ok(kernel.clone());
    }

    let graph = materialize_scope(&scope, store)?;
    let anchor_kinds = anchor_kinds_for_scope(&scope, anchor_kind.as_ref());
    let anchors = anchors_for_graph(&graph, store, &anchor_kinds)?;
    let mut scoped_params = params;
    if let Some(kind) = anchor_kind.or_else(|| anchor_kinds.first().cloned()) {
        scoped_params.anchor_kind = Some(anchor_kind_name(&kind));
    }

    let mut kernel = build_kernel_pipeline(&graph, &anchors, &scoped_params)?;
    mark_ungrounded_scope(&mut kernel);
    cache.insert(key, kernel.clone());
    Ok(kernel)
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
