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
//!   "gte-base":   { "vram_mb": 1340.0, "ms_per_input": 4.2, "ram_mb": 0.0 },
//!   "potion-256": { "vram_mb": 0.0,    "ms_per_input": 0.08, "ram_mb": 64.0 }
//! }
//! ```
//! There is no silent default: when `--cost-json` is supplied, every corpus
//! lens MUST have an entry (enforced in the engine), and every field is
//! validated fail-closed here.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Measured resource cost of one lens over a profiling probe batch.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct LensCost {
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

/// Per-lens signal density: measured bits divided by measured cost.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct LensDensity {
    pub(crate) vram_mb: f32,
    pub(crate) ms_per_input: f32,
    pub(crate) ram_mb: f32,
    /// `bits / VRAM-MB`. `None` when the lens uses zero VRAM (CPU-only): the
    /// GPU-density axis is undefined/unbounded there, which is the *best*
    /// possible position on the scarce resource — callers rank these first.
    pub(crate) bits_per_vram_mb: Option<f32>,
    /// `bits / ms`. Always defined (`ms_per_input > 0`).
    pub(crate) bits_per_ms: f32,
    /// True iff this lens has zero GPU footprint.
    pub(crate) zero_vram: bool,
}

impl LensDensity {
    /// Compute density from measured bits and measured cost. `bits` is clamped
    /// at zero (a negative MI point estimate is noise around zero signal and
    /// must not produce a negative density).
    pub(crate) fn compute(bits: f32, cost: LensCost) -> Self {
        let bits = bits.max(0.0);
        let zero_vram = cost.vram_mb == 0.0;
        let bits_per_vram_mb = if zero_vram {
            None
        } else {
            Some(bits / cost.vram_mb)
        };
        LensDensity {
            vram_mb: cost.vram_mb,
            ms_per_input: cost.ms_per_input,
            ram_mb: cost.ram_mb,
            bits_per_vram_mb,
            bits_per_ms: bits / cost.ms_per_input,
            zero_vram,
        }
    }
}
