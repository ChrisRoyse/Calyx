//! Prometheus registry for the Ledger chain-verify gauge family (PH66, issue #602).
//!
//! `calyx_ledger_chain_verify_ok{vault}` is 1 only when the most recent
//! `verify_chain` run proved the chain `Intact`. Broken, corrupt, and
//! verify-error outcomes all emit 0 — an unverifiable chain is never "ok"
//! (fail-closed). Companion series follow Prometheus batch-observation best
//! practice: a last-run unix timestamp (alert on `time() - ts` to catch a
//! wedged verify loop) and a per-outcome run counter whose label values are
//! pre-initialized at registration so the series exist from the first scrape.

mod calyx;
mod hazards;
mod zfs;

pub use calyx::{CalyxMetrics, SearchStrategy};
pub use hazards::HAZARD_IDS;
pub use zfs::{
    DEFAULT_ZFS_DATASETS, ZFS_SCRUB_MAX_AGE_SECONDS, ZfsDatasetChecksum, ZfsIntegritySnapshot,
    ZfsPoolIntegrity, collect_default_zfs_integrity, collect_zfs_integrity,
};

use prometheus::{IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder};

/// Outcome of one chain-verify run, in `calyx_ledger` verdict order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Chain proven intact end-to-end; `entries` rows verified.
    Intact { entries: u64 },
    /// Hash-chain link or entry-hash mismatch at `at_seq`.
    Broken { at_seq: u64 },
    /// Ledger CF integrity violation (missing/undecodable/mismatched row).
    Corrupt { at_seq: u64, reason: String },
    /// The verify run itself failed (store unreadable, scan error, ...).
    Error { detail: String },
}

impl VerifyOutcome {
    /// Stable `outcome` label value.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Intact { .. } => "intact",
            Self::Broken { .. } => "broken",
            Self::Corrupt { .. } => "corrupt",
            Self::Error { .. } => "error",
        }
    }

    /// Gauge value: 1 only for a proven-intact chain.
    pub fn ok_value(&self) -> i64 {
        match self {
            Self::Intact { .. } => 1,
            _ => 0,
        }
    }
}

/// All `outcome` label values, pre-initialized at registration.
pub const OUTCOME_LABELS: [&str; 4] = ["intact", "broken", "corrupt", "error"];

/// Registered handles for the chain-verify metric family.
pub struct ChainVerifyMetrics {
    registry: Registry,
    ok: IntGaugeVec,
    last_run_timestamp: IntGaugeVec,
    entries: IntGaugeVec,
    runs_total: IntCounterVec,
}

impl ChainVerifyMetrics {
    /// Registers the family and pre-initializes every series for the given
    /// vault labels. Duplicate registration is a programming error and panics
    /// at init time (PH66 T03: never silently overwrite a metric).
    pub fn new(vault_labels: &[String]) -> Self {
        let registry = Registry::new();
        let ok = IntGaugeVec::new(
            Opts::new(
                "calyx_ledger_chain_verify_ok",
                "1 when the last ledger hash-chain verify run proved the chain intact; \
                 0 on broken, corrupt, or verify error (fail-closed)",
            ),
            &["vault"],
        )
        .expect("define calyx_ledger_chain_verify_ok");
        let last_run_timestamp = IntGaugeVec::new(
            Opts::new(
                "calyx_ledger_chain_verify_last_run_timestamp_seconds",
                "Unix timestamp of the last completed chain-verify run for the vault",
            ),
            &["vault"],
        )
        .expect("define calyx_ledger_chain_verify_last_run_timestamp_seconds");
        let entries = IntGaugeVec::new(
            Opts::new(
                "calyx_ledger_chain_verify_entries",
                "Ledger entries proven intact by the last chain-verify run (0 unless intact)",
            ),
            &["vault"],
        )
        .expect("define calyx_ledger_chain_verify_entries");
        let runs_total = IntCounterVec::new(
            Opts::new(
                "calyx_ledger_chain_verify_runs_total",
                "Chain-verify runs by outcome (intact|broken|corrupt|error)",
            ),
            &["vault", "outcome"],
        )
        .expect("define calyx_ledger_chain_verify_runs_total");

        for collector in [&ok, &last_run_timestamp, &entries] {
            registry
                .register(Box::new(collector.clone()))
                .expect("register chain-verify gauge (duplicate registration is a bug)");
        }
        registry
            .register(Box::new(runs_total.clone()))
            .expect("register chain-verify counter (duplicate registration is a bug)");

        let metrics = Self {
            registry,
            ok,
            last_run_timestamp,
            entries,
            runs_total,
        };
        for vault in vault_labels {
            metrics.ok.with_label_values(&[vault]).set(0);
            metrics
                .last_run_timestamp
                .with_label_values(&[vault])
                .set(0);
            metrics.entries.with_label_values(&[vault]).set(0);
            for outcome in OUTCOME_LABELS {
                metrics.runs_total.with_label_values(&[vault, outcome]);
            }
        }
        metrics
    }

