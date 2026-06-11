//! Shared temporal policy contracts for post-retrieval boosting.

use crate::{CalyxError, Result};
use serde::{Deserialize, Deserializer, Serialize, de};

pub const CALYX_TEMPORAL_AP60_VIOLATION: &str = "CALYX_TEMPORAL_AP60_VIOLATION";
pub const CALYX_TEMPORAL_INVALID_BOOST_CONFIG: &str = "CALYX_TEMPORAL_INVALID_BOOST_CONFIG";
pub const CALYX_TEMPORAL_INVALID_PERIOD: &str = "CALYX_TEMPORAL_INVALID_PERIOD";
pub const CALYX_TEMPORAL_INVALID_WINDOW: &str = "CALYX_TEMPORAL_INVALID_WINDOW";
pub const CALYX_TEMPORAL_NEGATIVE_WEIGHT: &str = "CALYX_TEMPORAL_NEGATIVE_WEIGHT";
pub const CALYX_TEMPORAL_WEIGHT_SUM: &str = "CALYX_TEMPORAL_WEIGHT_SUM";

const WEIGHT_SUM_EPSILON: f32 = 1.0e-6;
const DEFAULT_HALF_LIFE_SECS: u64 = 3_600;
const DEFAULT_POST_RETRIEVAL_ALPHA: f32 = 0.10;
const MAX_POST_RETRIEVAL_ALPHA: f32 = 0.10;
const MAX_CAUSAL_MULTIPLIER: f32 = 10.0;
const DEFAULT_RECURRENCE_WEIGHT: f32 = 0.05;
const DEFAULT_MAX_RECURRENCE_BOOST: f32 = 0.10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecayFunction {
    Linear { max_age_secs: u64 },
    Exponential { half_life_secs: u64 },
    Step,
}

impl Default for DecayFunction {
    fn default() -> Self {
        Self::Exponential {
            half_life_secs: DEFAULT_HALF_LIFE_SECS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PeriodicOptions {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
    pub use_now: bool,
}

impl PeriodicOptions {
    pub fn new(target_hour: Option<u8>, target_day_of_week: Option<u8>) -> Result<Self> {
        let options = Self {
            target_hour,
            target_day_of_week,
            use_now: false,
        };
        options.validate()?;
        Ok(options)
    }

    pub fn from_query_time() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<()> {
        if self.target_hour.is_some_and(|hour| hour > 23) {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_PERIOD,
                "target_hour must be in 0..=23",
            ));
        }
        if self
            .target_day_of_week
            .is_some_and(|day_of_week| day_of_week > 6)
        {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_PERIOD,
                "target_day_of_week must be in 0..=6",
            ));
        }
        Ok(())
    }
}

impl Default for PeriodicOptions {
    fn default() -> Self {
        Self {
            target_hour: None,
            target_day_of_week: None,
            use_now: true,
        }
    }
}

