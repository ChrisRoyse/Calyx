use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use super::{TEMPORAL_FLAGS, TemporalLensFlags, clamp01, parse_i64_timestamp, temporal_lens_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecayFunction {
    Linear { max_age_secs: i64 },
    Exponential { half_life_secs: i64 },
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct E2RecencyConfig {
    pub decay: DecayFunction,
    pub reference_time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct E2RecencyLens {
    id: LensId,
    config: E2RecencyConfig,
}

impl E2RecencyLens {
    pub fn new(config: E2RecencyConfig) -> Self {
        let spec = format!("e2:{:?}:{}", config.decay, config.reference_time);
        Self {
            id: temporal_lens_id(&["E2_Temporal_Recent", &spec]),
            config,
        }
    }

    pub const fn config(&self) -> E2RecencyConfig {
        self.config
    }

    pub const fn flags(&self) -> TemporalLensFlags {
        TEMPORAL_FLAGS
    }

    pub fn score(&self, event_timestamp: i64) -> f32 {
        let age = (self.config.reference_time - event_timestamp).max(0);
        match self.config.decay {
            DecayFunction::Linear { max_age_secs } if max_age_secs > 0 => {
                clamp01(1.0 - age as f32 / max_age_secs as f32)
            }
            DecayFunction::Linear { .. } => 0.0,
            DecayFunction::Exponential { half_life_secs } if half_life_secs > 0 => {
                (-age as f32 * std::f32::consts::LN_2 / half_life_secs as f32).exp()
            }
            DecayFunction::Exponential { .. } => 0.0,
            DecayFunction::Step if age < 3_600 => 0.8,
            DecayFunction::Step if age < 86_400 => 0.5,
            DecayFunction::Step => 0.1,
        }
    }
}

impl Lens for E2RecencyLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Structured
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let timestamp = parse_i64_timestamp(&input.bytes, "E2")?;
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![self.score(timestamp)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(value: i64) -> Input {
        Input::new(Modality::Structured, value.to_le_bytes().to_vec())
    }

    #[test]
    fn linear_decay_matches_hand_math() {
        let lens = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Linear {
                max_age_secs: 1_000,
            },
            reference_time: 1_000,
        });

        assert_eq!(lens.measure(&ts(0)).unwrap().as_dense().unwrap(), &[0.0]);
        assert_eq!(lens.measure(&ts(900)).unwrap().as_dense().unwrap(), &[0.9]);
    }

    #[test]
    fn exponential_and_step_decay_match_reference_values() {
        let exp = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Exponential {
                half_life_secs: 86_400,
            },
            reference_time: 86_400,
        });
        let step = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Step,
            reference_time: 200_000,
        });

        assert!((exp.score(0) - 0.5).abs() < 1e-6);
        assert_eq!(step.score(200_000 - 1_800), 0.8);
        assert_eq!(step.score(200_000 - 43_200), 0.5);
        assert_eq!(step.score(200_000 - 172_800), 0.1);
    }

    #[test]
    fn edge_cases_fail_closed_or_clamp() {
        let future = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Linear { max_age_secs: 10 },
            reference_time: 100,
        });
        let zero_linear = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Linear { max_age_secs: 0 },
            reference_time: 100,
        });
        let zero_exp = E2RecencyLens::new(E2RecencyConfig {
            decay: DecayFunction::Exponential { half_life_secs: 0 },
            reference_time: 100,
        });

        assert_eq!(future.score(200), 1.0);
        assert_eq!(zero_linear.score(0), 0.0);
        assert_eq!(zero_exp.score(0), 0.0);
        assert_eq!(
            future
                .measure(&Input::new(Modality::Structured, vec![1]))
                .unwrap_err()
                .code,
            "CALYX_LENS_DIM_MISMATCH"
        );
    }
}
