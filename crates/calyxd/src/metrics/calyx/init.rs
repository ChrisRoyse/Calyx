use prometheus::core::Collector;
use prometheus::{
    GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};

use super::super::zfs::DEFAULT_ZFS_DATASETS;
use super::*;

/// Latency histogram buckets in seconds, spanning sub-millisecond to 10s.
const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Registers `collector` into `registry`, returning the handle. A duplicate
/// family name is a programming error and panics at init — never silently
/// overwrite a metric (PH66 T03 fail-closed).
fn register<C: Collector + Clone + 'static>(registry: &Registry, collector: C) -> C {
    registry
        .register(Box::new(collector.clone()))
        .expect("register metric family (duplicate registration is a bug)");
    collector
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
        let vram_audit_resident_mib = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_vram_budget_audit_resident_mib",
                    "NVML resident GPU footprint at the daemon VRAM audit, in MiB",
                ),
                &["vault", "panel"],
            )
            .expect("define calyx_vram_budget_audit_resident_mib"),
        );
        let vram_audit_budget_mib = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_vram_budget_audit_budget_mib",
                    "Configured Calyx daemon VRAM budget at the audit, in MiB",
                ),
                &["vault", "panel"],
            )
            .expect("define calyx_vram_budget_audit_budget_mib"),
        );
        let vram_audit_device_total_mib = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_vram_budget_audit_device_total_mib",
                    "NVML device total VRAM observed at the daemon audit, in MiB",
                ),
                &["vault", "panel"],
            )
            .expect("define calyx_vram_budget_audit_device_total_mib"),
        );
        let vram_audit_headroom_mib = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_vram_budget_audit_headroom_mib",
                    "Device VRAM headroom remaining after resident footprint plus configured Calyx budget, in MiB",
                ),
                &["vault", "panel"],
            )
            .expect("define calyx_vram_budget_audit_headroom_mib"),
        );
        let verify_restore_ok = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_ok",
                    "1 when the last verify-restore read-back succeeded; 0 otherwise",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_ok"),
        );
        let verify_restore_chain_intact = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_chain_intact",
                    "1 when verify-restore proved the Ledger chain intact; 0 otherwise",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_chain_intact"),
        );
        let verify_restore_last_run_timestamp = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_last_run_timestamp_seconds",
                    "Unix timestamp of the last completed verify-restore read-back",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_last_run_timestamp_seconds"),
        );
        let verify_restore_constellation_count = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_constellation_count",
                    "Constellations physically read by the last verify-restore run",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_constellation_count"),
        );
        let verify_restore_anchor_count = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_anchor_count",
                    "Anchors physically read by the last verify-restore run",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_anchor_count"),
        );
        let verify_restore_ledger_entry_count = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_ledger_entry_count",
                    "Ledger rows physically read by the last verify-restore run",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_ledger_entry_count"),
        );
        let verify_restore_wal_bytes_present = register(
            &registry,
            IntGaugeVec::new(
                Opts::new(
                    "calyx_verify_restore_wal_bytes_present",
                    "WAL bytes present in the vault at the last verify-restore run",
                ),
                &["vault"],
            )
            .expect("define calyx_verify_restore_wal_bytes_present"),
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
            vram_audit_resident_mib,
            vram_audit_budget_mib,
            vram_audit_device_total_mib,
            vram_audit_headroom_mib,
            verify_restore_ok,
            verify_restore_chain_intact,
            verify_restore_last_run_timestamp,
            verify_restore_constellation_count,
            verify_restore_anchor_count,
            verify_restore_ledger_entry_count,
            verify_restore_wal_bytes_present,
            hazards,
            zfs,
        };
        metrics.preinitialize(vault_labels);
        metrics
    }
}
