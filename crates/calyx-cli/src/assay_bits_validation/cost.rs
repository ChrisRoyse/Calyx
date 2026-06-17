//! Per-lens resource cost input for signal-DENSITY scoring (#718 / #717).
//!
//! `assay bits-validate` scores pre-computed vectors and therefore cannot
//! measure a lens's runtime cost itself — that is measured when the lens is
//! profiled (`calyx lens explain` / the registry capability card, whose
//! `CostMetrics` carries `vram_bytes` and `ms_per_input`). This module loads
//! those real, measured costs from a sidecar JSON so the engine can divide
//! measured signal (bits) by measured cost to produce **signal density**:
//! `bits / VRAM-MB` and `bits / ms`.
//!
//! The guiding principle (operator, 2026-06-17): the optimal panel maximizes
//! signal density, not raw bits — a CPU-only static-lookup lens that consumes
//! **zero** VRAM is the best possible trade on the scarce GPU resource. The
//! schema and the engine therefore treat `vram_mb == 0` as a first-class case
//! ("no GPU footprint"), not an error.
//!
//! ## Schema (`--cost-json`)
//! A flat JSON object keyed by the same lens names used in `vectors.jsonl`:
//! ```json
//! {
//!   "gte-base":   { "placement": "gpu", "vram_mb": 1340.0, "ms_per_input": 4.2, "ram_mb": 0.0 },
//!   "potion-256": { "placement": "cpu", "vram_mb": 0.0,    "ms_per_input": 0.08, "ram_mb": 64.0 }
//! }
//! ```
//! There is no silent default: when `--cost-json` is supplied, every corpus
//! lens MUST have an entry (enforced in the engine), and every field is
//! validated fail-closed here.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_assay::{PanelResourceBudget, ResourceUsage, pack_panel_by_density};
use calyx_core::Placement;
use serde::{Deserialize, Serialize};

/// Measured resource cost of one lens over a profiling probe batch.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct LensCost {
    /// Runtime placement selected by registry/Forge admission.
    pub(crate) placement: Placement,
    /// Resident GPU memory in MiB (`vram_bytes / 2^20`). `0.0` for CPU-only
    /// lenses (static_lookup / algorithmic) — a first-class, preferred case.
    pub(crate) vram_mb: f32,
    /// Wall-clock embed latency per input in milliseconds. Strictly positive
    /// (it is a divisor for the latency-density axis).
    pub(crate) ms_per_input: f32,
    /// Resident host memory in MiB. Informational; defaults to 0.0.
    #[serde(default)]
    pub(crate) ram_mb: f32,
}

impl LensCost {
    pub(crate) fn usage(self) -> ResourceUsage {
        ResourceUsage {
            vram_mb: self.vram_mb,
            ram_mb: self.ram_mb,
            ms_per_input: self.ms_per_input,
        }
    }
}

/// Loaded, validated map of lens name -> measured cost.
#[derive(Clone, Debug, Default)]
pub(crate) struct LensCostMap {
    costs: BTreeMap<String, LensCost>,
}

impl LensCostMap {
    /// Load and validate a `--cost-json` sidecar. Every field is checked
    /// fail-closed: non-finite or negative `vram_mb`/`ram_mb`, or a
    /// non-positive `ms_per_input`, is a hard error (no clamping, no defaults).
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("CALYX_FSV_ASSAY_COST_IO: {}: {error}", path.display()))?;
        let costs: BTreeMap<String, LensCost> = serde_json::from_str(&text).map_err(|error| {
            format!("CALYX_FSV_ASSAY_INVALID_COST: {}: {error}", path.display())
        })?;
        if costs.is_empty() {
            return Err(format!(
                "CALYX_FSV_ASSAY_INVALID_COST: {} has no lens cost entries",
                path.display()
            ));
        }
        for (name, cost) in &costs {
            if !cost.vram_mb.is_finite() || cost.vram_mb < 0.0 {
                return Err(format!(
                    "CALYX_FSV_ASSAY_INVALID_COST: lens={name} vram_mb={} must be finite and >= 0",
                    cost.vram_mb
                ));
            }
            if !cost.ram_mb.is_finite() || cost.ram_mb < 0.0 {
                return Err(format!(
                    "CALYX_FSV_ASSAY_INVALID_COST: lens={name} ram_mb={} must be finite and >= 0",
                    cost.ram_mb
                ));
            }
            if !cost.ms_per_input.is_finite() || cost.ms_per_input <= 0.0 {
                return Err(format!(
                    "CALYX_FSV_ASSAY_INVALID_COST: lens={name} ms_per_input={} must be finite and > 0",
                    cost.ms_per_input
                ));
            }
        }
        Ok(Self { costs })
    }

    /// Cost for a lens, or a fail-closed error naming the missing lens.
    pub(crate) fn require(&self, lens: &str) -> Result<LensCost, String> {
        self.costs
            .get(lens)
            .copied()
            .ok_or_else(|| format!("CALYX_FSV_ASSAY_MISSING_COST: no cost entry for lens={lens}"))
    }
}

pub(crate) struct PanelBudgetConfig;

impl PanelBudgetConfig {
    pub(crate) fn load(path: &Path) -> Result<PanelResourceBudget, String> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_PANEL_BUDGET_IO: {}: {error}",
                path.display()
            )
        })?;
        let budget: PanelResourceBudget = serde_json::from_str(&text).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_INVALID_PANEL_BUDGET: {}: {error}",
                path.display()
            )
        })?;
        pack_panel_by_density(&[], budget).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_INVALID_PANEL_BUDGET: {}: {}: {}",
                path.display(),
                error.code,
                error.message
            )
        })?;
        Ok(budget)
    }
}
