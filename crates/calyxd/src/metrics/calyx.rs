//! `CalyxMetrics`: the full daemon `/metrics` surface (PH66 T03, issue #538).
//!
//! Aggregates every named metric family the Grafana dashboard and Alertmanager
//! rules reference: ingest/search latency + throughput, the recall tripwire,
//! guard FAR/FRR, DDA `n_eff` + kernel recall ratio, Anneal A/B counters, the
//! VRAM budget gauges, and one gauge per PH59 hazard. The chain-verify family
//! (issue #602) is composed in unchanged via [`ChainVerifyMetrics`]; this struct
//! owns a second registry for the T03 families and concatenates the two
//! exposition blocks. Family names are disjoint, so the merged text is a valid
//! Prometheus v0.0.4 document.
//!
//! Series whose label sets are known at startup (vault, search strategy, ingest
//! status) are pre-initialized so the families exist from the very first scrape
//! and `rate()` has no startup gap. Genuinely dynamic-cardinality families
//! (guard slot, assay panel, kernel scope, Anneal experiment) appear on first
//! observation — pre-seeding them would mean inventing fake label values.

use std::sync::Arc;

use prometheus::core::Collector;
use prometheus::{
    GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};

use super::ChainVerifyMetrics;
use super::hazards::HazardGauges;
use super::zfs::{DEFAULT_ZFS_DATASETS, ZfsIntegrityMetrics, ZfsIntegritySnapshot};

/// Latency histogram buckets in seconds, spanning sub-millisecond to 10s.
const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Retrieval strategies, each a `strategy` label value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStrategy {
    SingleLens,
    Rrf,
    WeightedRrf,
    Sparse,
}

impl SearchStrategy {
    /// All strategies, used to pre-initialize the search families.
    pub const ALL: [SearchStrategy; 4] = [
        SearchStrategy::SingleLens,
        SearchStrategy::Rrf,
        SearchStrategy::WeightedRrf,
        SearchStrategy::Sparse,
    ];

    /// Stable `strategy` label value.
    pub fn label(&self) -> &'static str {
        match self {
            SearchStrategy::SingleLens => "single_lens",
            SearchStrategy::Rrf => "rrf",
            SearchStrategy::WeightedRrf => "weighted_rrf",
            SearchStrategy::Sparse => "sparse",
        }
    }
}

/// Outcome `status` label value for ingest/search counters.
fn status_label(ok: bool) -> &'static str {
    if ok { "ok" } else { "err" }
}

/// Registers `collector` into `registry`, returning the handle. A duplicate
/// family name is a programming error and panics at init — never silently
/// overwrite a metric (PH66 T03 fail-closed).
fn register<C: Collector + Clone + 'static>(registry: &Registry, collector: C) -> C {
    registry
        .register(Box::new(collector.clone()))
        .expect("register metric family (duplicate registration is a bug)");
    collector
}

/// The complete daemon metric surface served at `GET /metrics`.
pub struct CalyxMetrics {
    chain: Arc<ChainVerifyMetrics>,
    registry: Registry,
    ingest_duration: HistogramVec,
    ingest_total: IntCounterVec,
    search_duration: HistogramVec,
    search_recall_tripwire: IntGaugeVec,
    search_total: IntCounterVec,
    guard_far: GaugeVec,
    guard_frr: GaugeVec,
    assay_n_eff: GaugeVec,
    kernel_recall_ratio: GaugeVec,
    anneal_ab_variant_total: IntCounterVec,
    anneal_ab_improvement_ratio: GaugeVec,
    vram_used_mib: IntGauge,
    vram_limit_mib: IntGauge,
    hazards: HazardGauges,
    zfs: ZfsIntegrityMetrics,
}

