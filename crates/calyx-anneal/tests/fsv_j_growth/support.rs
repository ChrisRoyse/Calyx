use std::path::Path;
use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, ArtifactKey, ArtifactPtr, AsterAnnealLedgerStore, AsterRollbackStorage,
    GoodhartState, HeldOutSet, IntelligenceGradient, JMetricSources, JObjectiveContext,
    RollbackStore, WardGtau, compute_j, intelligence_report,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, Result as CalyxResult};
use calyx_ledger::{ActorId, LedgerAppender};
use serde_json::json;
use sha2::{Digest, Sha256};

#[allow(clippy::duplicate_mod)]
#[path = "../fsv_support/mod.rs"]
mod fsv_support;
pub(crate) use fsv_support::write_json;

pub(crate) const FSV_TS: u64 = 1_785_800_428;
const VAULT_SALT: &[u8] = b"calyx-anneal-intelligence-report";

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct Metrics {
    mutual_info_panel_anchor: f64,
    n_eff: f64,
    panel_sufficiency: f64,
    kernel_recall: f64,
    oracle_accuracy: f64,
    mistake_rate: f64,
    compression_yield: f64,
    coverage: f64,
    dpi_ceiling: f64,
    provisional_count: usize,
}

impl Metrics {
    pub(crate) fn report() -> Self {
        Self {
            mutual_info_panel_anchor: 1.5,
            n_eff: 7.0,
            panel_sufficiency: 1.1,
            kernel_recall: 0.8,
            oracle_accuracy: 0.75,
            mistake_rate: 0.05,
            compression_yield: 0.6,
            coverage: 0.4,
            dpi_ceiling: 2.0,
            provisional_count: 0,
        }
    }

    fn growth(step: u64) -> Self {
        let step = step as f64;
        Self {
            mutual_info_panel_anchor: 0.8 + step * 0.001,
            n_eff: 6.0 + step * 0.001,
            panel_sufficiency: 0.6 + step * 0.0005,
            kernel_recall: 0.7,
            oracle_accuracy: 0.65,
            mistake_rate: 0.10,
            compression_yield: 0.45,
            coverage: 0.35,
            dpi_ceiling: 3.0,
            provisional_count: 0,
        }
    }

    pub(crate) fn gamed_train() -> Self {
        Self {
            mutual_info_panel_anchor: 1.95,
            ..Self::report()
        }
    }

    pub(crate) fn heldout_flat() -> Self {
        Self::report()
    }
}

impl JMetricSources for Metrics {
    fn mutual_info_panel_anchor(&self) -> f64 {
        self.mutual_info_panel_anchor
    }

    fn n_eff(&self) -> f64 {
        self.n_eff
    }

    fn panel_sufficiency(&self, _domain: &str) -> f64 {
        self.panel_sufficiency
    }

    fn kernel_recall(&self) -> f64 {
        self.kernel_recall
    }

    fn oracle_accuracy(&self) -> f64 {
        self.oracle_accuracy
    }

    fn mistake_rate(&self) -> f64 {
        self.mistake_rate
    }

    fn compression_yield(&self) -> f64 {
        self.compression_yield
    }

    fn coverage(&self) -> f64 {
        self.coverage
    }

    fn dpi_ceiling(&self) -> f64 {
        self.dpi_ceiling
    }

    fn provisional_count(&self) -> usize {
        self.provisional_count
    }
}

pub(crate) struct StaticWard {
    pub(crate) in_region: f64,
}

impl WardGtau for StaticWard {
    fn in_region_fraction(&self, _held_out_set: &HeldOutSet) -> CalyxResult<Option<f64>> {
        Ok(Some(self.in_region))
    }
}

pub(crate) fn open_vault(path: &Path) -> AsterVault {
    AsterVault::new_durable(
        path,
        fsv_support::vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

pub(crate) fn seed_base_cf(vault: &AsterVault) {
    vault
        .write_cf(
            ColumnFamily::Base,
            b"issue428-base-sentinel".to_vec(),
            b"base-row-stays-byte-identical".to_vec(),
        )
        .unwrap();
    vault.flush().unwrap();
}

pub(crate) fn cf_sha256(vault: &AsterVault, cf: ColumnFamily) -> String {
    let mut rows = vault.scan_cf_at(vault.latest_seq(), cf).unwrap();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (key, value) in rows {
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(&key);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(&value);
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| row_json(&(key, value)))
        .collect()
}

pub(crate) fn report_for_growth_step(
    step: u64,
    clock: &FixedClock,
) -> calyx_anneal::IntelligenceReport {
    let metrics = Metrics::growth(step);
    let context = JObjectiveContext::new("issue428", 8);
    let j = compute_j(&context, &metrics).unwrap();
    let gradient = IntelligenceGradient::new(j, Arc::new(*clock));
    intelligence_report(
        &context,
        &metrics,
        &gradient,
        &GoodhartState::default(),
        None,
        clock,
    )
}

pub(crate) fn rollback_gamed_candidate(
    vault: &AsterVault,
    report: &calyx_anneal::GoodhartReport,
) -> calyx_anneal::RollbackReadback {
    let clock = FixedClock::new(FSV_TS + 2);
    let store = RollbackStore::open(&clock, 428, AsterRollbackStorage::new(vault)).unwrap();
    let key = ArtifactKey::ConfigCache(*blake3::hash(b"issue428-live-panel").as_bytes());
    let prior = ArtifactPtr::ConfigCacheKeyHash([0x11; 32]);
    let candidate = ArtifactPtr::ConfigCacheKeyHash([0x22; 32]);
    store.install_live_ptr(key.clone(), prior).unwrap();
    let change_id = store
        .prepare_with_description(key, candidate, "PH48 gamed correlated lens candidate")
        .unwrap();
    store.promote(change_id).unwrap();
    assert!(!report.passed);
    store.rollback(change_id).unwrap();
    store.readback(change_id).unwrap()
}

pub(crate) fn open_ledger(
    vault: &AsterVault,
) -> AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let appender = LedgerAppender::open(
        AsterAnnealLedgerStore::new(vault),
        FixedClock::new(FSV_TS + 2),
    )
    .unwrap();
    AnnealLedger::new(appender, ActorId::Service("calyx-issue428-fsv".to_string())).unwrap()
}

fn row_json((key, value): &(Vec<u8>, Vec<u8>)) -> serde_json::Value {
    json!({
        "key": hex(key),
        "value_len": value.len(),
        "value_prefix": hex_prefix(value, 96)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_prefix(bytes: &[u8], limit: usize) -> String {
    hex(&bytes[..bytes.len().min(limit)])
}
