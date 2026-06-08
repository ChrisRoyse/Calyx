//! Honest DDA abundance reporting.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeffEstimate {
    Provisional {
        value: f32,
    },
    Computed {
        value: f32,
        ci_low: f32,
        ci_high: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeilingEstimate {
    Provisional { bits: f32 },
    Computed { bits: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbundanceReport {
    pub n_lenses: usize,
    pub c_n2_upper_bound: usize,
    pub n_constellations: usize,
    pub materialized: usize,
    pub n_eff: NeffEstimate,
    pub dpi_ceiling: CeilingEstimate,
    pub measured_count: usize,
    pub derived_count: usize,
    pub meaning_compression_yield: f32,
}

impl AbundanceReport {
    pub fn new(
        n_lenses: usize,
        n_constellations: usize,
        materialized: usize,
        n_eff: NeffEstimate,
        dpi_ceiling: CeilingEstimate,
        measured_count: usize,
        derived_count: usize,
    ) -> Self {
        let c_n2 = n_lenses.saturating_mul(n_lenses.saturating_sub(1)) / 2;
        let possible = c_n2.saturating_mul(n_constellations).max(1);
        Self {
            n_lenses,
            c_n2_upper_bound: c_n2,
            n_constellations,
            materialized,
            n_eff,
            dpi_ceiling,
            measured_count,
            derived_count,
            meaning_compression_yield: materialized as f32 / possible as f32,
        }
    }
}
