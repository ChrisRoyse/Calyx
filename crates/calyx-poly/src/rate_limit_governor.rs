//! Central rate-limit governor for read-only feed capture.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::{PolyError, Result};

pub const RATE_LIMIT_GOVERNOR_SCHEMA_VERSION: &str = "poly.rate_limit_governor.v1";
pub const RATE_LIMIT_PERMITTED: &str = "POLY_RATE_LIMIT_PERMITTED";
pub const RATE_LIMIT_WAIT_REQUIRED: &str = "POLY_RATE_LIMIT_WAIT_REQUIRED";
pub const RATE_LIMIT_429_BACKOFF: &str = "POLY_RATE_LIMIT_429_BACKOFF";
pub const RATE_LIMIT_STATUS_RECORDED: &str = "POLY_RATE_LIMIT_STATUS_RECORDED";
pub const RATE_LIMIT_SUSTAINED_429: &str = "POLY_RATE_LIMIT_SUSTAINED_429";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RateLimitEndpoint {
    pub source: String,
    pub endpoint: String,
    pub method: String,
}

impl RateLimitEndpoint {
    pub fn new(
        source: impl Into<String>,
        endpoint: impl Into<String>,
        method: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            endpoint: endpoint.into(),
            method: method.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointRateLimitPolicy {
    pub capacity: u32,
    pub refill_interval_ms: u64,
    pub max_429_retries: u32,
    pub backoff_initial_ms: u64,
    pub backoff_multiplier: u32,
    pub backoff_max_ms: u64,
}

impl EndpointRateLimitPolicy {
    pub fn conservative_default() -> Self {
        Self {
            capacity: 8,
            refill_interval_ms: 25,
            max_429_retries: 3,
            backoff_initial_ms: 1_000,
            backoff_multiplier: 2,
            backoff_max_ms: 15_000,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.capacity == 0 {
            return Err(policy_error("capacity must be greater than zero"));
        }
        if self.refill_interval_ms == 0 {
            return Err(policy_error("refill_interval_ms must be greater than zero"));
        }
        if self.backoff_initial_ms == 0 {
            return Err(policy_error("backoff_initial_ms must be greater than zero"));
        }
        if self.backoff_multiplier == 0 {
            return Err(policy_error("backoff_multiplier must be greater than zero"));
        }
        if self.backoff_max_ms < self.backoff_initial_ms {
            return Err(policy_error(
                "backoff_max_ms must be at least backoff_initial_ms",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub default: EndpointRateLimitPolicy,
    pub endpoints: BTreeMap<RateLimitEndpoint, EndpointRateLimitPolicy>,
}

impl RateLimitPolicy {
    pub fn conservative_default() -> Self {
        Self {
            default: EndpointRateLimitPolicy::conservative_default(),
            endpoints: BTreeMap::new(),
        }
    }

    pub fn with_endpoint(
        mut self,
        endpoint: RateLimitEndpoint,
        policy: EndpointRateLimitPolicy,
    ) -> Self {
        self.endpoints.insert(endpoint, policy);
        self
    }

    pub fn validate(&self) -> Result<()> {
        self.default.validate()?;
        for policy in self.endpoints.values() {
            policy.validate()?;
        }
        Ok(())
    }

    fn for_endpoint(&self, endpoint: &RateLimitEndpoint) -> &EndpointRateLimitPolicy {
        self.endpoints.get(endpoint).unwrap_or(&self.default)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointRateLimitState {
    pub tokens: u32,
    pub last_refill_ms: u64,
    pub consecutive_429s: u32,
    pub next_allowed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitDecision {
    pub endpoint: RateLimitEndpoint,
    pub requested_at_ms: u64,
    pub permitted_at_ms: u64,
    pub wait_ms: u64,
    pub tokens_before: u32,
    pub tokens_after: u32,
    pub next_allowed_at_ms: u64,
    pub status_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitResponseDecision {
    pub endpoint: RateLimitEndpoint,
    pub observed_at_ms: u64,
    pub status_code: Option<u16>,
    pub consecutive_429s: u32,
    pub retry: bool,
    pub fail_loud: bool,
    pub backoff_ms: u64,
    pub next_allowed_at_ms: u64,
    pub decision_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitGovernor {
    pub policy: RateLimitPolicy,
    pub states: BTreeMap<RateLimitEndpoint, EndpointRateLimitState>,
}

impl RateLimitGovernor {
    pub fn new(policy: RateLimitPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            states: BTreeMap::new(),
        })
    }

    pub fn reserve(
        &mut self,
        endpoint: &RateLimitEndpoint,
        now_ms: u64,
    ) -> Result<RateLimitDecision> {
        let policy = self.policy.for_endpoint(endpoint).clone();
        let state = self
            .states
            .entry(endpoint.clone())
            .or_insert_with(|| initial_state(&policy, now_ms));
        let mut permitted_at_ms = now_ms.max(state.next_allowed_at_ms);
        refill_state(state, &policy, permitted_at_ms);
        if state.tokens == 0 {
            permitted_at_ms = permitted_at_ms.max(state.last_refill_ms + policy.refill_interval_ms);
            refill_state(state, &policy, permitted_at_ms);
        }
        let tokens_before = state.tokens;
        state.tokens = state.tokens.saturating_sub(1);
        let wait_ms = permitted_at_ms.saturating_sub(now_ms);
        Ok(RateLimitDecision {
            endpoint: endpoint.clone(),
            requested_at_ms: now_ms,
            permitted_at_ms,
            wait_ms,
            tokens_before,
            tokens_after: state.tokens,
            next_allowed_at_ms: state.next_allowed_at_ms,
            status_code: if wait_ms == 0 {
                RATE_LIMIT_PERMITTED.to_string()
            } else {
                RATE_LIMIT_WAIT_REQUIRED.to_string()
            },
        })
    }

    pub fn observe_response(
        &mut self,
        endpoint: &RateLimitEndpoint,
        status_code: Option<u16>,
        now_ms: u64,
        retry_after_ms: Option<u64>,
    ) -> Result<RateLimitResponseDecision> {
        let policy = self.policy.for_endpoint(endpoint).clone();
        let state = self
            .states
            .entry(endpoint.clone())
            .or_insert_with(|| initial_state(&policy, now_ms));
        if status_code == Some(429) {
            state.consecutive_429s = state.consecutive_429s.saturating_add(1);
            let backoff_ms =
                retry_after_ms.unwrap_or_else(|| backoff_ms(&policy, state.consecutive_429s));
            state.next_allowed_at_ms = state.next_allowed_at_ms.max(now_ms + backoff_ms);
            let fail_loud = state.consecutive_429s > policy.max_429_retries;
            return Ok(RateLimitResponseDecision {
                endpoint: endpoint.clone(),
                observed_at_ms: now_ms,
                status_code,
                consecutive_429s: state.consecutive_429s,
                retry: !fail_loud,
                fail_loud,
                backoff_ms,
                next_allowed_at_ms: state.next_allowed_at_ms,
                decision_code: if fail_loud {
                    RATE_LIMIT_SUSTAINED_429.to_string()
                } else {
                    RATE_LIMIT_429_BACKOFF.to_string()
                },
            });
        }
        state.consecutive_429s = 0;
        Ok(RateLimitResponseDecision {
            endpoint: endpoint.clone(),
            observed_at_ms: now_ms,
            status_code,
            consecutive_429s: 0,
            retry: false,
            fail_loud: false,
            backoff_ms: 0,
            next_allowed_at_ms: state.next_allowed_at_ms,
            decision_code: RATE_LIMIT_STATUS_RECORDED.to_string(),
        })
    }
}

pub(crate) struct RateLimitedHttpOutcome<T> {
    pub(crate) status_code: Option<u16>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) value: T,
}

pub(crate) fn execute_rate_limited_request<T>(
    clock: &dyn Clock,
    endpoint: &RateLimitEndpoint,
    mut attempt: impl FnMut() -> Result<RateLimitedHttpOutcome<T>>,
) -> Result<T> {
    loop {
        let decision = reserve_global(endpoint, clock)?;
        if decision.wait_ms > 0 {
            thread::sleep(Duration::from_millis(decision.wait_ms));
        }
        let outcome = attempt()?;
        let response =
            observe_global(endpoint, outcome.status_code, outcome.retry_after_ms, clock)?;
        if response.fail_loud {
            return Err(PolyError::raw_source(
                RATE_LIMIT_SUSTAINED_429,
                format!(
                    "{} {} sustained {} consecutive 429 responses",
                    endpoint.method, endpoint.endpoint, response.consecutive_429s
                ),
            ));
        }
        if !response.retry {
            return Ok(outcome.value);
        }
    }
}

pub(crate) fn parse_retry_after_ms(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1_000))
}

fn reserve_global(endpoint: &RateLimitEndpoint, clock: &dyn Clock) -> Result<RateLimitDecision> {
    let now = now_unix_ms(clock);
    let mut governor = global_governor().lock().map_err(|_| {
        PolyError::raw_source(
            "POLY_RATE_LIMIT_LOCK_POISONED",
            "central rate-limit governor lock was poisoned",
        )
    })?;
    governor.reserve(endpoint, now)
}

fn observe_global(
    endpoint: &RateLimitEndpoint,
    status_code: Option<u16>,
    retry_after_ms: Option<u64>,
    clock: &dyn Clock,
) -> Result<RateLimitResponseDecision> {
    let now = now_unix_ms(clock);
    let mut governor = global_governor().lock().map_err(|_| {
        PolyError::raw_source(
            "POLY_RATE_LIMIT_LOCK_POISONED",
            "central rate-limit governor lock was poisoned",
        )
    })?;
    governor.observe_response(endpoint, status_code, now, retry_after_ms)
}

fn global_governor() -> &'static Mutex<RateLimitGovernor> {
    static GOVERNOR: OnceLock<Mutex<RateLimitGovernor>> = OnceLock::new();
    GOVERNOR.get_or_init(|| {
        Mutex::new(
            RateLimitGovernor::new(RateLimitPolicy::conservative_default())
                .expect("default rate-limit policy must validate"),
        )
    })
}

fn initial_state(policy: &EndpointRateLimitPolicy, now_ms: u64) -> EndpointRateLimitState {
    EndpointRateLimitState {
        tokens: policy.capacity,
        last_refill_ms: now_ms,
        consecutive_429s: 0,
        next_allowed_at_ms: 0,
    }
}

fn refill_state(state: &mut EndpointRateLimitState, policy: &EndpointRateLimitPolicy, now_ms: u64) {
    let elapsed = now_ms.saturating_sub(state.last_refill_ms);
    let refill_count = elapsed / policy.refill_interval_ms;
    if refill_count == 0 {
        return;
    }
    state.tokens = policy
        .capacity
        .min(state.tokens.saturating_add(refill_count as u32));
    state.last_refill_ms += refill_count * policy.refill_interval_ms;
}

fn backoff_ms(policy: &EndpointRateLimitPolicy, consecutive_429s: u32) -> u64 {
    let mut value = policy.backoff_initial_ms;
    for _ in 1..consecutive_429s {
        value = value
            .saturating_mul(policy.backoff_multiplier as u64)
            .min(policy.backoff_max_ms);
    }
    value
}

fn now_unix_ms(clock: &dyn Clock) -> u64 {
    clock.now()
}

fn policy_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source("POLY_RATE_LIMIT_POLICY_INVALID", message.into())
}

#[cfg(test)]
mod tests {
    use calyx_core::FixedClock;

    use super::now_unix_ms;

    #[test]
    fn issue1394_rate_limit_clock_is_injected() {
        assert_eq!(now_unix_ms(&FixedClock::new(1_234_567)), 1_234_567);
    }
}
