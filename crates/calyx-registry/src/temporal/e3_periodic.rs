use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use super::{
    TEMPORAL_FLAGS, TemporalLensFlags, clamp01, parse_i64_timestamp, temporal_lens_id,
    utc_day_of_week_monday0, utc_hour,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicOptions {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
    pub use_now: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct E3PeriodicConfig {
    pub options: PeriodicOptions,
    pub reference_time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct E3PeriodicLens {
    id: LensId,
    config: E3PeriodicConfig,
}

impl E3PeriodicLens {
    pub fn new(config: E3PeriodicConfig) -> Self {
        let spec = format!("{:?}:{}", config.options, config.reference_time);
        Self {
            id: temporal_lens_id(&["E3_Temporal_Periodic", &spec]),
            config,
        }
    }

    pub const fn flags(&self) -> TemporalLensFlags {
        TEMPORAL_FLAGS
    }

    pub fn scores(&self, timestamp: i64) -> Result<[f32; 2]> {
        let event_hour = utc_hour(timestamp);
        let event_dow = utc_day_of_week_monday0(timestamp);
        let hour_target = self.config.options.target_hour.or_else(|| {
            self.config
                .options
                .use_now
                .then(|| utc_hour(self.config.reference_time))
        });
        let dow_target = self.config.options.target_day_of_week.or_else(|| {
            self.config
                .options
                .use_now
                .then(|| utc_day_of_week_monday0(self.config.reference_time))
        });
        Ok([
            match hour_target {
                Some(hour) if hour < 24 => circular_score(event_hour, hour, 24, 12.0),
                Some(hour) => return Err(invalid_target("target_hour", hour, 23)),
                None => 1.0,
            },
            match dow_target {
                Some(day) if day < 7 => circular_score(event_dow, day, 7, 3.5),
                Some(day) => return Err(invalid_target("target_day_of_week", day, 6)),
                None => 1.0,
            },
        ])
    }
}

impl Lens for E3PeriodicLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(2)
    }

    fn modality(&self) -> Modality {
        Modality::Structured
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let timestamp = parse_i64_timestamp(&input.bytes, "E3")?;
        let [hour_score, dow_score] = self.scores(timestamp)?;
        Ok(SlotVector::Dense {
            dim: 2,
            data: vec![hour_score, dow_score],
        })
    }
}

fn circular_score(value: u8, target: u8, span: i16, max_distance: f32) -> f32 {
    let diff = (i16::from(value) - i16::from(target)).abs();
    let dist = diff.min(span - diff) as f32;
    clamp01(1.0 - dist / max_distance)
}

fn invalid_target(field: &str, value: u8, max: u8) -> CalyxError {
    CalyxError::lens_dim_mismatch(format!("{field}={value} outside 0..={max}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(value: i64) -> Input {
        Input::new(Modality::Structured, value.to_le_bytes().to_vec())
    }

    #[test]
    fn hour_scores_are_circular() {
        let lens = E3PeriodicLens::new(E3PeriodicConfig {
            options: PeriodicOptions {
                target_hour: Some(14),
                ..PeriodicOptions::default()
            },
            reference_time: 0,
        });

        assert_eq!(lens.scores(14 * 3_600).unwrap()[0], 1.0);
        assert_eq!(lens.scores(2 * 3_600).unwrap()[0], 0.0);
        assert_eq!(lens.scores(8 * 3_600).unwrap()[0], 0.5);
    }

    #[test]
    fn dow_scores_are_monday_zero() {
        let lens = E3PeriodicLens::new(E3PeriodicConfig {
            options: PeriodicOptions {
                target_day_of_week: Some(1),
                ..PeriodicOptions::default()
            },
            reference_time: 0,
        });
        let tuesday = 5 * 86_400;
        let friday = 86_400;

        assert_eq!(lens.scores(tuesday).unwrap()[1], 1.0);
        assert!((lens.scores(friday).unwrap()[1] - (1.0 - 3.0 / 3.5)).abs() < 1e-6);
    }

    #[test]
    fn edges_and_bad_input_fail_closed() {
        let no_target = E3PeriodicLens::new(E3PeriodicConfig {
            options: PeriodicOptions::default(),
            reference_time: 14 * 3_600,
        });
        let bad_target = E3PeriodicLens::new(E3PeriodicConfig {
            options: PeriodicOptions {
                target_hour: Some(99),
                ..PeriodicOptions::default()
            },
            reference_time: 0,
        });

        assert_eq!(
            no_target.measure(&ts(0)).unwrap().as_dense().unwrap()[0],
            1.0
        );
        assert_eq!(
            bad_target.measure(&ts(0)).unwrap_err().code,
            "CALYX_LENS_DIM_MISMATCH"
        );
        assert_eq!(
            no_target
                .measure(&Input::new(Modality::Structured, vec![1]))
                .unwrap_err()
                .code,
            "CALYX_LENS_DIM_MISMATCH"
        );
    }
}
