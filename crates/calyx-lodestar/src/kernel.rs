use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, content_address};
use calyx_mincut::{betweenness, tarjan_scc};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::grounding_gaps::grounding_gaps_for_members;
use crate::recall_test::RecallTestParams;
use crate::temporal_kernel::apply_frequency_bonuses;
use crate::{
    DfvsResult, KernelGraph, KernelGraphParams, LpRoundParams, Result, dfvs_approx,
    lp_round_kernel_graph, select_kernel_graph,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundednessReport {
    pub reached_anchor: f32,
    pub unanchored_members: Vec<CxId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallReport {
    pub kernel_only: f32,
    pub full: f32,
    pub ratio: f32,
    pub approx_factor: f64,
    pub tau_star_estimate: usize,
    pub tau_star_exact: bool,
    pub recall_test_params: Option<RecallTestParams>,
    pub corpus_name: Option<String>,
    pub n_queries_tested: usize,
    pub held_out: Vec<CxId>,
    pub warning: Option<String>,
}

impl Default for RecallReport {
    fn default() -> Self {
        Self {
            kernel_only: 0.0,
            full: 0.0,
            ratio: 0.0,
            approx_factor: 1.0,
            tau_star_estimate: 0,
            tau_star_exact: true,
            recall_test_params: None,
            corpus_name: None,
            n_queries_tested: 0,
            held_out: Vec::new(),
            warning: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kernel {
    pub kernel_id: CxId,
    pub panel_version: u64,
    pub anchor_kind: Option<String>,
    pub corpus_shard_hash: [u8; 32],
    pub members: Vec<CxId>,
    pub kernel_graph: Vec<CxId>,
    pub groundedness: GroundednessReport,
    pub recall: RecallReport,
    pub built_at_millis: u64,
    pub estimator_provenance: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelParams {
    pub panel_version: u64,
    pub anchor_kind: Option<String>,
    pub corpus_shard_hash: [u8; 32],
    pub built_at_millis: u64,
    pub kernel_graph: KernelGraphParams,
    pub lp_round: LpRoundParams,
}

impl Default for KernelParams {
    fn default() -> Self {
        Self {
            panel_version: 1,
            anchor_kind: Some("synthetic".to_string()),
            corpus_shard_hash: [0; 32],
            built_at_millis: 0,
            kernel_graph: KernelGraphParams::default(),
            lp_round: LpRoundParams::default(),
        }
    }
}

pub fn build_kernel_pipeline(
    graph: &AssocGraph,
    anchors: &[CxId],
    params: &KernelParams,
) -> Result<Kernel> {
    build_kernel_pipeline_with_adjustment(graph, anchors, params, |_| Ok(()))
}

pub fn build_kernel_pipeline_with_frequency<C>(
    graph: &AssocGraph,
    anchors: &[CxId],
    params: &KernelParams,
    vault: &AsterVault<C>,
) -> Result<Kernel>
where
    C: Clock,
{
    build_kernel_pipeline_with_adjustment(graph, anchors, params, |heuristic| {
        apply_frequency_bonuses(heuristic, vault).map(|_| ())
    })
}

fn build_kernel_pipeline_with_adjustment(
    graph: &AssocGraph,
    anchors: &[CxId],
    params: &KernelParams,
    mut adjust_heuristic: impl FnMut(&mut KernelGraph) -> Result<()>,
) -> Result<Kernel> {
    if graph.is_empty() {
        return Ok(empty_kernel(params));
    }
    let scc = tarjan_scc(graph);
    let bet = betweenness(graph)?;
    let mut heuristic = select_kernel_graph(graph, &scc, &bet, anchors, &params.kernel_graph)?;
    adjust_heuristic(&mut heuristic)?;
    let rounded = lp_round_kernel_graph(&heuristic, &params.lp_round)?;
    let dfvs = dfvs_approx(&rounded)?;
    let gap_report = grounding_gaps_for_members(
        &dfvs.members,
        graph,
        anchors,
        params.kernel_graph.max_groundedness_distance,
    )?;
    let warnings = warnings(&rounded.warnings, &dfvs, &gap_report.gaps);
    let provenance = estimator_provenance(&dfvs, &warnings);
    let kernel_graph = rounded.selected.clone();
    let kernel_id = kernel_id(params, &dfvs.members, &kernel_graph);

    Ok(Kernel {
        kernel_id,
        panel_version: params.panel_version,
        anchor_kind: params.anchor_kind.clone(),
        corpus_shard_hash: params.corpus_shard_hash,
        members: dfvs.members.clone(),
        kernel_graph,
        groundedness: groundedness_report(&dfvs.members, gap_report.gaps),
        recall: RecallReport {
            approx_factor: dfvs.approx_factor,
            tau_star_estimate: dfvs.tau_star_estimate,
            tau_star_exact: dfvs.tau_star_exact,
            ..RecallReport::default()
        },
        built_at_millis: params.built_at_millis,
        estimator_provenance: provenance,
        warnings,
    })
}

fn groundedness_report(members: &[CxId], unanchored: Vec<CxId>) -> GroundednessReport {
    let reached = members.len().saturating_sub(unanchored.len());
    GroundednessReport {
        reached_anchor: if members.is_empty() {
            1.0
        } else {
            reached as f32 / members.len() as f32
        },
        unanchored_members: unanchored,
    }
}

fn warnings(rounded_warnings: &[String], dfvs: &DfvsResult, unanchored: &[CxId]) -> Vec<String> {
    let mut warnings = rounded_warnings.to_vec();
    if !dfvs.members.is_empty() && unanchored.len() == dfvs.members.len() {
        warnings.push("CALYX_KERNEL_UNGROUNDED: all kernel members are provisional".to_string());
    }
    warnings
}

fn estimator_provenance(dfvs: &DfvsResult, warnings: &[String]) -> String {
    let trust = if warnings
        .iter()
        .any(|warning| warning.starts_with("CALYX_KERNEL_UNGROUNDED"))
    {
        "provisional"
    } else {
        "anchored"
    };
    format!(
        "ph32::{:?}; approx_factor={:.6}; tau_star_estimate={}; tau_star_exact={}; trust={trust}",
        dfvs.method, dfvs.approx_factor, dfvs.tau_star_estimate, dfvs.tau_star_exact
    )
}

fn kernel_id(params: &KernelParams, members: &[CxId], kernel_graph: &[CxId]) -> CxId {
    let mut parts = vec![
        params.panel_version.to_be_bytes().to_vec(),
        params.anchor_kind.clone().unwrap_or_default().into_bytes(),
        params.corpus_shard_hash.to_vec(),
    ];
    parts.extend(members.iter().map(|id| id.as_bytes().to_vec()));
    parts.extend(kernel_graph.iter().map(|id| id.as_bytes().to_vec()));
    CxId::from_bytes(content_address(parts))
}

fn empty_kernel(params: &KernelParams) -> Kernel {
    Kernel {
        kernel_id: kernel_id(params, &[], &[]),
        panel_version: params.panel_version,
        anchor_kind: params.anchor_kind.clone(),
        corpus_shard_hash: params.corpus_shard_hash,
        members: Vec::new(),
        kernel_graph: Vec::new(),
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: params.built_at_millis,
        estimator_provenance: "ph32::empty; trust=anchored".to_string(),
        warnings: Vec::new(),
    }
}