impl CalyxMetrics {
    /// Registers every T03 family into a fresh registry and pre-initializes the
    /// statically-known series for `vault_labels`. The chain-verify family is
    /// composed in via `chain` and emitted alongside in [`Self::encode_text`].
    pub fn new(chain: Arc<ChainVerifyMetrics>, vault_labels: &[String]) -> Self {
        let registry = Registry::new();
        let ingest_duration = register(
            &registry,
            HistogramVec::new(
                HistogramOpts::new(
                    "calyx_ingest_duration_seconds",
                    "Ingest batch wall-clock latency in seconds",
                )
                .buckets(LATENCY_BUCKETS.to_vec()),
                &["vault"],
            )
            .expect("define calyx_ingest_duration_seconds"),
        );
        let ingest_total = register(
            &registry,
            IntCounterVec::new(
                Opts::new("calyx_ingest_total", "Ingest operations by outcome status"),
                &["vault", "status"],
            )
            .expect("define calyx_ingest_total"),
        );
        let search_duration = register(
            &registry,
            HistogramVec::new(
                HistogramOpts::new(
                    "calyx_search_duration_seconds",
                    "Search latency in seconds by retrieval strategy",
                )
                .buckets(LATENCY_BUCKETS.to_vec()),
                &["vault", "strategy"],
            )
            .expect("define calyx_search_duration_seconds"),
        );
        let search_recall_tripwire = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_search_recall_tripwire",
                    "1 when measured recall is at or above threshold, 0 when the tripwire has \
                     fired (recall regression)",
                ),
                &["vault"],
            )
            .expect("define calyx_search_recall_tripwire"),
        );
        let search_total = register(
            &registry,
            IntCounterVec::new(
                Opts::new(
                    "calyx_search_total",
                    "Search operations by strategy and outcome status",
                ),
                &["vault", "strategy", "status"],
            )
            .expect("define calyx_search_total"),
        );
        let guard_far = register(
            &registry,
            GaugeVec::new(
                Opts::new(
                    "calyx_guard_far",
                    "Guard false-accept rate per required slot",
                ),
                &["vault", "slot"],
            )
            .expect("define calyx_guard_far"),
        );
        let guard_frr = register(
            &registry,
            GaugeVec::new(
                Opts::new(
                    "calyx_guard_frr",
                    "Guard false-reject rate per required slot",
                ),
                &["vault", "slot"],
            )
            .expect("define calyx_guard_frr"),
        );
        let assay_n_eff = register(
            &registry,
            GaugeVec::new(
                Opts::new(
                    "calyx_assay_n_eff",
                    "DDA effective sample size (n_eff) per panel",
                ),
                &["vault", "panel"],
            )
            .expect("define calyx_assay_n_eff"),
        );
        let kernel_recall_ratio = register(
            &registry,
            GaugeVec::new(
                Opts::new(
                    "calyx_kernel_recall_ratio",
                    "Kernel-answer recall ratio versus brute force per scope",
                ),
                &["vault", "scope"],
            )
            .expect("define calyx_kernel_recall_ratio"),
        );
        let anneal_ab_variant_total = register(
            &registry,
            IntCounterVec::new(
                Opts::new(
                    "calyx_anneal_ab_variant_total",
                    "Anneal A/B experiment exposures by variant",
                ),
                &["experiment", "variant"],
            )
            .expect("define calyx_anneal_ab_variant_total"),
        );
        let anneal_ab_improvement_ratio = register(
            &registry,
            GaugeVec::new(
                Opts::new(
                    "calyx_anneal_ab_improvement_ratio",
                    "Anneal A/B measured improvement ratio of treatment over control",
                ),
                &["experiment"],
            )
            .expect("define calyx_anneal_ab_improvement_ratio"),
        );
        let vram_used_mib = register(
            &registry,
            IntGauge::new(
                "calyx_vram_budget_used_mib",
                "VRAM budget currently used, in MiB",
            )
            .expect("define calyx_vram_budget_used_mib"),
        );
        let vram_limit_mib = register(
            &registry,
            IntGauge::new("calyx_vram_budget_limit_mib", "VRAM budget ceiling, in MiB")
                .expect("define calyx_vram_budget_limit_mib"),
        );
        let hazards = HazardGauges::register(&registry);
        let zfs = ZfsIntegrityMetrics::register(&registry, &DEFAULT_ZFS_DATASETS);

        let metrics = Self {
            chain,
            registry,
            ingest_duration,
            ingest_total,
            search_duration,
            search_recall_tripwire,
            search_total,
            guard_far,
            guard_frr,
            assay_n_eff,
            kernel_recall_ratio,
            anneal_ab_variant_total,
            anneal_ab_improvement_ratio,
            vram_used_mib,
            vram_limit_mib,
            hazards,
            zfs,
        };
        metrics.preinitialize(vault_labels);
        metrics
    }

    /// Materializes the series whose labels are known at startup so the families
    /// exist from the first scrape. The recall tripwire starts at 1 (healthy)
    /// per vault: a real sub-threshold measurement drives it to 0; starting at 0
    /// would page the operator on every idle startup before any search has run.
    fn preinitialize(&self, vault_labels: &[String]) {
        for vault in vault_labels {
            let _ = self.ingest_duration.with_label_values(&[vault]);
            for status in ["ok", "err"] {
                self.ingest_total.with_label_values(&[vault, status]);
            }
            self.search_recall_tripwire
                .with_label_values(&[vault])
                .set(1);
            for strategy in SearchStrategy::ALL {
                let _ = self
                    .search_duration
                    .with_label_values(&[vault, strategy.label()]);
                for status in ["ok", "err"] {
                    self.search_total
                        .with_label_values(&[vault, strategy.label(), status]);
                }
            }
        }
    }

    /// Records one ingest operation: latency sample + outcome counter.
    pub fn observe_ingest(&self, vault: &str, duration_secs: f64, ok: bool) {
        self.ingest_duration
            .with_label_values(&[vault])
            .observe(duration_secs);
        self.ingest_total
            .with_label_values(&[vault, status_label(ok)])
            .inc();
    }

    /// Records one search operation under `strategy`: latency + outcome counter.
    pub fn observe_search(
        &self,
        vault: &str,
        strategy: SearchStrategy,
        duration_secs: f64,
        ok: bool,
    ) {
        self.search_duration
            .with_label_values(&[vault, strategy.label()])
            .observe(duration_secs);
        self.search_total
            .with_label_values(&[vault, strategy.label(), status_label(ok)])
            .inc();
    }

    /// Sets the recall tripwire for `vault` (true = recall ≥ threshold).
    pub fn set_recall_tripwire(&self, vault: &str, ok: bool) {
        self.search_recall_tripwire
            .with_label_values(&[vault])
            .set(i64::from(ok));
    }

    /// Sets guard false-accept/false-reject rates for one slot.
    pub fn set_guard_rates(&self, vault: &str, slot: &str, far: f64, frr: f64) {
        self.guard_far.with_label_values(&[vault, slot]).set(far);
        self.guard_frr.with_label_values(&[vault, slot]).set(frr);
    }

    /// Sets the DDA effective sample size for one panel.
    pub fn set_assay_n_eff(&self, vault: &str, panel: &str, n_eff: f64) {
        self.assay_n_eff
            .with_label_values(&[vault, panel])
            .set(n_eff);
    }

    /// Sets the kernel recall ratio for one scope.
    pub fn set_kernel_recall_ratio(&self, vault: &str, scope: &str, ratio: f64) {
        self.kernel_recall_ratio
            .with_label_values(&[vault, scope])
            .set(ratio);
    }

    /// Records one Anneal A/B exposure of `variant` in `experiment`.
    pub fn record_anneal_exposure(&self, experiment: &str, variant: &str) {
        self.anneal_ab_variant_total
            .with_label_values(&[experiment, variant])
            .inc();
    }

    /// Sets the measured A/B improvement ratio for `experiment`.
    pub fn set_anneal_improvement(&self, experiment: &str, ratio: f64) {
        self.anneal_ab_improvement_ratio
            .with_label_values(&[experiment])
            .set(ratio);
    }

    /// Sets the VRAM budget used/limit gauges (MiB).
    pub fn set_vram_budget(&self, used_mib: i64, limit_mib: i64) {
        self.vram_used_mib.set(used_mib);
        self.vram_limit_mib.set(limit_mib);
    }

    /// Sets one PH59 hazard's state. An unknown hazard id is a fail-closed error.
    pub fn set_hazard(&self, hazard_id: &str, triggered: bool) -> Result<(), String> {
        self.hazards.set(hazard_id, triggered)
    }

    pub fn record_zfs_integrity(&self, snapshot: &ZfsIntegritySnapshot) {
        self.zfs.record(snapshot);
    }

    /// Encodes the full surface in Prometheus text exposition format v0.0.4:
    /// the chain-verify families first, then the T03 families.
    pub fn encode_text(&self) -> Result<String, String> {
        let mut buffer = self.chain.encode_text()?;
        let mut own = String::new();
        TextEncoder::new()
            .encode_utf8(&self.registry.gather(), &mut own)
            .map_err(|error| format!("encode prometheus text format: {error}"))?;
        buffer.push_str(&own);
        Ok(buffer)
    }
}

