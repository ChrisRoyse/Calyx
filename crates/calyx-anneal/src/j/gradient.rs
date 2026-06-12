use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_core::{CalyxError, Clock, LensId, Result};
use serde::{Deserialize, Serialize};

use crate::{AnchorId, ComponentKind, JValue, JWeights, LogicalTime, ScopeId};

pub const CALYX_ANNEAL_GRADIENT_INVALID_METRIC: &str = "CALYX_ANNEAL_GRADIENT_INVALID_METRIC";
pub const CALYX_ANNEAL_GRADIENT_INVALID_CONFIG: &str = "CALYX_ANNEAL_GRADIENT_INVALID_CONFIG";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuneScopeKind {
    FusionWeights,
    Quantization,
    AnnIndex,
    Kernel,
    Guard,
    OnlineHead,
    Compression,
    Custom { scope: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CandidateAction {
    ProposeLens {
        anchor: AnchorId,
        estimated_dj: f64,
    },
    LabelAnchor {
        anchor: AnchorId,
        estimated_dj: f64,
    },
    PruneRedundantLens {
        lens_id: LensId,
        estimated_dj: f64,
    },
    RecalibrateHeal {
        component: ComponentKind,
        estimated_dj: f64,
    },
    RecomputeKernel {
        scope: ScopeId,
        estimated_dj: f64,
    },
    MaterializeCrossTerm {
        pair: (LensId, LensId),
        estimated_dj: f64,
    },
    RetuneMath {
        scope: TuneScopeKind,
        estimated_dj: f64,
    },
}

impl CandidateAction {
    pub fn propose_lens_from_info(
        anchor: AnchorId,
        info_before: f64,
        info_after: f64,
    ) -> Result<Self> {
        let estimated_dj =
            validate_nonnegative(info_after - info_before, "propose_lens_estimated_dj")
                .map_err(invalid_metric)?;
        Ok(Self::ProposeLens {
            anchor,
            estimated_dj,
        })
    }

    pub fn estimated_dj(&self) -> f64 {
        match self {
            Self::ProposeLens { estimated_dj, .. }
            | Self::LabelAnchor { estimated_dj, .. }
            | Self::PruneRedundantLens { estimated_dj, .. }
            | Self::RecalibrateHeal { estimated_dj, .. }
            | Self::RecomputeKernel { estimated_dj, .. }
            | Self::MaterializeCrossTerm { estimated_dj, .. }
            | Self::RetuneMath { estimated_dj, .. } => *estimated_dj,
        }
    }

    fn weighted_estimated_dj(&self, weights: JWeights) -> Result<f64> {
        let raw =
            validate_nonnegative(self.estimated_dj(), "estimated_dj").map_err(invalid_metric)?;
        let weighted = match self {
            Self::ProposeLens { .. } => raw * weights.w1,
            Self::LabelAnchor { .. } => raw * weights.w1,
            Self::PruneRedundantLens { .. } => raw * weights.w2,
            Self::RecalibrateHeal { .. } => raw * weights.w5,
            Self::RecomputeKernel { .. } => raw * weights.w4,
            Self::MaterializeCrossTerm { .. } => raw * weights.w1,
            Self::RetuneMath { .. } => raw * weights.w7,
        };
        validate_nonnegative(weighted, "weighted_estimated_dj").map_err(invalid_metric)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientCandidate {
    pub action: CandidateAction,
    pub cost_budget_units: u64,
}

#[derive(Clone, Debug)]
pub struct GradientEntry {
    pub action: CandidateAction,
    pub dj_per_cost: f64,
    pub cost_budget_units: u64,
    estimated_dj: f64,
    sequence: u64,
}

impl GradientEntry {
    fn from_candidate(
        candidate: GradientCandidate,
        weights: JWeights,
        sequence: u64,
    ) -> Result<Self> {
        let estimated_dj = estimate_dj(&candidate.action, weights)?;
        let dj_per_cost = if candidate.cost_budget_units == 0 {
            f64::INFINITY
        } else {
            estimated_dj / candidate.cost_budget_units as f64
        };
        Ok(Self {
            action: candidate.action,
            dj_per_cost,
            cost_budget_units: candidate.cost_budget_units,
            estimated_dj,
            sequence,
        })
    }

    pub fn estimated_dj(&self) -> f64 {
        self.estimated_dj
    }

    pub fn to_readback(&self) -> GradientEntryReadback {
        GradientEntryReadback {
            action: self.action.clone(),
            estimated_dj: self.estimated_dj,
            dj_per_cost: PriorityReadback::from_f64(self.dj_per_cost),
            cost_budget_units: self.cost_budget_units,
        }
    }
}

impl PartialEq for GradientEntry {
    fn eq(&self, other: &Self) -> bool {
        self.dj_per_cost.total_cmp(&other.dj_per_cost) == Ordering::Equal
            && self.sequence == other.sequence
    }
}

impl Eq for GradientEntry {}

impl PartialOrd for GradientEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GradientEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dj_per_cost
            .total_cmp(&other.dj_per_cost)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

#[derive(Clone)]
pub struct IntelligenceGradient {
    queue: BinaryHeap<GradientEntry>,
    pub current_j: JValue,
    clock: Arc<dyn Clock>,
    weights: JWeights,
    current_budget_units: u64,
    next_sequence: u64,
    warnings: Vec<GradientWarning>,
}

impl IntelligenceGradient {
    pub fn new(current_j: JValue, clock: Arc<dyn Clock>) -> Self {
        Self {
            weights: current_j.weights,
            current_j,
            clock,
            queue: BinaryHeap::new(),
            current_budget_units: u64::MAX,
            next_sequence: 0,
            warnings: Vec::new(),
        }
    }

    pub fn with_budget_units(mut self, budget_units: u64) -> Self {
        self.current_budget_units = budget_units;
        self
    }

    pub fn set_objective_weights(&mut self, weights: JWeights) -> Result<()> {
        validate_weights(weights)?;
        self.weights = weights;
        Ok(())
    }

    pub fn refresh<I>(&mut self, candidates: I) -> GradientRefreshReport
    where
        I: IntoIterator<Item = GradientCandidate>,
    {
        self.queue.clear();
        self.warnings.clear();
        let mut accepted = 0;
        for candidate in candidates {
            if candidate.cost_budget_units > self.current_budget_units {
                self.warnings.push(GradientWarning::excluded(
                    &candidate.action,
                    "CALYX_ANNEAL_GRADIENT_OVER_BUDGET",
                    "candidate cost exceeds current budget",
                ));
                continue;
            }
            match GradientEntry::from_candidate(candidate.clone(), self.weights, self.next_sequence)
            {
                Ok(entry) => {
                    self.next_sequence += 1;
                    self.queue.push(entry);
                    accepted += 1;
                }
                Err(error) => self.warnings.push(GradientWarning::excluded(
                    &candidate.action,
                    error.code,
                    error.message,
                )),
            }
        }
        GradientRefreshReport {
            accepted,
            rejected: self.warnings.clone(),
        }
    }

    pub fn next_best_action(&self) -> Option<&CandidateAction> {
        self.queue.peek().map(|entry| &entry.action)
    }

    pub fn top_entries(&self, limit: usize) -> Vec<GradientEntry> {
        self.queue
            .clone()
            .into_sorted_vec()
            .into_iter()
            .rev()
            .take(limit)
            .collect()
    }

    pub fn top_readback(&self, limit: usize) -> Vec<GradientEntryReadback> {
        self.top_entries(limit)
            .into_iter()
            .map(|entry| entry.to_readback())
            .collect()
    }

    pub fn warnings(&self) -> &[GradientWarning] {
        &self.warnings
    }

    pub fn snapshot(&self, limit: usize) -> GradientSnapshot {
        GradientSnapshot {
            generated_at: self.clock.now(),
            current_j: self.current_j.j,
            budget_units: self.current_budget_units,
            weights: self.weights,
            gradient: self.top_readback(limit),
            next_best_action: self.next_best_action().cloned(),
            warnings: self.warnings.clone(),
        }
    }
}

pub fn estimate_dj(action: &CandidateAction, weights: JWeights) -> Result<f64> {
    validate_weights(weights)?;
    action.weighted_estimated_dj(weights)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientRefreshReport {
    pub accepted: usize,
    pub rejected: Vec<GradientWarning>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientWarning {
    pub action: String,
    pub code: String,
    pub message: String,
}

impl GradientWarning {
    fn excluded(
        action: &CandidateAction,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            action: format!("{action:?}"),
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientEntryReadback {
    pub action: CandidateAction,
    pub estimated_dj: f64,
    pub dj_per_cost: PriorityReadback,
    pub cost_budget_units: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriorityReadback {
    Finite { value: f64 },
    Infinite,
}

impl PriorityReadback {
    fn from_f64(value: f64) -> Self {
        if value.is_infinite() {
            Self::Infinite
        } else {
            Self::Finite { value }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientSnapshot {
    pub generated_at: LogicalTime,
    pub current_j: f64,
    pub budget_units: u64,
    pub weights: JWeights,
    pub gradient: Vec<GradientEntryReadback>,
    pub next_best_action: Option<CandidateAction>,
    pub warnings: Vec<GradientWarning>,
}

pub fn gradient_state_path(vault: &Path) -> PathBuf {
    vault.join(".anneal").join("gradient_queue.json")
}

pub fn write_gradient_snapshot(vault: &Path, snapshot: &GradientSnapshot) -> Result<PathBuf> {
    let path = gradient_state_path(vault);
    let parent = path
        .parent()
        .ok_or_else(|| invalid_config("gradient state path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        invalid_config(format!(
            "create gradient state dir {}: {error}",
            parent.display()
        ))
    })?;
    let encoded = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| invalid_config(format!("encode gradient snapshot: {error}")))?;
    fs::write(&path, encoded).map_err(|error| {
        invalid_config(format!("write gradient state {}: {error}", path.display()))
    })?;
    Ok(path)
}

pub fn read_gradient_snapshot_from_vault(vault: &Path) -> Result<Option<GradientSnapshot>> {
    let path = gradient_state_path(vault);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| {
        invalid_config(format!("read gradient state {}: {error}", path.display()))
    })?;
    serde_json::from_slice::<GradientSnapshot>(&bytes)
        .map(Some)
        .map_err(|error| {
            invalid_config(format!("parse gradient state {}: {error}", path.display()))
        })
}

fn validate_weights(weights: JWeights) -> Result<()> {
    for (name, value) in [
        ("w1", weights.w1),
        ("w2", weights.w2),
        ("w3", weights.w3),
        ("w4", weights.w4),
        ("w5", weights.w5),
        ("w6", weights.w6),
        ("w7", weights.w7),
        ("w8", weights.w8),
    ] {
        validate_nonnegative(value, name).map_err(invalid_config)?;
    }
    Ok(())
}

fn validate_nonnegative(value: f64, name: &str) -> std::result::Result<f64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "{name} must be finite and non-negative, got {value}"
        ));
    }
    Ok(value)
}

fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GRADIENT_INVALID_METRIC,
        message: message.into(),
        remediation: "exclude invalid gradient candidate and re-measure grounded gain",
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_GRADIENT_INVALID_CONFIG,
        message: message.into(),
        remediation: "correct gradient queue configuration before refreshing candidates",
    }
}
