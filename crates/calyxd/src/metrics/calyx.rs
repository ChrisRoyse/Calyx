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

use prometheus::{
    GaugeVec, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Registry, TextEncoder,
};

use crate::verify::VerifyRestoreReport;
use crate::vram::VramAuditReport;

use super::ChainVerifyMetrics;
use super::hazards::HazardGauges;
use super::zfs::{ZfsIntegrityMetrics, ZfsIntegritySnapshot};

mod init;
#[cfg(test)]
mod tests;

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
    vram_audit_resident_mib: IntGaugeVec,
    vram_audit_budget_mib: IntGaugeVec,
    vram_audit_device_total_mib: IntGaugeVec,
    vram_audit_headroom_mib: IntGaugeVec,
    verify_restore_ok: IntGaugeVec,
    verify_restore_chain_intact: IntGaugeVec,
    verify_restore_last_run_timestamp: IntGaugeVec,
    verify_restore_constellation_count: IntGaugeVec,
    verify_restore_anchor_count: IntGaugeVec,
    verify_restore_ledger_entry_count: IntGaugeVec,
    verify_restore_wal_bytes_present: IntGaugeVec,
    hazards: HazardGauges,
    zfs: ZfsIntegrityMetrics,
}

impl CalyxMetrics {
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
            self.verify_restore_ok.with_label_values(&[vault]).set(0);
            self.verify_restore_chain_intact
                .with_label_values(&[vault])
                .set(0);
            self.verify_restore_last_run_timestamp
                .with_label_values(&[vault])
                .set(0);
            self.verify_restore_constellation_count
                .with_label_values(&[vault])
                .set(0);
            self.verify_restore_anchor_count
                .with_label_values(&[vault])
                .set(0);
            self.verify_restore_ledger_entry_count
                .with_label_values(&[vault])
                .set(0);
            self.verify_restore_wal_bytes_present
                .with_label_values(&[vault])
                .set(0);
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

    /// Records the live NVML startup audit. The unlabeled compatibility gauges
    /// are updated alongside the labeled audit gauges consumed by dashboards.
    pub fn record_vram_budget_audit(&self, vault: &str, panel: &str, audit: &VramAuditReport) {
        let resident_mib = i64::from(audit.tei_used_mib);
        let budget_mib = i64::from(audit.calyx_budget_mib);
        let device_total_mib = i64::from(audit.device_total_mib);
        let headroom_mib = i64::from(
            audit
                .device_total_mib
                .saturating_sub(audit.tei_used_mib)
                .saturating_sub(audit.calyx_budget_mib),
        );
        self.set_vram_budget(resident_mib, budget_mib);
        self.vram_audit_resident_mib
            .with_label_values(&[vault, panel])
            .set(resident_mib);
        self.vram_audit_budget_mib
            .with_label_values(&[vault, panel])
            .set(budget_mib);
        self.vram_audit_device_total_mib
            .with_label_values(&[vault, panel])
            .set(device_total_mib);
        self.vram_audit_headroom_mib
            .with_label_values(&[vault, panel])
            .set(headroom_mib);
    }

    /// Records the zero-write restore verification read-back used at startup.
    pub fn record_verify_restore(&self, vault: &str, report: &VerifyRestoreReport, now_secs: i64) {
        self.verify_restore_ok
            .with_label_values(&[vault])
            .set(i64::from(report.success()));
        self.verify_restore_chain_intact
            .with_label_values(&[vault])
            .set(i64::from(report.chain_intact));
        self.verify_restore_last_run_timestamp
            .with_label_values(&[vault])
            .set(now_secs);
        self.verify_restore_constellation_count
            .with_label_values(&[vault])
            .set(u64_to_i64(report.constellation_count));
        self.verify_restore_anchor_count
            .with_label_values(&[vault])
            .set(u64_to_i64(report.anchor_count));
        self.verify_restore_ledger_entry_count
            .with_label_values(&[vault])
            .set(u64_to_i64(report.ledger_entry_count));
        self.verify_restore_wal_bytes_present
            .with_label_values(&[vault])
            .set(u64_to_i64(report.wal_bytes_present));
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

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
impl CalyxMetrics {
    /// MetricFamily count for the T03 registry (excludes the chain-verify
    /// registry, which is gathered separately in `encode_text`).
    pub fn family_count(&self) -> usize {
        self.registry.gather().len()
    }
}