#[cfg(test)]
impl CalyxMetrics {
    /// MetricFamily count for the T03 registry (excludes the chain-verify
    /// registry, which is gathered separately in `encode_text`).
    pub fn family_count(&self) -> usize {
        self.registry.gather().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Arc<ChainVerifyMetrics> {
        Arc::new(ChainVerifyMetrics::new(&["/data/vault-a".to_string()]))
    }

    fn metrics() -> CalyxMetrics {
        CalyxMetrics::new(chain(), &["/data/vault-a".to_string()])
    }

    #[test]
    fn new_registers_at_least_30_preinitialized_families() {
        let metrics = metrics();
        // 13 T03 families (2 ingest, 3 search, 2 guard*, 2 assay/kernel*,
        // 2 anneal*, 2 vram) + 25 hazards. The guard/assay/kernel/anneal Vec
        // families have no series until observed, so the live count after
        // pre-init is the always-present ones: 2 vram + 25 hazard + 5
        // vault-seeded ingest/search = 32.
        assert!(
            metrics.family_count() >= 30,
            "expected >= 30 families, got {}",
            metrics.family_count()
        );
        let text = metrics.encode_text().unwrap();
        // Chain-verify family is composed in.
        assert!(text.contains("calyx_ledger_chain_verify_ok"));
        // Recall tripwire pre-initialized healthy.
        assert!(text.contains("calyx_search_recall_tripwire{vault=\"/data/vault-a\"} 1"));
    }

    #[test]
    fn ingest_observation_increments_counter_and_histogram() {
        let metrics = metrics();
        metrics.observe_ingest("/data/vault-a", 0.150, true);
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_ingest_total{status=\"ok\",vault=\"/data/vault-a\"} 1"));
        assert!(text.contains("calyx_ingest_duration_seconds_count{vault=\"/data/vault-a\"} 1"));
        // 0.150s falls in the le="0.25" bucket but not le="0.1".
        assert!(text.contains(
            "calyx_ingest_duration_seconds_bucket{vault=\"/data/vault-a\",le=\"0.25\"} 1"
        ));
        assert!(text.contains(
            "calyx_ingest_duration_seconds_bucket{vault=\"/data/vault-a\",le=\"0.1\"} 0"
        ));
    }

