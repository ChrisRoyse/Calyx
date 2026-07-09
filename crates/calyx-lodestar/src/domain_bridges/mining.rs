use crate::{ScopeCache, build_kernel};

use super::*;

pub fn mine_domain_bridges(
    store: &dyn AssocStore,
    pairs: &[DomainBridgeScopePair],
    params: &DomainBridgeMiningParams,
) -> Result<DomainBridgeReport> {
    validate_params(&params.ranking)?;
    if pairs.is_empty() {
        return invalid_params("at least one domain pair is required");
    }
    let graph = store.full_graph()?;
    if graph.is_empty() {
        return Err(LodestarError::KernelEmptyGraph);
    }
    let mut degree_summary = None;
    let mut inputs = Vec::new();
    let mut cache = ScopeCache::default();
    for pair in pairs {
        validate_pair(&pair.pair)?;
        let left_roots = nonempty_roots(store, &pair.left_scope, &pair.pair.left)?;
        let right_roots = nonempty_roots(store, &pair.right_scope, &pair.pair.right)?;
        let left_kernel = build_kernel(
            store,
            pair.left_scope.clone(),
            params.anchor_kind.clone(),
            params.kernel.clone(),
            &mut cache,
        )?;
        let right_kernel = build_kernel(
            store,
            pair.right_scope.clone(),
            params.anchor_kind.clone(),
            params.kernel.clone(),
            &mut cache,
        )?;
        let bridge_ids =
            bridge_members_by_frequency(&graph, &left_kernel.members, &right_kernel.members)?;
        if bridge_ids.is_empty() {
            return invalid_params(format!(
                "domain pair {} / {} produced no shared bridge members from scoped kernels",
                pair.pair.left, pair.pair.right
            ));
        }
        let left_dist = distance_from_roots(&graph, &left_roots, params.ranking.max_evidence_hops);
        let right_dist =
            distance_from_roots(&graph, &right_roots, params.ranking.max_evidence_hops);
        for cx_id in bridge_ids {
            let Some(left_hops) = left_dist.get(&cx_id).copied() else {
                continue;
            };
            let Some(right_hops) = right_dist.get(&cx_id).copied() else {
                continue;
            };
            let (degree_counts, max_degree) = degree_summary.get_or_insert_with(|| {
                let counts = degree_counts(&graph);
                let max = max_degree(&counts);
                (counts, max)
            });
            let metadata = store.node_metadata(cx_id)?.ok_or_else(|| {
                LodestarError::KernelInvalidParams {
                    detail: format!(
                        "bridge candidate {cx_id} has no graph metadata; re-run weave-loom with metadata-preserving node props"
                    ),
                }
            })?;
            let confidence = gate_confidence(
                left_kernel.groundedness.reached_anchor,
                right_kernel.groundedness.reached_anchor,
                left_hops + right_hops,
            );
            let passed = confidence >= params.ranking.min_gate_confidence;
            let (code, reason) = if passed {
                (
                    "CALYX_DOMAIN_BRIDGE_GATE_PASS",
                    "candidate is present in both scoped kernels and reachable from both scope root sets".to_string(),
                )
            } else {
                (
                    "CALYX_DOMAIN_BRIDGE_GATE_REFUSED",
                    format!(
                        "candidate confidence {confidence:.6} is below min_gate_confidence {:.6}",
                        params.ranking.min_gate_confidence
                    ),
                )
            };
            inputs.push(DomainBridgeInput {
                pair: pair.pair.clone(),
                cx_id,
                text: bridge_text(cx_id, &metadata)?,
                centrality_score: degree_score(cx_id, degree_counts, *max_degree)?,
                cross_domain_distance: Some(left_hops + right_hops),
                gate: DomainBridgeGateVerdict {
                    passed,
                    confidence,
                    code: code.to_string(),
                    reason,
                    evidence: vec![
                        format!("left_kernel_id={}", left_kernel.kernel_id),
                        format!(
                            "left_groundedness={:.6}",
                            left_kernel.groundedness.reached_anchor
                        ),
                        format!("left_hops={left_hops}"),
                        format!("right_kernel_id={}", right_kernel.kernel_id),
                        format!(
                            "right_groundedness={:.6}",
                            right_kernel.groundedness.reached_anchor
                        ),
                        format!("right_hops={right_hops}"),
                    ],
                },
                provenance: bridge_provenance(cx_id, &metadata),
            });
        }
    }
    let report = rank_domain_bridges(&graph, &inputs, &params.ranking)?;
    for pair in pairs {
        let Some(pair_report) = report
            .pair_reports
            .iter()
            .find(|found| found.pair == pair.pair)
        else {
            return invalid_params(format!(
                "domain pair {} / {} produced no sufficiency-checkable bridge candidates",
                pair.pair.left, pair.pair.right
            ));
        };
        if pair_report.candidate_count == 0 {
            return invalid_params(format!(
                "domain pair {} / {} had only refused bridge candidates",
                pair.pair.left, pair.pair.right
            ));
        }
    }
    Ok(report)
}