    /// Records one completed verify run for `vault` at unix time `now_secs`.
    pub fn record(&self, vault: &str, outcome: &VerifyOutcome, now_secs: i64) {
        self.ok.with_label_values(&[vault]).set(outcome.ok_value());
        self.last_run_timestamp
            .with_label_values(&[vault])
            .set(now_secs);
        let verified_entries = match outcome {
            VerifyOutcome::Intact { entries } => i64::try_from(*entries).unwrap_or(i64::MAX),
            _ => 0,
        };
        self.entries
            .with_label_values(&[vault])
            .set(verified_entries);
        self.runs_total
            .with_label_values(&[vault, outcome.label()])
            .inc();
    }

    /// Encodes the registry in Prometheus text exposition format v0.0.4.
    pub fn encode_text(&self) -> Result<String, String> {
        let mut buffer = String::new();
        TextEncoder::new()
            .encode_utf8(&self.registry.gather(), &mut buffer)
            .map_err(|error| format!("encode prometheus text format: {error}"))?;
        Ok(buffer)
    }
}

/// In-process readback accessors (the production readback is the encoded
/// `/metrics` text itself). Public API rather than test-gated because the
/// binary's verify-loop tests, which depend on this library crate, exercise
/// them across the crate boundary.
impl ChainVerifyMetrics {
    pub fn family_count(&self) -> usize {
        self.registry.gather().len()
    }

    pub fn ok_value_for(&self, vault: &str) -> i64 {
        self.ok.with_label_values(&[vault]).get()
    }

    pub fn runs_for(&self, vault: &str, outcome: &str) -> u64 {
        self.runs_total.with_label_values(&[vault, outcome]).get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Vec<String> {
        vec!["/data/vault-a".to_string()]
    }

    #[test]
    fn new_registers_four_families_preinitialized() {
        let metrics = ChainVerifyMetrics::new(&labels());
        assert_eq!(metrics.family_count(), 4);
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_ledger_chain_verify_ok{vault=\"/data/vault-a\"} 0"));
        for outcome in OUTCOME_LABELS {
            assert!(
                text.contains(&format!(
                    "calyx_ledger_chain_verify_runs_total{{outcome=\"{outcome}\",vault=\"/data/vault-a\"}} 0"
                )),
                "missing pre-initialized outcome {outcome} in:\n{text}"
            );
        }
    }

    #[test]
    fn intact_outcome_sets_ok_one_and_entries() {
        let metrics = ChainVerifyMetrics::new(&labels());
        metrics.record(
            "/data/vault-a",
            &VerifyOutcome::Intact { entries: 7 },
            1_770_000_000,
        );
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_ledger_chain_verify_ok{vault=\"/data/vault-a\"} 1"));
        assert!(text.contains("calyx_ledger_chain_verify_entries{vault=\"/data/vault-a\"} 7"));
        assert!(text.contains(
            "calyx_ledger_chain_verify_last_run_timestamp_seconds{vault=\"/data/vault-a\"} 1770000000"
        ));
        assert_eq!(metrics.runs_for("/data/vault-a", "intact"), 1);
    }

    #[test]
    fn broken_corrupt_and_error_outcomes_all_emit_zero() {
        let metrics = ChainVerifyMetrics::new(&labels());
        metrics.record(
            "/data/vault-a",
            &VerifyOutcome::Intact { entries: 3 },
            1_770_000_000,
        );
        for outcome in [
            VerifyOutcome::Broken { at_seq: 1 },
            VerifyOutcome::Corrupt {
                at_seq: 2,
                reason: "missing row".to_string(),
            },
            VerifyOutcome::Error {
                detail: "store unreadable".to_string(),
            },
        ] {
            metrics.record("/data/vault-a", &outcome, 1_770_000_100);
            assert_eq!(
                metrics.ok_value_for("/data/vault-a"),
                0,
                "outcome {} must emit gauge 0",
                outcome.label()
            );
        }
        assert_eq!(metrics.runs_for("/data/vault-a", "broken"), 1);
        assert_eq!(metrics.runs_for("/data/vault-a", "corrupt"), 1);
        assert_eq!(metrics.runs_for("/data/vault-a", "error"), 1);
    }

    #[test]
    fn duplicate_registration_in_one_registry_panics() {
        let registry = Registry::new();
        let first = IntGaugeVec::new(
            Opts::new("calyx_ledger_chain_verify_ok", "help"),
            &["vault"],
        )
        .unwrap();
        registry.register(Box::new(first.clone())).unwrap();
        let second = IntGaugeVec::new(
            Opts::new("calyx_ledger_chain_verify_ok", "help"),
            &["vault"],
        )
        .unwrap();
        assert!(registry.register(Box::new(second)).is_err());
    }
}
