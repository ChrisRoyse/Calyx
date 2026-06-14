//! Retention horizon guard for PH72 time-travel reads.

use std::time::Duration;

use calyx_core::{CalyxError, Clock, Result, Ts};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const CALYX_TIMETRAVEL_BEFORE_HORIZON: &str = "CALYX_TIMETRAVEL_BEFORE_HORIZON";

/// Vault-level lower bound for historical `as_of(t)` reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RetentionHorizon {
    Rolling {
        #[serde(rename = "min_age_secs", with = "duration_secs")]
        min_age: Duration,
    },
    Absolute {
        horizon_millis: u64,
    },
    #[default]
    None,
}

impl RetentionHorizon {
    pub const fn none() -> Self {
        Self::None
    }

    pub const fn absolute(horizon_millis: u64) -> Self {
        Self::Absolute { horizon_millis }
    }

    pub const fn rolling(min_age: Duration) -> Self {
        Self::Rolling { min_age }
    }

    pub fn validate(&self) -> Result<()> {
        if let Self::Rolling { min_age } = self
            && min_age.subsec_nanos() != 0
        {
            return Err(CalyxError {
                code: "CALYX_TIMETRAVEL_INVALID_RETENTION_HORIZON",
                message: "rolling retention horizon must use whole-second precision".to_string(),
                remediation: "round the rolling horizon to whole seconds before persisting it",
            });
        }
        Ok(())
    }

    pub fn effective_horizon_millis(&self, clock: &dyn Clock) -> Option<u64> {
        self.effective_horizon_millis_at(clock.now())
    }

    pub fn safe_to_gc_before_millis(&self, clock: &dyn Clock) -> Option<u64> {
        self.effective_horizon_millis(clock)
    }

    pub(crate) fn effective_horizon_millis_at(&self, now_millis: Ts) -> Option<u64> {
        match self {
            Self::Rolling { min_age } => {
                Some(now_millis.saturating_sub(duration_millis_saturating(*min_age)))
            }
            Self::Absolute { horizon_millis } => Some(*horizon_millis),
            Self::None => None,
        }
    }
}

pub fn check_horizon(horizon: &RetentionHorizon, t_millis: u64, clock: &dyn Clock) -> Result<()> {
    check_horizon_at(horizon, t_millis, clock.now())
}

pub(crate) fn check_horizon_at(
    horizon: &RetentionHorizon,
    t_millis: u64,
    now_millis: Ts,
) -> Result<()> {
    if let Some(horizon_millis) = horizon.effective_horizon_millis_at(now_millis)
        && t_millis < horizon_millis
    {
        return Err(before_horizon(t_millis, horizon_millis));
    }
    Ok(())
}

pub fn before_horizon(requested_millis: u64, horizon_millis: u64) -> CalyxError {
    CalyxError {
        code: CALYX_TIMETRAVEL_BEFORE_HORIZON,
        message: format!(
            "requested_millis={requested_millis} is before horizon_millis={horizon_millis}"
        ),
        remediation: "query at or after the retention horizon, or lower the vault horizon if policy permits",
    }
}

fn duration_millis_saturating(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

mod duration_secs {
    use super::*;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Duration::from_secs(u64::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::FixedClock;
    use proptest::prelude::*;

    #[test]
    fn serde_shape_matches_manifest_contract() {
        let absolute = serde_json::to_string(&RetentionHorizon::absolute(5000)).unwrap();
        assert_eq!(absolute, r#"{"kind":"Absolute","horizon_millis":5000}"#);

        let rolling = serde_json::to_string(&RetentionHorizon::rolling(Duration::from_secs(60)))
            .expect("serialize rolling");
        assert_eq!(rolling, r#"{"kind":"Rolling","min_age_secs":60}"#);

        let none = serde_json::to_string(&RetentionHorizon::none()).unwrap();
        assert_eq!(none, r#"{"kind":"None"}"#);
        assert_eq!(
            serde_json::from_str::<RetentionHorizon>(&rolling).unwrap(),
            RetentionHorizon::rolling(Duration::from_secs(60))
        );
    }

    #[test]
    fn rolling_horizon_saturates_at_epoch() {
        let clock = FixedClock::new(10_000);
        let horizon = RetentionHorizon::rolling(Duration::from_secs(60));
        assert_eq!(horizon.effective_horizon_millis(&clock), Some(0));
    }

    #[test]
    fn safe_to_gc_uses_the_same_boundary_as_as_of() {
        let clock = FixedClock::new(10_000);
        let horizon = RetentionHorizon::rolling(Duration::from_secs(1));
        assert_eq!(horizon.safe_to_gc_before_millis(&clock), Some(9_000));
    }

    #[test]
    fn none_does_not_mask_no_data_errors() {
        let clock = FixedClock::new(10_000);
        assert!(check_horizon(&RetentionHorizon::none(), 0, &clock).is_ok());
    }

    #[test]
    fn rejects_subsecond_rolling_horizon_before_manifest_write() {
        let error = RetentionHorizon::rolling(Duration::from_millis(60))
            .validate()
            .unwrap_err();
        assert_eq!(error.code, "CALYX_TIMETRAVEL_INVALID_RETENTION_HORIZON");
    }

    proptest! {
        #[test]
        fn absolute_horizon_fails_exactly_before_boundary(
            horizon_millis in any::<u64>(),
            delta in 1_u64..=1_000_000,
        ) {
            let horizon = RetentionHorizon::absolute(horizon_millis);
            let before = horizon_millis.saturating_sub(delta);
            if before < horizon_millis {
                prop_assert_eq!(
                    check_horizon_at(&horizon, before, 0).unwrap_err().code,
                    CALYX_TIMETRAVEL_BEFORE_HORIZON
                );
            }
            prop_assert!(check_horizon_at(&horizon, horizon_millis, 0).is_ok());
            let after = horizon_millis.saturating_add(delta);
            prop_assert!(check_horizon_at(&horizon, after, 0).is_ok());
        }
    }
}
