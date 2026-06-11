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
        let c_n2 = cross_term_upper_bound(n_lenses);
        let meaning_compression_yield = meaning_compression_yield(materialized, n_constellations);
        Self {
            n_lenses,
            c_n2_upper_bound: c_n2,
            n_constellations,
            materialized,
            n_eff,
            dpi_ceiling,
            measured_count,
            derived_count,
            meaning_compression_yield,
        }
    }
}

pub fn cross_term_upper_bound(n_lenses: usize) -> usize {
    n_lenses.saturating_mul(n_lenses.saturating_sub(1)) / 2
}

pub fn dda_signal_yield(n_inputs: usize, n_lenses: usize) -> usize {
    let per_input = n_lenses
        .saturating_add(cross_term_upper_bound(n_lenses))
        .saturating_add(1);
    n_inputs.saturating_mul(per_input)
}

pub fn meaning_compression_yield(materialized_signals: usize, n_inputs: usize) -> f32 {
    if n_inputs == 0 {
        f32::NAN
    } else {
        materialized_signals as f32 / n_inputs as f32
    }
}
