//! Hierarchical skill discovery and skill-scoped search (PRD 10 §4).
//!
//! `skills()` clusters the engine's constellations with deterministic
//! HDBSCAN* over fused per-lens cosine distance and returns the condensed
//! hierarchy as a named skill tree. Names are content-addressed (blake3 of
//! the sorted member ids) so the same vault state always yields the same
//! tree bytes. `search_skill()` runs a normal engine search with an exact
//! recall window and keeps only hits inside the named skill.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, Result, SlotId, content_address};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED,
    CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP, CALYX_SEXTANT_SKILL_UNKNOWN, sextant_error,
};
use crate::hit::Hit;
use crate::navigation::consensus::{dense_cosine, dense_vectors};
use crate::navigation::hdbscan::{DistanceMatrix, condensed_tree};
use crate::query::Query;
use crate::search::SearchEngine;

/// Deterministic clustering parameters for skill discovery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillParams {
    /// Smallest group that counts as a skill (HDBSCAN* min_cluster_size, >= 2).
    pub min_cluster_size: usize,
    /// Core-distance neighbor count (HDBSCAN* min_samples, >= 1).
    pub min_samples: usize,
    /// Hard O(n²) budget; more constellations than this fails closed.
    pub max_constellations: usize,
    /// Restrict clustering to these lenses (default: all active slots).
    pub slots: Option<Vec<SlotId>>,
    /// Allow the root itself to be the single selected skill.
    pub allow_single_cluster: bool,
}

impl Default for SkillParams {
    fn default() -> Self {
        Self {
            min_cluster_size: 3,
            min_samples: 3,
            max_constellations: 2048,
            slots: None,
            allow_single_cluster: false,
        }
    }
}

/// One node of the skill hierarchy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillNode {
    pub name: String,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub members: Vec<CxId>,
    pub size: usize,
    pub depth: usize,
    pub lambda_birth: f64,
    pub stability: f64,
    /// True for the flat skill set chosen by excess-of-mass selection.
    pub selected: bool,
}

/// The condensed skill hierarchy for a vault snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillTree {
    pub root: Option<String>,
    pub nodes: BTreeMap<String, SkillNode>,
    /// Names of the selected (flat) skills, sorted.
    pub selected: Vec<String>,
    /// Constellations not inside any selected skill.
    pub noise: Vec<CxId>,
    pub params: Option<SkillParams>,
}

/// Discovers the hierarchical skill tree over the engine's constellations.
pub fn skills(engine: &SearchEngine, params: &SkillParams) -> Result<SkillTree> {
    let ids = engine.constellation_ids();
    if ids.is_empty() {
        return Ok(SkillTree {
            params: Some(params.clone()),
            ..SkillTree::default()
        });
    }
    if ids.len() > params.max_constellations {
        return Err(sextant_error(
            CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED,
            format!(
                "{} constellations exceed max_constellations {}",
                ids.len(),
                params.max_constellations
            ),
        ));
    }
    let slots = match &params.slots {
        Some(slots) => slots.clone(),
        None => engine.indexes.slots(),
    };
    let mut vectors = Vec::with_capacity(ids.len());
    for cx_id in &ids {
        let dense = dense_vectors(engine, &slots, *cx_id)?;
        if dense.is_empty() {
            return Err(sextant_error(
                CALYX_SEXTANT_CX_MISSING,
                format!("constellation {cx_id} has no dense vector in any participating slot"),
            ));
        }
        vectors.push(dense);
    }
    let dist = pairwise_distances(&ids, &vectors)?;
    let clusters = condensed_tree(
        &dist,
        params.min_samples,
        params.min_cluster_size,
        params.allow_single_cluster,
    )?;

    let names: Vec<String> = clusters
        .iter()
        .map(|cluster| {
            if cluster.parent.is_none() {
                "skill-root".to_string()
            } else {
                skill_name(&cluster.members_at_birth, &ids)
            }
        })
        .collect();
    let mut nodes = BTreeMap::new();
    for (idx, cluster) in clusters.iter().enumerate() {
        let depth = std::iter::successors(cluster.parent, |p| clusters[*p].parent).count();
        nodes.insert(
            names[idx].clone(),
            SkillNode {
                name: names[idx].clone(),
                parent: cluster.parent.map(|p| names[p].clone()),
                children: cluster.children.iter().map(|c| names[*c].clone()).collect(),
                members: cluster.members_at_birth.iter().map(|p| ids[*p]).collect(),
                size: cluster.members_at_birth.len(),
                depth,
                lambda_birth: cluster.birth_lambda,
                stability: cluster.stability,
                selected: cluster.selected,
            },
        );
    }
    let mut selected: Vec<String> = clusters
        .iter()
        .enumerate()
        .filter(|(_, cluster)| cluster.selected)
        .map(|(idx, _)| names[idx].clone())
        .collect();
    selected.sort();
    let clustered: BTreeSet<CxId> = clusters
        .iter()
        .filter(|cluster| cluster.selected)
        .flat_map(|cluster| cluster.members_at_birth.iter().map(|p| ids[*p]))
        .collect();
    let noise: Vec<CxId> = ids
        .iter()
        .filter(|id| !clustered.contains(id))
        .copied()
        .collect();
    Ok(SkillTree {
        root: clusters
            .iter()
            .position(|cluster| cluster.parent.is_none())
            .map(|idx| names[idx].clone()),
        nodes,
        selected,
        noise,
        params: Some(params.clone()),
    })
}

/// Searches the engine, restricted to the members of one named skill.
pub fn search_skill(
    engine: &SearchEngine,
    tree: &SkillTree,
    skill: &str,
    query: &Query,
) -> Result<Vec<Hit>> {
    let node = tree.nodes.get(skill).ok_or_else(|| {
        sextant_error(
            CALYX_SEXTANT_SKILL_UNKNOWN,
            format!("skill '{skill}' is not in the skill tree"),
        )
    })?;
    let members: BTreeSet<CxId> = node.members.iter().copied().collect();
    let recall_window = engine
        .indexes
        .stats()
        .into_iter()
        .map(|stats| stats.len)
        .max()
        .unwrap_or(query.k)
        .max(query.k);
    let mut widened = query.clone();
    widened.k = recall_window;
    widened.ef = widened.ef.map(|ef| ef.max(recall_window));
    let mut hits = engine.search(&widened)?;
    hits.retain(|hit| members.contains(&hit.cx_id));
    hits.truncate(query.k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
    Ok(hits)
}

/// Fused distance: 1 − mean per-shared-lens cosine, clamped to [0, 2].
fn pairwise_distances(
    ids: &[CxId],
    vectors: &[BTreeMap<SlotId, Vec<f32>>],
) -> Result<DistanceMatrix> {
    let n = ids.len();
    let mut upper = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let mut sum = 0.0_f64;
            let mut count = 0usize;
            for (slot, a) in &vectors[i] {
                if let Some(b) = vectors[j].get(slot) {
                    sum += f64::from(dense_cosine(a, b)?);
                    count += 1;
                }
            }
            if count == 0 {
                return Err(sextant_error(
                    CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP,
                    format!(
                        "constellations {} and {} share no dense lens; cannot cluster",
                        ids[i], ids[j]
                    ),
                ));
            }
            upper.push((1.0 - sum / count as f64).clamp(0.0, 2.0));
        }
    }
    DistanceMatrix::new(n, upper)
}

/// Content-addressed deterministic skill name.
fn skill_name(member_points: &[usize], ids: &[CxId]) -> String {
    let digest = content_address(member_points.iter().map(|p| ids[*p].as_bytes()));
    let hex: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("skill-{hex}")
}
