use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_core::{AbsentReason, CalyxError, Input, LensId, Result, SlotVector};
use serde::{Deserialize, Serialize};

pub const DEFAULT_INGEST_MICROBATCH_CAP_BYTES: usize = 16 * 1024 * 1024;
pub const INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES: usize = 64;
const DEFAULT_HIGH_WATER_NUMERATOR: usize = 3;
const DEFAULT_HIGH_WATER_DENOMINATOR: usize = 4;
const DEFAULT_BREAKER_FAILURE_THRESHOLD: u32 = 3;
const DEFAULT_BREAKER_OPEN_MS: u64 = 30_000;

/// Hard limits and breaker policy for ingest microbatch admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestMicrobatchConfig {
    pub cap_bytes: usize,
    pub high_water_bytes: usize,
    pub breaker_failure_threshold: u32,
    pub breaker_open_ms: u64,
}

impl Default for IngestMicrobatchConfig {
    fn default() -> Self {
        Self::new(DEFAULT_INGEST_MICROBATCH_CAP_BYTES)
    }
}

impl IngestMicrobatchConfig {
    pub fn new(cap_bytes: usize) -> Self {
        let high_water_bytes =
            cap_bytes.saturating_mul(DEFAULT_HIGH_WATER_NUMERATOR) / DEFAULT_HIGH_WATER_DENOMINATOR;
        Self {
            cap_bytes,
            high_water_bytes,
            breaker_failure_threshold: DEFAULT_BREAKER_FAILURE_THRESHOLD,
            breaker_open_ms: DEFAULT_BREAKER_OPEN_MS,
        }
    }

    pub const fn with_high_water(mut self, high_water_bytes: usize) -> Self {
        self.high_water_bytes = high_water_bytes;
        self
    }

    pub const fn with_breaker(mut self, failure_threshold: u32, open_ms: u64) -> Self {
        self.breaker_failure_threshold = failure_threshold;
        self.breaker_open_ms = open_ms;
        self
    }

    fn normalized(mut self) -> Self {
        self.high_water_bytes = self.high_water_bytes.min(self.cap_bytes);
        self.breaker_failure_threshold = self.breaker_failure_threshold.max(1);
        self.breaker_open_ms = self.breaker_open_ms.max(1);
        self
    }
}

/// Shared admission controller for bounded ingest microbatches.
#[derive(Clone, Debug)]
pub struct IngestMicrobatchController {
    inner: Arc<Mutex<IngestMicrobatchState>>,
}

impl Default for IngestMicrobatchController {
    fn default() -> Self {
        Self::new(IngestMicrobatchConfig::default())
    }
}