    #[test]
    fn recall_tripwire_tripped_emits_zero() {
        let metrics = metrics();
        metrics.set_recall_tripwire("/data/vault-a", false);
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_search_recall_tripwire{vault=\"/data/vault-a\"} 0"));
    }

    #[test]
    fn vram_budget_exact_text_match() {
        let metrics = metrics();
        metrics.set_vram_budget(4096, 8192);
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_vram_budget_used_mib 4096"));
        assert!(text.contains("calyx_vram_budget_limit_mib 8192"));
    }

    #[test]
    fn search_strategy_and_guard_families_appear_on_record() {
        let metrics = metrics();
        metrics.observe_search("/data/vault-a", SearchStrategy::WeightedRrf, 0.02, true);
        metrics.set_guard_rates("/data/vault-a", "subject", 0.01, 0.02);
        metrics.set_assay_n_eff("/data/vault-a", "default", 128.0);
        let text = metrics.encode_text().unwrap();
        assert!(text.contains(
            "calyx_search_total{status=\"ok\",strategy=\"weighted_rrf\",vault=\"/data/vault-a\"} 1"
        ));
        assert!(text.contains("calyx_guard_far{slot=\"subject\",vault=\"/data/vault-a\"} 0.01"));
        assert!(text.contains("calyx_guard_frr{slot=\"subject\",vault=\"/data/vault-a\"} 0.02"));
        assert!(text.contains("calyx_assay_n_eff{panel=\"default\",vault=\"/data/vault-a\"} 128"));
    }

    #[test]
    fn all_25_hazards_present_and_zero_at_init() {
        let metrics = metrics();
        let text = metrics.encode_text().unwrap();
        let hazard_lines: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("calyx_hazard_"))
            .collect();
        assert_eq!(hazard_lines.len(), 25, "expected 25 hazard value lines");
        for line in &hazard_lines {
            assert!(line.ends_with(" 0"), "hazard not zero at init: {line}");
        }
    }

    #[test]
    fn set_hazard_unknown_is_fail_closed() {
        let metrics = metrics();
        assert!(metrics.set_hazard("nope", true).is_err());
        metrics.set_hazard("disk_full", true).unwrap();
        let text = metrics.encode_text().unwrap();
        assert!(text.contains("calyx_hazard_disk_full{hazard=\"disk_full\"} 1"));
    }
}
