use prometheus::{IntCounterVec, Opts, Registry, TextEncoder};

const ENDPOINTS: [&str; 3] = [
    "learner_signals_batches",
    "interventions_decide",
    "intervention_outcomes",
];
const STATUS_CODES: [&str; 9] = [
    "200", "201", "400", "401", "403", "404", "405", "409", "500",
];
const WRITE_KINDS: [&str; 3] = [
    "learner_signal_batch",
    "intervention_decision",
    "intervention_outcome",
];
const WRITE_RESULTS: [&str; 4] = ["accepted", "duplicate", "rejected", "error"];

/// Bounded-cardinality Prometheus counters for the learner-origin API.
pub struct OriginMetrics {
    registry: Registry,
    requests: IntCounterVec,
    writes: IntCounterVec,
}

impl OriginMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new(
                "calyx_origin_requests_total",
                "Worker-origin learner API requests by bounded endpoint and status code",
            ),
            &["endpoint", "status"],
        )
        .expect("define calyx_origin_requests_total");
        let writes = IntCounterVec::new(
            Opts::new(
                "calyx_origin_writes_total",
                "Worker-origin learner Aster write outcomes by bounded kind",
            ),
            &["kind", "result"],
        )
        .expect("define calyx_origin_writes_total");
        registry
            .register(Box::new(requests.clone()))
            .expect("register origin request counter");
        registry
            .register(Box::new(writes.clone()))
            .expect("register origin write counter");
        let metrics = Self {
            registry,
            requests,
            writes,
        };
        for endpoint in ENDPOINTS {
            for status in STATUS_CODES {
                metrics.requests.with_label_values(&[endpoint, status]);
            }
        }
        for kind in WRITE_KINDS {
            for result in WRITE_RESULTS {
                metrics.writes.with_label_values(&[kind, result]);
            }
        }
        metrics
    }

    pub fn record_request(&self, endpoint: &'static str, status: &'static str) {
        self.requests.with_label_values(&[endpoint, status]).inc();
    }

    pub fn record_write(&self, kind: &'static str, result: &'static str) {
        self.writes.with_label_values(&[kind, result]).inc();
    }

    pub fn encode_text(&self) -> Result<String, String> {
        let mut buffer = String::new();
        TextEncoder::new()
            .encode_utf8(&self.registry.gather(), &mut buffer)
            .map_err(|error| format!("encode origin prometheus text format: {error}"))?;
        Ok(buffer)
    }

    #[cfg(test)]
    pub fn request_count(&self, endpoint: &'static str, status: &'static str) -> u64 {
        self.requests.with_label_values(&[endpoint, status]).get()
    }

    #[cfg(test)]
    pub fn write_count(&self, kind: &'static str, result: &'static str) -> u64 {
        self.writes.with_label_values(&[kind, result]).get()
    }
}

impl Default for OriginMetrics {
    fn default() -> Self {
        Self::new()
    }
}