impl IngestMicrobatchController {
    pub fn new(config: IngestMicrobatchConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(IngestMicrobatchState::new(config.normalized()))),
        }
    }

    /// Attempts to reserve the bytes required by a microbatch.
    pub fn admit(&self, inputs: &[Input]) -> Result<IngestMicrobatchPermit> {
        let bytes = estimate_microbatch_bytes(inputs);
        let mut state = self.inner.lock().expect("ingest admission lock poisoned");
        let next = state.current_buffer_bytes.saturating_add(bytes);
        if next > state.config.cap_bytes {
            state.backpressure_total = state.backpressure_total.saturating_add(1);
            return Err(CalyxError::backpressure(format!(
                "ingest microbatch needs {bytes} bytes with {} already admitted, cap {}",
                state.current_buffer_bytes, state.config.cap_bytes
            )));
        }

        state.current_buffer_bytes = next;
        state.buffer_high_water_bytes = state.buffer_high_water_bytes.max(next);
        state.admitted_total = state.admitted_total.saturating_add(1);
        if next > state.config.high_water_bytes {
            state.high_water_events_total = state.high_water_events_total.saturating_add(1);
        }
        Ok(IngestMicrobatchPermit {
            inner: Arc::clone(&self.inner),
            bytes,
            released: false,
        })
    }

    pub fn measure_lens_batch<F>(
        &self,
        lens_id: LensId,
        inputs: &[Input],
        now_ms: u64,
        measure: F,
    ) -> Result<IngestLensOutcome>
    where
        F: FnOnce(&[Input]) -> Result<Vec<SlotVector>>,
    {
        if let Some(open_until_ms) = self.breaker_open_until(lens_id, now_ms) {
            return Ok(self.degraded_outcome(lens_id, inputs.len(), 0, None, Some(open_until_ms)));
        }

        let permit = self.admit(inputs)?;
        let buffer_bytes = permit.bytes();
        match measure(inputs) {
            Ok(vectors) => {
                if vectors.len() != inputs.len() {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "lens {lens_id} returned {} vectors for {} inputs",
                        vectors.len(),
                        inputs.len()
                    )));
                }
                self.record_lens_success(lens_id);
                Ok(IngestLensOutcome {
                    lens_id,
                    status: IngestLensOutcomeStatus::Measured,
                    vectors,
                    buffer_bytes,
                    error_code: None,
                    breaker_open_until_ms: None,
                })
            }
            Err(error) if is_lens_timeout_like(&error) => {
                let open_until_ms = self.record_lens_timeout(lens_id, now_ms);
                Ok(self.degraded_outcome(
                    lens_id,
                    inputs.len(),
                    buffer_bytes,
                    Some(error.code),
                    open_until_ms,
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub fn stats(&self) -> IngestMicrobatchStats {
        self.inner
            .lock()
            .expect("ingest admission lock poisoned")
            .stats()
    }

    pub fn metrics_text(&self) -> String {
        let stats = self.stats();
        format!(
            "# TYPE calyx_ingest_microbatch_buffer_bytes gauge\n\
             calyx_ingest_microbatch_buffer_bytes {}\n\
             # TYPE calyx_ingest_microbatch_cap_bytes gauge\n\
             calyx_ingest_microbatch_cap_bytes {}\n\
             # TYPE calyx_ingest_microbatch_high_water_bytes gauge\n\
             calyx_ingest_microbatch_high_water_bytes {}\n\
             # TYPE calyx_ingest_microbatch_admitted_total counter\n\
             calyx_ingest_microbatch_admitted_total {}\n\
             # TYPE calyx_ingest_microbatch_backpressure_total counter\n\
             calyx_ingest_microbatch_backpressure_total {}\n\
             # TYPE calyx_ingest_microbatch_high_water_events_total counter\n\
             calyx_ingest_microbatch_high_water_events_total {}\n\
             # TYPE calyx_ingest_lens_timeouts_total counter\n\
             calyx_ingest_lens_timeouts_total {}\n\
             # TYPE calyx_ingest_lens_breaker_trips_total counter\n\
             calyx_ingest_lens_breaker_trips_total {}\n\
             # TYPE calyx_ingest_lens_degraded_total counter\n\
             calyx_ingest_lens_degraded_total {}\n\
             # TYPE calyx_ingest_lens_open_breakers gauge\n\
             calyx_ingest_lens_open_breakers {}\n",
            stats.current_buffer_bytes,
            stats.cap_bytes,
            stats.buffer_high_water_bytes,
            stats.admitted_total,
            stats.backpressure_total,
            stats.high_water_events_total,
            stats.lens_timeouts_total,
            stats.breaker_trips_total,
            stats.degraded_lenses_total,
            stats.open_breaker_count
        )
    }

    pub fn panel_readout(
        &self,
        acknowledged_inputs: usize,
        outcomes: Vec<IngestLensOutcome>,
    ) -> IngestPanelReadout {
        IngestPanelReadout {
            acknowledged_inputs,
            outcomes,
            stats_after: self.stats(),
        }
    }

    fn breaker_open_until(&self, lens_id: LensId, now_ms: u64) -> Option<u64> {
        let state = self.inner.lock().expect("ingest admission lock poisoned");
        state
            .breakers
            .get(&lens_id)
            .and_then(|breaker| breaker.opened_until_ms)
            .filter(|opened_until| *opened_until > now_ms)
    }

    fn record_lens_success(&self, lens_id: LensId) {
        let mut state = self.inner.lock().expect("ingest admission lock poisoned");
        let recovered = if let Some(breaker) = state.breakers.get_mut(&lens_id) {
            let recovered = breaker.opened_until_ms.is_some();
            breaker.consecutive_failures = 0;
            breaker.opened_until_ms = None;
            recovered
        } else {
            false
        };
        if recovered {
            state.breaker_recoveries_total = state.breaker_recoveries_total.saturating_add(1);
        }
    }

    fn record_lens_timeout(&self, lens_id: LensId, now_ms: u64) -> Option<u64> {
        let mut state = self.inner.lock().expect("ingest admission lock poisoned");
        state.lens_timeouts_total = state.lens_timeouts_total.saturating_add(1);
        let threshold = state.config.breaker_failure_threshold;
        let open_ms = state.config.breaker_open_ms;
        let tripped;
        let open_until = {
            let breaker = state.breakers.entry(lens_id).or_default();
            breaker.consecutive_failures = breaker.consecutive_failures.saturating_add(1);
            if breaker.consecutive_failures < threshold {
                return None;
            }
            let open_until = now_ms.saturating_add(open_ms);
            let was_open = breaker
                .opened_until_ms
                .is_some_and(|opened_until| opened_until > now_ms);
            breaker.consecutive_failures = 0;
            breaker.opened_until_ms = Some(open_until);
            tripped = !was_open;
            open_until
        };
        if tripped {
            state.breaker_trips_total = state.breaker_trips_total.saturating_add(1);
        }
        Some(open_until)
    }
    fn degraded_outcome(
        &self,
        lens_id: LensId,
        input_len: usize,
        buffer_bytes: usize,
        error_code: Option<&'static str>,
        breaker_open_until_ms: Option<u64>,
    ) -> IngestLensOutcome {
        let mut state = self.inner.lock().expect("ingest admission lock poisoned");
        state.degraded_lenses_total = state.degraded_lenses_total.saturating_add(1);
        drop(state);
        IngestLensOutcome {
            lens_id,
            status: IngestLensOutcomeStatus::Degraded,
            vectors: absent_vectors(input_len),
            buffer_bytes,
            error_code: error_code.map(str::to_string),
            breaker_open_until_ms,
        }
    }
}

#[derive(Debug)]
pub struct IngestMicrobatchPermit {
    inner: Arc<Mutex<IngestMicrobatchState>>,
    bytes: usize,
    released: bool,
}

impl IngestMicrobatchPermit {
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut state = self.inner.lock().expect("ingest admission lock poisoned");
        state.current_buffer_bytes = state.current_buffer_bytes.saturating_sub(self.bytes);
        self.released = true;
    }
}

impl Drop for IngestMicrobatchPermit {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestMicrobatchStats {
    pub cap_bytes: usize,
    pub high_water_bytes: usize,
    pub current_buffer_bytes: usize,
    pub buffer_high_water_bytes: usize,
    pub admitted_total: u64,
    pub backpressure_total: u64,
    pub high_water_events_total: u64,
    pub lens_timeouts_total: u64,
    pub breaker_trips_total: u64,
    pub breaker_recoveries_total: u64,
    pub degraded_lenses_total: u64,
    pub open_breaker_count: usize,
    pub open_breakers: Vec<LensId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestLensOutcomeStatus {
    Measured,
    Degraded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestLensOutcome {
    pub lens_id: LensId,
    pub status: IngestLensOutcomeStatus,
    pub vectors: Vec<SlotVector>,
    pub buffer_bytes: usize,
    pub error_code: Option<String>,
    pub breaker_open_until_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestPanelReadout {
    pub acknowledged_inputs: usize,
    pub outcomes: Vec<IngestLensOutcome>,
    pub stats_after: IngestMicrobatchStats,
}

#[derive(Clone, Debug)]
struct IngestMicrobatchState {
    config: IngestMicrobatchConfig,
    current_buffer_bytes: usize,
    buffer_high_water_bytes: usize,
    admitted_total: u64,
    backpressure_total: u64,
    high_water_events_total: u64,
    lens_timeouts_total: u64,
    breaker_trips_total: u64,
    breaker_recoveries_total: u64,
    degraded_lenses_total: u64,
    breakers: BTreeMap<LensId, LensBreakerState>,
}

impl IngestMicrobatchState {
    fn new(config: IngestMicrobatchConfig) -> Self {
        Self {
            config,
            current_buffer_bytes: 0,
            buffer_high_water_bytes: 0,
            admitted_total: 0,
            backpressure_total: 0,
            high_water_events_total: 0,
            lens_timeouts_total: 0,
            breaker_trips_total: 0,
            breaker_recoveries_total: 0,
            degraded_lenses_total: 0,
            breakers: BTreeMap::new(),
        }
    }

    fn stats(&self) -> IngestMicrobatchStats {
        let open_breakers = self
            .breakers
            .iter()
            .filter_map(|(lens_id, breaker)| breaker.opened_until_ms.map(|_| *lens_id))
            .collect::<Vec<_>>();
        IngestMicrobatchStats {
            cap_bytes: self.config.cap_bytes,
            high_water_bytes: self.config.high_water_bytes,
            current_buffer_bytes: self.current_buffer_bytes,
            buffer_high_water_bytes: self.buffer_high_water_bytes,
            admitted_total: self.admitted_total,
            backpressure_total: self.backpressure_total,
            high_water_events_total: self.high_water_events_total,
            lens_timeouts_total: self.lens_timeouts_total,
            breaker_trips_total: self.breaker_trips_total,
            breaker_recoveries_total: self.breaker_recoveries_total,
            degraded_lenses_total: self.degraded_lenses_total,
            open_breaker_count: open_breakers.len(),
            open_breakers,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct LensBreakerState {
    consecutive_failures: u32,
    opened_until_ms: Option<u64>,
}

pub fn estimate_microbatch_bytes(inputs: &[Input]) -> usize {
    inputs.iter().fold(0usize, |total, input| {
        total
            .saturating_add(INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES)
            .saturating_add(input.bytes.len())
            .saturating_add(input.pointer.as_ref().map_or(0, |pointer| pointer.len()))
    })
}

fn absent_vectors(input_len: usize) -> Vec<SlotVector> {
    vec![
        SlotVector::Absent {
            reason: AbsentReason::LensUnavailable,
        };
        input_len
    ]
}

fn is_lens_timeout_like(error: &CalyxError) -> bool {
    error.code == "CALYX_LENS_UNREACHABLE"
}

#[cfg(test)]
mod tests;
