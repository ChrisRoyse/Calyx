//! Stratified signal-bit accounting for rare sole-carrier anchors.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StratumBits {
    pub name: String,
    pub bits: f32,
    pub frequency: f32,
    pub sole_carrier: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StratifiedBits {
    pub global_bits: f32,
    pub effective_bits: f32,
    pub strata: Vec<StratumBits>,
    pub no_frequency_multiplier: bool,
}

pub fn stratified_bits(global_bits: f32, strata: Vec<StratumBits>) -> StratifiedBits {
    let sole_carrier_bits = strata
        .iter()
        .filter(|stratum| stratum.sole_carrier)
        .map(|stratum| stratum.bits)
        .fold(0.0, f32::max);
    StratifiedBits {
        global_bits,
        effective_bits: global_bits.max(sole_carrier_bits),
        strata,
        no_frequency_multiplier: true,
    }
}
