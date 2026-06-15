use calyx_core::LensCost;
use serde::{Deserialize, Serialize};

use super::{Observation, ProfileProbe};

const BATCH_TARGET_MS: f32 = 1_000.0;
const F32_BYTES: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostMetrics {
    pub total_ms: f32,
    pub ms_per_input: f32,
    pub vram_bytes: u64,
    #[serde(default)]
    pub ram_bytes: u64,
    #[serde(default)]
    pub batch_ceiling: u32,
}

impl CostMetrics {
    pub(super) fn from_profile(
        total_ms: f32,
        probes: &[ProfileProbe],
        observations: &[Observation],
        vram_before: u64,
        vram_after: u64,
    ) -> Self {
        let measured = observations.len().max(1) as f32;
        let ms_per_input = total_ms / measured;
        Self {
            total_ms,
            ms_per_input,
            vram_bytes: vram_after.saturating_sub(vram_before),
            ram_bytes: ram_bytes(probes, observations),
            batch_ceiling: batch_ceiling(ms_per_input),
        }
    }
}

impl From<CostMetrics> for LensCost {
    fn from(cost: CostMetrics) -> Self {
        Self {
            total_ms: cost.total_ms,
            ms_per_input: cost.ms_per_input,
            vram_bytes: cost.vram_bytes,
            ram_bytes: cost.ram_bytes,
            batch_ceiling: cost.batch_ceiling,
        }
    }
}

impl From<LensCost> for CostMetrics {
    fn from(cost: LensCost) -> Self {
        Self {
            total_ms: cost.total_ms,
            ms_per_input: cost.ms_per_input,
            vram_bytes: cost.vram_bytes,
            ram_bytes: cost.ram_bytes,
            batch_ceiling: cost.batch_ceiling,
        }
    }
}

fn ram_bytes(probes: &[ProfileProbe], observations: &[Observation]) -> u64 {
    let probe_bytes = probes.iter().fold(0_u64, |acc, probe| {
        acc.saturating_add(probe.input.bytes.len() as u64)
            .saturating_add(probe.label.as_ref().map_or(0, |label| label.len() as u64))
    });
    let vector_bytes = observations.iter().fold(0_u64, |acc, observation| {
        acc.saturating_add((observation.data.len() as u64).saturating_mul(F32_BYTES))
    });
    probe_bytes.saturating_add(vector_bytes)
}

fn batch_ceiling(ms_per_input: f32) -> u32 {
    if !ms_per_input.is_finite() || ms_per_input < 0.0 {
        return 1;
    }
    if ms_per_input <= f32::EPSILON {
        return u32::MAX;
    }
    (BATCH_TARGET_MS / ms_per_input)
        .floor()
        .clamp(1.0, u32::MAX as f32) as u32
}

#[cfg(test)]
mod tests {
    use calyx_core::{Input, Modality};

    use super::*;

    #[test]
    fn profile_cost_records_ram_and_batch_ceiling() {
        let probes = vec![
            ProfileProbe::labeled(Input::new(Modality::Text, b"abc".to_vec()), "x"),
            ProfileProbe::new(Input::new(Modality::Text, b"defg".to_vec())),
        ];
        let observations = vec![
            Observation {
                data: vec![1.0, 2.0],
                label: Some("x".to_string()),
            },
            Observation {
                data: vec![3.0, 4.0],
                label: None,
            },
        ];

        let cost = CostMetrics::from_profile(20.0, &probes, &observations, 10, 12);

        assert_eq!(cost.ms_per_input, 10.0);
        assert_eq!(cost.vram_bytes, 2);
        assert_eq!(cost.ram_bytes, 24);
        assert_eq!(cost.batch_ceiling, 100);
    }

    #[test]
    fn zero_latency_gets_unbounded_batch_ceiling() {
        assert_eq!(batch_ceiling(0.0), u32::MAX);
    }

    #[test]
    fn invalid_latency_fails_to_single_batch() {
        assert_eq!(batch_ceiling(f32::NAN), 1);
        assert_eq!(batch_ceiling(-1.0), 1);
    }
}
