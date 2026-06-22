use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_anneal::{
    AsterGrowthCf, CALYX_ANNEAL_GROWTH_INVALID_SAMPLE, GrowthCf, GrowthCurve, GrowthSummary,
    IntelligenceReport, JTerms, JWeights, ReportAvailability, decode_growth_row,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, FixedClock};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

#[test]
fn rising_samples_compute_delta_summary_and_persist() {
    let storage = MemoryGrowthCf::default();
    let inspect = storage.clone();
    let mut curve = curve(storage, 10);

    record_values(&mut curve, &[1.0, 1.5, 2.0, 2.3, 2.8]).unwrap();

    let summary = curve.curve_summary_with_window(5);
    assert!(curve.is_rising(5));
    assert_eq!(summary.j_first, Some(1.0));
    assert_eq!(summary.j_last, Some(2.8));
    assert_eq!(summary.j_max, Some(2.8));
    assert!((summary.slope_recent.unwrap() - 0.44).abs() < 1e-12);
    assert_eq!(inspect.rows().len(), 5);
    assert_eq!(curve.samples().last().unwrap().delta_j, 0.5);
}

#[test]
fn falling_tail_is_not_rising() {
    let mut curve = curve(MemoryGrowthCf::default(), 10);

    record_values(&mut curve, &[1.0, 2.0, 1.5, 1.8, 1.6]).unwrap();

    assert!(!curve.is_rising(5));
    assert!(!curve.is_rising(2));
    assert!(curve.curve_summary_with_window(5).slope_recent.unwrap() > 0.0);
}

#[test]
fn plot_ascii_is_bounded_and_non_empty() {
    let mut curve = curve(MemoryGrowthCf::default(), 10);
    record_values(&mut curve, &[1.0, 1.5, 2.0, 2.3, 2.8]).unwrap();

    let plot = curve.plot_ascii(40, 5);

    assert!(plot.contains('*'));
    assert!(plot.lines().all(|line| line.len() <= 40));
}

#[test]
fn edges_single_sample_max_samples_and_storage_failure_are_explicit() {
    let mut single = curve(MemoryGrowthCf::default(), 10);
    record_values(&mut single, &[1.0]).unwrap();
    assert!(!single.is_rising(10));

    let mut capped = curve(MemoryGrowthCf::default(), 1);
    record_values(&mut capped, &[1.0, 3.0]).unwrap();
    assert_eq!(capped.len(), 1);
    assert_eq!(capped.samples().next().unwrap().j, 3.0);

    let storage = MemoryGrowthCf::default();
    storage.fail_next();
    let mut failing = curve(storage, 10);
    let error = failing.record_sample(&report(7.0), 3, vec!["sample".to_string()]);
    assert_eq!(error.unwrap_err().code, "CALYX_TEST_GROWTH_CF_FAIL");
    assert_eq!(failing.len(), 1);
    assert_eq!(failing.samples().next().unwrap().j, 7.0);
}

#[test]
fn invalid_report_fails_closed_without_zero_fill() {
    let mut curve = curve(MemoryGrowthCf::default(), 10);
    let error = curve
        .record_sample(&unavailable_report(), 0, Vec::new())
        .expect_err("unavailable report rejected");

    assert_eq!(error.code, CALYX_ANNEAL_GROWTH_INVALID_SAMPLE);
    assert!(curve.is_empty());
}

#[test]
fn persists_to_anneal_growth_cf_and_decodes() {
    let dir = temp_dir("persist");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"growth-test", VaultOptions::default())
        .expect("vault");
    {
        let storage = AsterGrowthCf::new(&vault);
        let mut curve =
            GrowthCurve::load_from_cf(storage, Arc::new(FixedClock::new(1_785_700_001)), 10)
                .expect("growth curve");
        record_values(&mut curve, &[1.0, 1.25, 1.75]).unwrap();
    }

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealGrowth)
        .expect("scan growth cf");
    let sst_dir = dir.join("cf").join("anneal_growth");

    assert_eq!(rows.len(), 3);
    assert_eq!(decode_growth_row(&rows[2].1).unwrap().1.j, 1.75);
    assert!(fs::read_dir(sst_dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            == Some("sst")
    }));
    cleanup(dir);
}

proptest! {
    #[test]
    fn summary_max_is_at_least_last(values in proptest::collection::vec(0.0f64..100.0, 1..32)) {
        let mut curve = curve(MemoryGrowthCf::default(), 64);
        record_values(&mut curve, &values).unwrap();
        let GrowthSummary { j_max, j_last, .. } = curve.curve_summary();

        prop_assert!(j_max.unwrap() >= j_last.unwrap());
    }
}

#[derive(Clone, Default)]
struct MemoryGrowthCf {
    rows: Arc<Mutex<GrowthRows>>,
    fail_next: Arc<AtomicBool>,
}

type GrowthRows = Vec<(Vec<u8>, Vec<u8>)>;

impl MemoryGrowthCf {
    fn rows(&self) -> GrowthRows {
        self.rows.lock().unwrap().clone()
    }

    fn fail_next(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

impl GrowthCf for MemoryGrowthCf {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> calyx_core::Result<()> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(CalyxError {
                code: "CALYX_TEST_GROWTH_CF_FAIL",
                message: "injected growth CF failure".to_string(),
                remediation: "test storage should retry",
            });
        }
        self.rows.lock().unwrap().push((key, value));
        Ok(())
    }

    fn scan(&self) -> calyx_core::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self.rows())
    }
}

fn curve(storage: MemoryGrowthCf, max_samples: usize) -> GrowthCurve<MemoryGrowthCf> {
    GrowthCurve::load_from_cf(
        storage,
        Arc::new(FixedClock::new(1_785_700_000)),
        max_samples,
    )
    .unwrap()
}

fn record_values<S>(curve: &mut GrowthCurve<S>, values: &[f64]) -> calyx_core::Result<()>
where
    S: GrowthCf,
{
    for (index, value) in values.iter().enumerate() {
        curve.record_sample(&report(*value), index as u64, vec![format!("step-{index}")])?;
    }
    Ok(())
}

fn report(j: f64) -> IntelligenceReport {
    IntelligenceReport {
        j,
        terms: terms(j),
        weights: JWeights::default(),
        dpi_ceiling: j.abs() + 1.0,
        dpi_headroom: 1.0,
        provisional_excluded: 0,
        gradient: Vec::new(),
        next_best_action: None,
        goodhart_last: None,
        ts: 1_785_700_000,
        availability: ReportAvailability::Available,
    }
}

fn unavailable_report() -> IntelligenceReport {
    IntelligenceReport {
        j: f64::NAN,
        availability: ReportAvailability::Unavailable {
            code: "CALYX_ANNEAL_J_INVALID_METRIC".to_string(),
            message: "fixture invalid".to_string(),
            remediation: "fix fixture".to_string(),
        },
        ..report(0.0)
    }
}

fn terms(j: f64) -> JTerms {
    JTerms {
        w1_info: j.max(0.0),
        w2_n_eff: 0.0,
        w3_sufficiency: 0.0,
        w4_kernel_recall: 0.0,
        w5_oracle_accuracy: 0.0,
        w6_mistake_rate: 0.0,
        w7_compression: 0.0,
        w8_coverage: 0.0,
        p_redundant: 0.0,
        p_ungrounded: 0.0,
        p_goodhart: 0.0,
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-growth-curve-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: std::path::PathBuf) {
    assert!(dir.starts_with(std::env::temp_dir()));
    let _ = fs::remove_dir_all(dir);
}
