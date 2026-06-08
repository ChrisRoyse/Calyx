use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{Kernel, Result, groundedness_distance};

pub const CALYX_KERNEL_UNGROUNDED: &str = "CALYX_KERNEL_UNGROUNDED";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundingGapReport {
    pub gaps: Vec<CxId>,
    pub grounded_fraction: f32,
    pub grounded_count: usize,
    pub member_count: usize,
    pub max_anchor_dist: usize,
    pub warning: Option<String>,
}

pub fn grounding_gaps(
    kernel: &Kernel,
    graph: &AssocGraph,
    anchors: &[CxId],
    max_anchor_dist: usize,
) -> Result<GroundingGapReport> {
    grounding_gaps_for_members(&kernel.members, graph, anchors, max_anchor_dist)
}

pub(crate) fn grounding_gaps_for_members(
    members: &[CxId],
    graph: &AssocGraph,
    anchors: &[CxId],
    max_anchor_dist: usize,
) -> Result<GroundingGapReport> {
    let mut gaps = Vec::new();
    for member in members {
        let grounded =
            anchored_within_distance(graph, *member, anchors, max_anchor_dist)?.is_some();
        if !grounded {
            gaps.push(*member);
        }
    }
    gaps.sort();
    let grounded_count = members.len().saturating_sub(gaps.len());
    let grounded_fraction = if members.is_empty() {
        1.0
    } else {
        grounded_count as f32 / members.len() as f32
    };
    let warning = (grounded_fraction == 0.0 && !members.is_empty())
        .then(|| format!("{CALYX_KERNEL_UNGROUNDED}: all kernel members are provisional"));
    Ok(GroundingGapReport {
        gaps,
        grounded_fraction,
        grounded_count,
        member_count: members.len(),
        max_anchor_dist,
        warning,
    })
}

fn anchored_within_distance(
    graph: &AssocGraph,
    member: CxId,
    anchors: &[CxId],
    max_anchor_dist: usize,
) -> Result<Option<usize>> {
    if anchors.is_empty() {
        return Ok(None);
    }
    groundedness_distance(graph, member, anchors, max_anchor_dist)
}
