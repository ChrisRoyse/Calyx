use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{FREQUENCY_SCALAR, StoredRecurrenceRow, decode_recurrence_row};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, VaultStore};
use calyx_mincut::{betweenness, tarjan_scc};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::kernel_graph::sort_node_scores;
use crate::{
    KernelGraph, KernelGraphParams, LodestarError, NodeScore, Result, select_kernel_graph,
};

pub const FREQ_BONUS_MAX: u64 = 10_000;
pub const FREQ_WEIGHT: f64 = 0.15;
pub const CALYX_LODESTAR_MISSING_FREQUENCY: &str = "CALYX_LODESTAR_MISSING_FREQUENCY";
pub const CALYX_LODESTAR_INVALID_FREQUENCY: &str = "CALYX_LODESTAR_INVALID_FREQUENCY";
pub const CALYX_LODESTAR_INVALID_WINDOW: &str = "CALYX_LODESTAR_INVALID_WINDOW";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_secs: i64,
    pub end_secs: i64,
}

impl TimeWindow {
    pub fn new(start_secs: i64, end_secs: i64) -> Result<Self> {
        let window = Self {
            start_secs,
            end_secs,
        };
        validate_window(&window)?;
        Ok(window)
    }

    pub fn contains(self, time: EpochSecs) -> bool {
        time.0 >= self.start_secs && time.0 < self.end_secs
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum KernelScope {
    TimeWindow { window: TimeWindow },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelResult {
    pub scope: KernelScope,
    pub nodes: Vec<KernelWeight>,
    pub active_node_count: usize,
    pub source_node_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelWeight {
    pub cx_id: CxId,
    pub rank: usize,
    pub degree_score: f64,
    pub betweenness_score: f64,
    pub groundedness_score: f64,
    pub frequency: u64,
    pub frequency_bonus: f32,
    pub total_score: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequencyRead {
    pub cx_id: CxId,
    pub frequency: u64,
    pub missing: bool,
}

pub fn frequency_kernel_bonus(frequency: u64) -> f32 {
    if frequency == 0 {
        return 0.0;
    }
    let capped = frequency.min(FREQ_BONUS_MAX) as f32;
    let denom = (FREQ_BONUS_MAX as f32 + 1.0).ln();
    ((capped + 1.0).ln() / denom).min(1.0)
}

pub fn apply_frequency_bonuses<C>(
    kernel_graph: &mut KernelGraph,
    vault: &AsterVault<C>,
) -> Result<Vec<FrequencyRead>>
where
    C: Clock,
{
    let mut reads = Vec::with_capacity(kernel_graph.scores.len());
    let mut warnings = Vec::new();
    for score in &mut kernel_graph.scores {
        let read = read_frequency(vault, score.id, &mut warnings)?;
        let previous = score.frequency_bonus;
        score.frequency_bonus = frequency_kernel_bonus(read.frequency);
        score.total_score -= FREQ_WEIGHT * f64::from(previous);
        score.total_score += FREQ_WEIGHT * f64::from(score.frequency_bonus);
        reads.push(read);
    }
    sort_node_scores(&mut kernel_graph.scores);
    let take = kernel_graph.selected.len().min(kernel_graph.scores.len());
    kernel_graph.selected = kernel_graph
        .scores
        .iter()
        .take(take)
        .map(|score| score.id)
        .collect();
    extend_unique(&mut kernel_graph.warnings, warnings);
    Ok(reads)
}

pub fn kernel_weight_rows(
    kernel_graph: &KernelGraph,
    reads: &[FrequencyRead],
    k: usize,
) -> Vec<KernelWeight> {
    let frequencies: BTreeMap<_, _> = reads
        .iter()
        .map(|read| (read.cx_id, read.frequency))
        .collect();
    kernel_graph
        .scores
        .iter()
        .take(k)
        .enumerate()
        .map(|(idx, score)| weight_row(idx + 1, score, &frequencies))
        .collect()
}

pub fn kernel_for_window<C>(
    vault: &AsterVault<C>,
    window: &TimeWindow,
    k: usize,
) -> Result<KernelResult>
where
    C: Clock,
{
    let active = active_cxids_in_window(vault, window)?;
    let graph = recurrence_only_graph(vault, &active)?;
    kernel_for_window_from_graph(vault, &graph, window, k)
}

pub fn kernel_for_window_from_graph<C>(
    vault: &AsterVault<C>,
    graph: &AssocGraph,
    window: &TimeWindow,
    k: usize,
) -> Result<KernelResult>
where
    C: Clock,
{
    validate_window(window)?;
    let active = active_cxids_in_window(vault, window)?;
    let scoped = subgraph_from_nodes(graph, &active)?;
    if scoped.is_empty() || k == 0 {
        return Ok(KernelResult {
            scope: KernelScope::TimeWindow { window: *window },
            nodes: Vec::new(),
            active_node_count: active.len(),
            source_node_count: graph.node_count(),
            warnings: Vec::new(),
        });
    }
    let mut kernel_graph = full_score_graph(&scoped)?;
    let reads = apply_frequency_bonuses(&mut kernel_graph, vault)?;
    let nodes = kernel_weight_rows(&kernel_graph, &reads, k.min(kernel_graph.scores.len()));
    Ok(KernelResult {
        scope: KernelScope::TimeWindow { window: *window },
        nodes,
        active_node_count: active.len(),
        source_node_count: graph.node_count(),
        warnings: kernel_graph.warnings,
    })
}

pub fn active_cxids_in_window<C>(
    vault: &AsterVault<C>,
    window: &TimeWindow,
) -> Result<BTreeSet<CxId>>
where
    C: Clock,
{
    validate_window(window)?;
    let mut active = BTreeSet::new();
    for (key, value) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::Recurrence)? {
        let cx_id = cx_id_from_recurrence_key(&key)?;
        if let StoredRecurrenceRow::Occurrence(occurrence) = decode_recurrence_row(&value)?
            && window.contains(occurrence.t_k)
        {
            active.insert(cx_id);
        }
    }
    Ok(active)
}

fn full_score_graph(graph: &AssocGraph) -> Result<KernelGraph> {
    let scc = tarjan_scc(graph);
    let bet = betweenness(graph)?;
    let params = KernelGraphParams {
        target_fraction: 1.0,
        ..KernelGraphParams::default()
    };
    select_kernel_graph(graph, &scc, &bet, &[], &params)
}

fn recurrence_only_graph<C>(vault: &AsterVault<C>, active: &BTreeSet<CxId>) -> Result<AssocGraph>
where
    C: Clock,
{
    let mut warnings = Vec::new();
    let mut builder = AssocGraph::builder();
    for id in active {
        let read = read_frequency(vault, *id, &mut warnings)?;
        let weight = frequency_kernel_bonus(read.frequency).max(f32::EPSILON);
        builder.add_node(*id, weight)?;
    }
    Ok(builder.build())
}

fn read_frequency<C>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    warnings: &mut Vec<String>,
) -> Result<FrequencyRead>
where
    C: Clock,
{
    let cx = match vault.get(cx_id, vault.snapshot()) {
        Ok(cx) => cx,
        Err(_) => {
            warnings.push(missing_frequency_warning(cx_id, "base row missing"));
            return Ok(FrequencyRead {
                cx_id,
                frequency: 0,
                missing: true,
            });
        }
    };
    let Some(value) = cx.scalars.get(FREQUENCY_SCALAR) else {
        warnings.push(missing_frequency_warning(cx_id, "scalar missing"));
        return Ok(FrequencyRead {
            cx_id,
            frequency: 0,
            missing: true,
        });
    };
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 {
        return Err(LodestarError::TemporalKernel {
            code: CALYX_LODESTAR_INVALID_FREQUENCY,
            message: format!("{FREQUENCY_SCALAR} for {cx_id} must be a non-negative integer"),
        });
    }
    Ok(FrequencyRead {
        cx_id,
        frequency: *value as u64,
        missing: false,
    })
}

fn weight_row(rank: usize, score: &NodeScore, frequencies: &BTreeMap<CxId, u64>) -> KernelWeight {
    KernelWeight {
        cx_id: score.id,
        rank,
        degree_score: score.degree_score,
        betweenness_score: score.betweenness_score,
        groundedness_score: score.groundedness_score,
        frequency: *frequencies.get(&score.id).unwrap_or(&0),
        frequency_bonus: score.frequency_bonus,
        total_score: score.total_score,
    }
}

fn validate_window(window: &TimeWindow) -> Result<()> {
    if window.start_secs <= window.end_secs {
        return Ok(());
    }
    Err(LodestarError::TemporalKernel {
        code: CALYX_LODESTAR_INVALID_WINDOW,
        message: format!(
            "time window start_secs={} must be <= end_secs={}",
            window.start_secs, window.end_secs
        ),
    })
}

fn cx_id_from_recurrence_key(key: &[u8]) -> Result<CxId> {
    let bytes: [u8; 16] = key
        .get(..16)
        .ok_or_else(|| LodestarError::TemporalKernel {
            code: CALYX_LODESTAR_INVALID_WINDOW,
            message: format!("recurrence key length {} is shorter than CxId", key.len()),
        })?
        .try_into()
        .expect("slice length checked");
    Ok(CxId::from_bytes(bytes))
}

fn subgraph_from_nodes(source: &AssocGraph, nodes: &BTreeSet<CxId>) -> Result<AssocGraph> {
    let mut builder = AssocGraph::builder();
    for id in nodes {
        if source.node_index(*id).is_some() {
            builder.add_node(*id, source.node_weight(*id)?)?;
        }
    }
    for edge in source.edges() {
        let (src, dst) = source.edge_endpoints(*edge);
        if nodes.contains(&src) && nodes.contains(&dst) {
            builder.add_edge(src, dst, edge.weight)?;
        }
    }
    Ok(builder.build())
}

fn missing_frequency_warning(cx_id: CxId, detail: &str) -> String {
    format!("{CALYX_LODESTAR_MISSING_FREQUENCY}: {detail} for {cx_id}; using frequency=0")
}

fn extend_unique(target: &mut Vec<String>, source: Vec<String>) {
    for warning in source {
        if !target.contains(&warning) {
            target.push(warning);
        }
    }
}
