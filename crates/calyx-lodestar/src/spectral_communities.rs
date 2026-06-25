use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_mincut::{eigenvector_centrality, laplacian_eigenmaps_with_max_iter, spectral_gap};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const SPECTRAL_COMMUNITY_SCHEMA_VERSION: u32 = 1;
const EDGE_WEIGHT: f32 = 0.50;
const BRIDGE_CENTRALITY_WEIGHT: f32 = 0.35;
const BRIDGE_FREQUENCY_WEIGHT: f32 = 0.15;
const CENTRALITY_WEIGHT: f32 = 0.70;
const CENTRALITY_DEGREE_WEIGHT: f32 = 0.20;
const CENTRALITY_FREQUENCY_WEIGHT: f32 = 0.10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCommunityParams {
    pub eigen_k: usize,
    pub eigen_max_iter: usize,
    pub centrality_max_iter: usize,
    pub centrality_tol: f32,
    pub max_bridge_candidates: usize,
    pub max_centrality_candidates: usize,
}

impl Default for SpectralCommunityParams {
    fn default() -> Self {
        Self {
            eigen_k: 3,
            eigen_max_iter: 256,
            centrality_max_iter: 128,
            centrality_tol: 1.0e-6,
            max_bridge_candidates: 32,
            max_centrality_candidates: 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCommunitySummary {
    pub community: u8,
    pub member_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCommunityMember {
    pub cx_id: CxId,
    pub community: u8,
    pub fiedler_value: f32,
    pub centrality_score: f32,
    pub frequency_weight: f32,
    pub degree: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterCommunityBridgeCandidate {
    pub src: CxId,
    pub dst: CxId,
    pub src_community: u8,
    pub dst_community: u8,
    pub edge_weight: f32,
    pub src_centrality: f32,
    pub dst_centrality: f32,
    pub src_frequency_weight: f32,
    pub dst_frequency_weight: f32,
    pub rank_score: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCentralityCandidate {
    pub cx_id: CxId,
    pub community: u8,
    pub centrality_score: f32,
    pub frequency_weight: f32,
    pub degree: usize,
    pub rank_score: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCommunityReport {
    pub schema_version: u32,
    pub node_count: usize,
    pub edge_count: usize,
    pub spectral_gap: f32,
    pub fiedler_eigenvalue: f32,
    pub eigenvalues: Vec<f32>,
    pub communities: Vec<SpectralCommunitySummary>,
    pub members: Vec<SpectralCommunityMember>,
    pub bridge_candidates: Vec<InterCommunityBridgeCandidate>,
    pub centrality_candidates: Vec<SpectralCentralityCandidate>,
}

pub fn spectral_community_report(
    graph: &AssocGraph,
    params: &SpectralCommunityParams,
) -> Result<SpectralCommunityReport> {
    validate_params(params)?;
    let eigenmaps =
        laplacian_eigenmaps_with_max_iter(graph, params.eigen_k, params.eigen_max_iter)?;
    if eigenmaps.len() < 2 {
        return Err(LodestarError::Graph {
            code: "CALYX_SPECTRAL_GRAPH_TOO_SMALL",
            message: "spectral community report requires at least two eigenmaps".to_string(),
        });
    }
    let centrality = centrality_map(eigenvector_centrality(
        graph,
        params.centrality_max_iter,
        params.centrality_tol,
    )?);
    let fiedler = &eigenmaps[1];
    let max_frequency = max_frequency(graph);
    let max_degree = max_degree(graph)?;
    let members = community_members(graph, &fiedler.eigenvector, &centrality)?;
    let community_by_id = members
        .iter()
        .map(|member| (member.cx_id, member.community))
        .collect::<BTreeMap<_, _>>();
    let communities = community_summaries(&members);
    let mut bridge_candidates =
        bridge_candidates(graph, &community_by_id, &centrality, max_frequency)?;
    sort_bridge_candidates(&mut bridge_candidates);
    bridge_candidates.truncate(params.max_bridge_candidates);

    let mut centrality_candidates = centrality_candidates(&members, max_frequency, max_degree);
    sort_centrality_candidates(&mut centrality_candidates);
    centrality_candidates.truncate(params.max_centrality_candidates);

    Ok(SpectralCommunityReport {
        schema_version: SPECTRAL_COMMUNITY_SCHEMA_VERSION,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
        spectral_gap: spectral_gap(&eigenmaps),
        fiedler_eigenvalue: fiedler.eigenvalue,
        eigenvalues: eigenmaps.iter().map(|pair| pair.eigenvalue).collect(),
        communities,
        members,
        bridge_candidates,
        centrality_candidates,
    })
}

fn community_members(
    graph: &AssocGraph,
    fiedler: &[f32],
    centrality: &BTreeMap<CxId, f32>,
) -> Result<Vec<SpectralCommunityMember>> {
    if fiedler.len() != graph.node_count() {
        return invalid_params("fiedler vector length must match graph node count");
    }
    let mut members = Vec::with_capacity(graph.node_count());
    for (index, fiedler_value) in fiedler.iter().copied().enumerate() {
        let cx_id = graph.node_id(index).ok_or_else(|| LodestarError::Graph {
            code: "CALYX_GRAPH_UNKNOWN_NODE",
            message: format!("graph node index {index} is missing"),
        })?;
        members.push(SpectralCommunityMember {
            cx_id,
            community: if fiedler_value >= 0.0 { 0 } else { 1 },
            fiedler_value,
            centrality_score: centrality.get(&cx_id).copied().unwrap_or(0.0),
            frequency_weight: graph.node_weight(cx_id)?,
            degree: graph.in_degree(cx_id)? + graph.out_degree(cx_id)?,
        });
    }
    Ok(members)
}

fn community_summaries(members: &[SpectralCommunityMember]) -> Vec<SpectralCommunitySummary> {
    let mut counts = BTreeMap::<u8, usize>::new();
    for member in members {
        *counts.entry(member.community).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(community, member_count)| SpectralCommunitySummary {
            community,
            member_count,
        })
        .collect()
}

fn bridge_candidates(
    graph: &AssocGraph,
    community_by_id: &BTreeMap<CxId, u8>,
    centrality: &BTreeMap<CxId, f32>,
    max_frequency: f32,
) -> Result<Vec<InterCommunityBridgeCandidate>> {
    let mut candidates = Vec::new();
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        let src_community = community_by_id[&src];
        let dst_community = community_by_id[&dst];
        if src_community == dst_community {
            continue;
        }
        let src_frequency_weight = graph.node_weight(src)?;
        let dst_frequency_weight = graph.node_weight(dst)?;
        let src_centrality = centrality.get(&src).copied().unwrap_or(0.0);
        let dst_centrality = centrality.get(&dst).copied().unwrap_or(0.0);
        let centrality_score = (src_centrality + dst_centrality) * 0.5;
        let frequency_score =
            ((src_frequency_weight + dst_frequency_weight) * 0.5) / max_frequency.max(f32::EPSILON);
        let rank_score = edge.weight * EDGE_WEIGHT
            + centrality_score * BRIDGE_CENTRALITY_WEIGHT
            + frequency_score * BRIDGE_FREQUENCY_WEIGHT;
        candidates.push(InterCommunityBridgeCandidate {
            src,
            dst,
            src_community,
            dst_community,
            edge_weight: edge.weight,
            src_centrality,
            dst_centrality,
            src_frequency_weight,
            dst_frequency_weight,
            rank_score,
            provenance: vec![format!("spectral_inter_community_edge:{src}->{dst}")],
        });
    }
    Ok(candidates)
}

fn centrality_candidates(
    members: &[SpectralCommunityMember],
    max_frequency: f32,
    max_degree: usize,
) -> Vec<SpectralCentralityCandidate> {
    members
        .iter()
        .map(|member| {
            let degree_score = member.degree as f32 / max_degree.max(1) as f32;
            let frequency_score = member.frequency_weight / max_frequency.max(f32::EPSILON);
            let rank_score = member.centrality_score * CENTRALITY_WEIGHT
                + degree_score * CENTRALITY_DEGREE_WEIGHT
                + frequency_score * CENTRALITY_FREQUENCY_WEIGHT;
            SpectralCentralityCandidate {
                cx_id: member.cx_id,
                community: member.community,
                centrality_score: member.centrality_score,
                frequency_weight: member.frequency_weight,
                degree: member.degree,
                rank_score,
                provenance: vec![format!("spectral_eigenvector_centrality:{}", member.cx_id)],
            }
        })
        .collect()
}

fn centrality_map(scores: Vec<(CxId, f32)>) -> BTreeMap<CxId, f32> {
    scores.into_iter().collect()
}

fn sort_bridge_candidates(candidates: &mut [InterCommunityBridgeCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.edge_weight.total_cmp(&left.edge_weight))
            .then_with(|| left.src.as_bytes().cmp(right.src.as_bytes()))
            .then_with(|| left.dst.as_bytes().cmp(right.dst.as_bytes()))
    });
}

fn sort_centrality_candidates(candidates: &mut [SpectralCentralityCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.centrality_score.total_cmp(&left.centrality_score))
            .then_with(|| right.degree.cmp(&left.degree))
            .then_with(|| left.cx_id.as_bytes().cmp(right.cx_id.as_bytes()))
    });
}

fn max_frequency(graph: &AssocGraph) -> f32 {
    graph
        .nodes()
        .iter()
        .map(|node| node.frequency_weight)
        .fold(0.0_f32, f32::max)
}

fn max_degree(graph: &AssocGraph) -> Result<usize> {
    let mut value = 1_usize;
    for id in graph.node_ids() {
        value = value.max(graph.in_degree(id)? + graph.out_degree(id)?);
    }
    Ok(value)
}

fn validate_params(params: &SpectralCommunityParams) -> Result<()> {
    if params.eigen_k < 2 {
        return invalid_params("eigen_k must be at least two to include a Fiedler vector");
    }
    if params.eigen_max_iter == 0 || params.centrality_max_iter == 0 {
        return invalid_params("spectral iteration counts must be greater than zero");
    }
    if !params.centrality_tol.is_finite() || params.centrality_tol <= 0.0 {
        return invalid_params("centrality_tol must be finite and greater than zero");
    }
    if params.max_bridge_candidates == 0 || params.max_centrality_candidates == 0 {
        return invalid_params("candidate limits must be greater than zero");
    }
    Ok(())
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
