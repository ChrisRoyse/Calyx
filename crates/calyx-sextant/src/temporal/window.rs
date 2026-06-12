use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::Result;
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::error::{CALYX_TEMPORAL_INVALID_WINDOW, sextant_error};
use crate::hit::Hit;

const SECS_PER_HOUR: u64 = 3_600;
const SECS_PER_DAY: u64 = 86_400;

pub trait Clock: Send + Sync {
    fn now_secs(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> i64 {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_secs();
        i64::try_from(secs).unwrap_or(i64::MAX)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedClock {
    pub secs: i64,
}

impl FixedClock {
    pub const fn new(secs: i64) -> Self {
        Self { secs }
    }
}

impl Clock for FixedClock {
    fn now_secs(&self) -> i64 {
        self.secs
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TimeWindow {
    pub start_secs: i64,
    pub end_secs: i64,
}

impl TimeWindow {
    pub fn new(start_secs: i64, end_secs: i64) -> Result<Self> {
        let window = Self {
            start_secs,
            end_secs,
        };
        window.validate()?;
        Ok(window)
    }

    pub fn all() -> Self {
        Self {
            start_secs: i64::MIN,
            end_secs: i64::MAX,
        }
    }

    pub fn last_hours(n: u64, clock: &dyn Clock) -> Result<Self> {
        Self::last_span(n, SECS_PER_HOUR, clock)
    }

    pub fn last_days(n: u64, clock: &dyn Clock) -> Result<Self> {
        Self::last_span(n, SECS_PER_DAY, clock)
    }

    pub fn contains(&self, event_time_secs: i64) -> bool {
        self.start_secs <= event_time_secs && event_time_secs < self.end_secs
    }

    pub fn validate(&self) -> Result<()> {
        if self.start_secs >= self.end_secs {
            return Err(invalid_window("temporal window must be non-empty"));
        }
        Ok(())
    }

    fn is_all(&self) -> bool {
        self.start_secs == i64::MIN && self.end_secs == i64::MAX
    }

    fn last_span(n: u64, unit_secs: u64, clock: &dyn Clock) -> Result<Self> {
        if n == 0 {
            return Err(invalid_window("temporal window span must be non-zero"));
        }
        let span_u64 = n
            .checked_mul(unit_secs)
            .ok_or_else(|| invalid_window("temporal window span overflow"))?;
        let span = i64::try_from(span_u64)
            .map_err(|_| invalid_window("temporal window span exceeds i64"))?;
        let end = clock.now_secs();
        let start = end
            .checked_sub(span)
            .ok_or_else(|| invalid_window("temporal window start underflow"))?;
        Self::new(start, end)
    }
}

impl<'de> Deserialize<'de> for TimeWindow {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            start_secs: i64,
            end_secs: i64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.start_secs, wire.end_secs).map_err(de::Error::custom)
    }
}

pub fn filter_hits_by_window(hits: Vec<Hit>, window: &TimeWindow) -> Vec<Hit> {
    if window.is_all() {
        return hits;
    }
    hits.into_iter()
        .filter(|hit| hit_matches_window(hit, window))
        .collect()
}

/// Non-consuming count with identical semantics to [`filter_hits_by_window`].
pub fn count_hits_in_window(hits: &[Hit], window: &TimeWindow) -> usize {
    if window.is_all() {
        return hits.len();
    }
    hits.iter()
        .filter(|hit| hit_matches_window(hit, window))
        .count()
}

fn hit_matches_window(hit: &Hit, window: &TimeWindow) -> bool {
    hit.event_time_secs
        .is_some_and(|event_time_secs| window.contains(event_time_secs))
}

fn invalid_window(message: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_TEMPORAL_INVALID_WINDOW, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{CxId, LedgerRef};
    use proptest::prelude::*;

    use crate::hit::{FreshnessTag, ProvenanceSource};

    #[test]
    fn last_hours_uses_injected_clock_seconds() {
        let window = TimeWindow::last_hours(1, &FixedClock::new(1_000_000)).expect("window");
        assert_eq!(window.start_secs, 996_400);
        assert_eq!(window.end_secs, 1_000_000);
    }

    #[test]
    fn last_days_uses_injected_clock_seconds() {
        let window = TimeWindow::last_days(1, &FixedClock::new(1_000_000)).expect("window");
        assert_eq!(window.start_secs, 913_600);
        assert_eq!(window.end_secs, 1_000_000);
    }

    #[test]
    fn contains_uses_half_open_bounds() {
        let window = TimeWindow::new(100, 200).expect("window");
        assert!(window.contains(150));
        assert!(!window.contains(200));
        assert!(!window.contains(99));
    }

    #[test]
    fn filter_hits_by_window_preserves_retained_order() {
        let window = TimeWindow::new(100, 200).expect("window");
        let hits = [50, 120, 170, 250, 300]
            .into_iter()
            .enumerate()
            .map(|(index, time)| hit(index as u8, Some(time)))
            .collect();

        let filtered = filter_hits_by_window(hits, &window);

        assert_eq!(ids(&filtered), vec![1, 2]);
        assert_eq!(
            filtered
                .iter()
                .map(|hit| hit.event_time_secs)
                .collect::<Vec<_>>(),
            vec![Some(120), Some(170)]
        );
    }

    #[test]
    fn all_window_preserves_hits_without_event_times() {
        let hits = vec![hit(1, None), hit(2, Some(123))];
        let filtered = filter_hits_by_window(hits.clone(), &TimeWindow::all());
        assert_eq!(filtered, hits);
    }

    #[test]
    fn invalid_windows_fail_closed() {
        let zero =
            TimeWindow::last_hours(0, &FixedClock::new(1_000_000)).expect_err("zero span rejected");
        assert_eq!(zero.code, CALYX_TEMPORAL_INVALID_WINDOW);

        let reversed = TimeWindow::new(200, 100).expect_err("reversed rejected");
        assert_eq!(reversed.code, CALYX_TEMPORAL_INVALID_WINDOW);

        let overflow = TimeWindow::last_hours(u64::MAX, &FixedClock::new(1_000_000))
            .expect_err("overflow rejected");
        assert_eq!(overflow.code, CALYX_TEMPORAL_INVALID_WINDOW);
    }

    proptest! {
        #[test]
        fn filter_hits_by_window_returns_ordered_subset(
            times in proptest::collection::vec(-10_000_i64..10_000, 0..32),
            start in -10_000_i64..9_000,
            width in 1_i64..1_000,
        ) {
            let end = start + width;
            let window = TimeWindow::new(start, end).expect("valid generated window");
            let hits = times
                .iter()
                .enumerate()
                .map(|(index, time)| hit(index as u8, Some(*time)))
                .collect::<Vec<_>>();

            let filtered = filter_hits_by_window(hits.clone(), &window);
            let expected = hits
                .into_iter()
                .filter(|hit| window.contains(hit.event_time_secs.expect("time")))
                .collect::<Vec<_>>();

            prop_assert_eq!(filtered, expected);
        }
    }

    fn hit(seed: u8, event_time_secs: Option<i64>) -> Hit {
        Hit {
            cx_id: CxId::from_bytes([seed; 16]),
            score: 1.0 - f32::from(seed) * 0.001,
            rank: seed as usize + 1,
            event_time_secs,
            temporal_scores: None,
            causal_confidence: crate::temporal::CausalConfidence::Absent,
            causal_gate: None,
            per_lens: Vec::new(),
            cross_terms_used: false,
            guard: None,
            provenance: LedgerRef {
                seq: seed as u64,
                hash: [seed; 32],
            },
            provenance_source: ProvenanceSource::Stub,
            freshness: FreshnessTag::fresh(0),
            explain: None,
        }
    }

    fn ids(hits: &[Hit]) -> Vec<u8> {
        hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
    }
}
