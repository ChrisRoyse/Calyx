use std::collections::BTreeSet;

use calyx_lodestar::{Scope, ScopeCache};
use serde_json::json;

use super::{RealScopeStore, build_scoped, coll};

pub(super) fn union_mfvs_not_naive(store: &RealScopeStore) -> serde_json::Value {
    let mut cache = ScopeCache::new(8);
    let a = coll("mfvs_a");
    let b = coll("mfvs_b");
    let kernel_a = build_scoped(store, a.clone(), 201, &mut cache);
    let kernel_b = build_scoped(store, b.clone(), 202, &mut cache);
    let union_scope = Scope::Union {
        left: Box::new(a),
        right: Box::new(b),
    };
    let union = build_scoped(store, union_scope, 203, &mut cache);
    let naive: BTreeSet<_> = kernel_a
        .members
        .iter()
        .chain(kernel_b.members.iter())
        .copied()
        .collect();
    let union_members: BTreeSet<_> = union.members.iter().copied().collect();
    json!({
        "kernel_a": kernel_a.members,
        "kernel_b": kernel_b.members,
        "union_kernel": union.members,
        "naive_union_size": naive.len(),
        "union_kernel_size": union_members.len(),
        "mfvs_not_naive_union": union_members != naive,
    })
}