impl<'de> Deserialize<'de> for PeriodicOptions {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            target_hour: Option<u8>,
            target_day_of_week: Option<u8>,
            #[serde(default)]
            use_now: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        let options = Self {
            target_hour: wire.target_hour,
            target_day_of_week: wire.target_day_of_week,
            use_now: wire.use_now,
        };
        options.validate().map_err(de::Error::custom)?;
        Ok(options)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiAnchorMode {
    First,
    Last,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceOptions {
    pub direction: SequenceDirection,
    pub multi_anchor_mode: MultiAnchorMode,
}

impl Default for SequenceOptions {
    fn default() -> Self {
        Self {
            direction: SequenceDirection::Forward,
            multi_anchor_mode: MultiAnchorMode::First,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FusionWeights {
    pub recency: f32,
    pub sequence: f32,
    pub periodic: f32,
}

impl FusionWeights {
    pub fn new(recency: f32, sequence: f32, periodic: f32) -> Result<Self> {
        let weights = Self {
            recency,
            sequence,
            periodic,
        };
        weights.validate()?;
        Ok(weights)
    }

    pub fn validate(&self) -> Result<()> {
        let sum = self.recency + self.sequence + self.periodic;
        if !self.recency.is_finite() || !self.sequence.is_finite() || !self.periodic.is_finite() {
            return Err(temporal_error(
                CALYX_TEMPORAL_WEIGHT_SUM,
                format!("temporal fusion weights must sum to 1.0, got {sum}"),
            ));
        }
        if self.recency < 0.0 || self.sequence < 0.0 || self.periodic < 0.0 {
            return Err(temporal_error(
                CALYX_TEMPORAL_NEGATIVE_WEIGHT,
                format!(
                    "temporal fusion weights must be non-negative: recency={} sequence={} periodic={}",
                    self.recency, self.sequence, self.periodic
                ),
            ));
        }
        if (sum - 1.0).abs() >= WEIGHT_SUM_EPSILON {
            return Err(temporal_error(
                CALYX_TEMPORAL_WEIGHT_SUM,
                format!("temporal fusion weights must sum to 1.0, got {sum}"),
            ));
        }
        Ok(())
    }
}

impl Default for FusionWeights {
    fn default() -> Self {
        Self {
            recency: 0.50,
            sequence: 0.35,
            periodic: 0.15,
        }
    }
}

impl<'de> Deserialize<'de> for FusionWeights {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            recency: f32,
            sequence: f32,
            periodic: f32,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.recency, wire.sequence, wire.periodic).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BoostConfig {
    pub post_retrieval_alpha: f32,
    pub causal_high_mult: f32,
    pub causal_low_mult: f32,
}

impl BoostConfig {
    pub fn new(
        post_retrieval_alpha: f32,
        causal_high_mult: f32,
        causal_low_mult: f32,
    ) -> Result<Self> {
        let config = Self {
            post_retrieval_alpha,
            causal_high_mult,
            causal_low_mult,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.post_retrieval_alpha.is_finite()
            || !(0.0..=MAX_POST_RETRIEVAL_ALPHA).contains(&self.post_retrieval_alpha)
        {
            return Err(temporal_error(
                CALYX_TEMPORAL_AP60_VIOLATION,
                format!(
                    "post_retrieval_alpha must be finite and in 0.0..={MAX_POST_RETRIEVAL_ALPHA}"
                ),
            ));
        }
        if !self.causal_high_mult.is_finite()
            || self.causal_high_mult <= 1.0
            || self.causal_high_mult > MAX_CAUSAL_MULTIPLIER
        {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
                format!("causal_high_mult must be finite and in 1.0..={MAX_CAUSAL_MULTIPLIER}"),
            ));
        }
        if !self.causal_low_mult.is_finite() || !(0.0..1.0).contains(&self.causal_low_mult) {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
                "causal_low_mult must be finite and in 0.0..1.0",
            ));
        }
        if self.causal_low_mult >= self.causal_high_mult {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
                "causal_low_mult must be less than causal_high_mult",
            ));
        }
        Ok(())
    }
}

impl Default for BoostConfig {
    fn default() -> Self {
        Self {
            post_retrieval_alpha: DEFAULT_POST_RETRIEVAL_ALPHA,
            causal_high_mult: 1.10,
            causal_low_mult: 0.85,
        }
    }
}

const fn default_post_retrieval_alpha() -> f32 {
    DEFAULT_POST_RETRIEVAL_ALPHA
}

impl<'de> Deserialize<'de> for BoostConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default = "default_post_retrieval_alpha")]
            post_retrieval_alpha: f32,
            causal_high_mult: f32,
            causal_low_mult: f32,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.post_retrieval_alpha,
            wire.causal_high_mult,
            wire.causal_low_mult,
        )
        .map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceBoostConfig {
    pub frequency_weight: f32,
    pub recency_weight: f32,
    pub max_recurrence_boost: f32,
}

impl RecurrenceBoostConfig {
    pub fn new(
        frequency_weight: f32,
        recency_weight: f32,
        max_recurrence_boost: f32,
    ) -> Result<Self> {
        let config = Self {
            frequency_weight,
            recency_weight,
            max_recurrence_boost,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let values = [
            self.frequency_weight,
            self.recency_weight,
            self.max_recurrence_boost,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
            || self.max_recurrence_boost > DEFAULT_MAX_RECURRENCE_BOOST
        {
            return Err(temporal_error(
                CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
                format!(
                    "recurrence boost weights must be finite, non-negative, and max <= {DEFAULT_MAX_RECURRENCE_BOOST}"
                ),
            ));
        }
        Ok(())
    }
}

impl Default for RecurrenceBoostConfig {
    fn default() -> Self {
        Self {
            frequency_weight: DEFAULT_RECURRENCE_WEIGHT,
            recency_weight: DEFAULT_RECURRENCE_WEIGHT,
            max_recurrence_boost: DEFAULT_MAX_RECURRENCE_BOOST,
        }
    }
}

fn default_recurrence_boost() -> Option<RecurrenceBoostConfig> {
    Some(RecurrenceBoostConfig::default())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TemporalPolicy {
    pub enabled: bool,
    pub decay: DecayFunction,
    pub periodic: PeriodicOptions,
    pub sequence: SequenceOptions,
    pub fusion_weights: FusionWeights,
    pub boost: BoostConfig,
    pub recurrence_boost: Option<RecurrenceBoostConfig>,
    pub never_dominant: bool,
}

impl TemporalPolicy {
    pub fn new(
        enabled: bool,
        decay: DecayFunction,
        periodic: PeriodicOptions,
        sequence: SequenceOptions,
        fusion_weights: FusionWeights,
        boost: BoostConfig,
        never_dominant: bool,
    ) -> Result<Self> {
        let policy = Self {
            enabled,
            decay,
            periodic,
            sequence,
            fusion_weights,
            boost,
            recurrence_boost: default_recurrence_boost(),
            never_dominant,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.never_dominant {
            return Err(temporal_error(
                CALYX_TEMPORAL_AP60_VIOLATION,
                "AP-60 requires temporal signals to remain post-retrieval and never dominant",
            ));
        }
        self.periodic.validate()?;
        self.fusion_weights.validate()?;
        if let Some(config) = &self.recurrence_boost {
            config.validate()?;
        }
        self.boost.validate()
    }
}

impl Default for TemporalPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            decay: DecayFunction::default(),
            periodic: PeriodicOptions::default(),
            sequence: SequenceOptions::default(),
            fusion_weights: FusionWeights::default(),
            boost: BoostConfig::default(),
            recurrence_boost: default_recurrence_boost(),
            never_dominant: true,
        }
    }
}

impl<'de> Deserialize<'de> for TemporalPolicy {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            enabled: bool,
            decay: DecayFunction,
            periodic: PeriodicOptions,
            sequence: SequenceOptions,
            fusion_weights: FusionWeights,
            boost: BoostConfig,
            #[serde(default = "default_recurrence_boost")]
            recurrence_boost: Option<RecurrenceBoostConfig>,
            never_dominant: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        let policy = Self {
            enabled: wire.enabled,
            decay: wire.decay,
            periodic: wire.periodic,
            sequence: wire.sequence,
            fusion_weights: wire.fusion_weights,
            boost: wire.boost,
            recurrence_boost: wire.recurrence_boost,
            never_dominant: wire.never_dominant,
        };
        policy.validate().map_err(de::Error::custom)?;
        Ok(policy)
    }
}

fn temporal_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    let remediation = match code {
        CALYX_TEMPORAL_AP60_VIOLATION => {
            "keep temporal signals post-retrieval only and never dominant"
        }
        CALYX_TEMPORAL_INVALID_BOOST_CONFIG => {
            "set post-retrieval alpha and causal multipliers within their valid ranges"
        }
        CALYX_TEMPORAL_INVALID_PERIOD => "set target_hour 0..=23 and day_of_week 0..=6",
        CALYX_TEMPORAL_INVALID_WINDOW => "set a non-empty temporal window within i64 bounds",
        CALYX_TEMPORAL_NEGATIVE_WEIGHT => {
            "use a convex temporal fusion blend with every component >= 0.0"
        }
        CALYX_TEMPORAL_WEIGHT_SUM => "normalize recency + sequence + periodic to exactly 1.0",
        _ => "inspect temporal policy",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
