use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterGrowthCf, GrowthCurve, IntelligenceReport, JTerms, JWeights, ReportAvailability,
    decode_growth_row,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use serde_json::json;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::{vault_id, write_json};

const VAULT_SALT: &[u8] = b"calyx-anneal-intelligence-report";

#[test]
#[ignore = "manual FSV writes source-of-truth evidence under CALYX_FSV_OUT"]
fn ph48_growth_curve_manual_fsv() {
    let out = PathBuf::from(std::env::var_os("CALYX_FSV_OUT").expect("CALYX_FSV_OUT"));
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("create fsv out");
    let vault_dir = out.join("growth-vault");
    let vault = open_vault(&vault_dir);

    let before_rows = scan_growth_rows(&vault);
    let mut curve = GrowthCurve::load_from_cf(
        AsterGrowthCf::new(&vault),
        Arc::new(FixedClock::new(1_785_700_427)),
        10,
    )
    .expect("growth curve");
    for (index, value) in [1.0, 1.5, 2.0, 2.6, 3.2].iter().enumerate() {
        curve
            .record_sample(
                &report(*value),
                1_000,
                vec![format!("real-corpus-batch-{index}")],
            )
            .expect("record sample");
    }
    let after_rows = scan_growth_rows(&vault);
    let decoded = decode_rows(&after_rows);
    let summary = curve.curve_summary_with_window(5);
    let plot = curve.plot_ascii(20, 6);
    assert!(summary.is_rising);
    assert_eq!(summary.j_first, Some(1.0));
    assert_eq!(summary.j_last, Some(3.2));
    assert_eq!(before_rows.len(), 0);
    assert_eq!(after_rows.len(), 5);

    let evidence = json!({
        "source_of_truth": format!("{}/cf/anneal_growth", vault_dir.display()),
        "trigger": "record five PH48 IntelligenceReport snapshots into GrowthCurve",
        "expected": {
            "before_rows": 0,
            "after_rows": 5,
            "j_first": 1.0,
            "j_last": 3.2,
            "is_rising": true
        },
        "actual": {
            "before_rows": before_rows.len(),
            "after_rows": after_rows.len(),
            "summary": summary,
            "plot_ascii": plot,
            "decoded_rows": decoded,
            "sst_files": sst_files(&vault_dir.join("cf").join("anneal_growth")),
            "row_hex": after_rows.iter().map(|(key, value)| {
                json!({"key": hex(key), "value_prefix": hex_prefix(value, 96), "value_len": value.len()})
            }).collect::<Vec<_>>(),
        }
    });
    write_json(&out.join("ph48_growth_curve.json"), &evidence);

    let edge_evidence = run_edges(&out);
    write_json(&out.join("ph48_growth_curve_edges.json"), &edge_evidence);
}

fn run_edges(out: &Path) -> serde_json::Value {
    let empty_vault_dir = out.join("empty-vault");
    let empty_vault = open_vault(&empty_vault_dir);
    let empty_curve = GrowthCurve::load_from_cf(
        AsterGrowthCf::new(&empty_vault),
        Arc::new(FixedClock::new(1_785_700_428)),
        10,
    )
    .expect("empty curve");
    let empty_rows = scan_growth_rows(&empty_vault);

    let capped_vault_dir = out.join("capped-vault");
    let capped_vault = open_vault(&capped_vault_dir);
    let mut capped = GrowthCurve::load_from_cf(
        AsterGrowthCf::new(&capped_vault),
        Arc::new(FixedClock::new(1_785_700_429)),
        1,
    )
    .expect("capped curve");
    let capped_before = scan_growth_rows(&capped_vault);
    capped
        .record_sample(&report(1.0), 1, vec!["first".to_string()])
        .expect("record first");
    capped
        .record_sample(&report(3.0), 1, vec!["second".to_string()])
        .expect("record second");
    let capped_after = scan_growth_rows(&capped_vault);

    let invalid_before = capped_after.len();
    let invalid_error = capped
        .record_sample(&unavailable_report(), 0, Vec::new())
        .expect_err("invalid report rejected");
    let invalid_after = scan_growth_rows(&capped_vault);

    json!({
        "empty_cf": {
            "source_of_truth": format!("{}/cf/anneal_growth", empty_vault_dir.display()),
            "before_rows": empty_rows.len(),
            "summary": empty_curve.curve_summary(),
            "is_rising_10": empty_curve.is_rising(10)
        },
        "max_samples_one": {
            "source_of_truth": format!("{}/cf/anneal_growth", capped_vault_dir.display()),
            "before_rows": capped_before.len(),
            "after_rows": capped_after.len(),
            "retained_in_memory": capped.len(),
            "retained_j": capped.samples().next().map(|sample| sample.j),
            "decoded_rows": decode_rows(&capped_after)
        },
        "invalid_report_fail_closed": {
            "before_rows": invalid_before,
            "after_rows": invalid_after.len(),
            "error_code": invalid_error.code,
            "message": invalid_error.message
        }
    })
}

fn open_vault(path: &Path) -> AsterVault {
    AsterVault::new_durable(
        path,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn scan_growth_rows(vault: &AsterVault) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealGrowth)
        .expect("scan anneal_growth")
}

fn decode_rows(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|(key, value)| {
            let (seq, sample) = decode_growth_row(value).expect("decode growth row");
            json!({"key": hex(key), "seq": seq, "sample": sample})
        })
        .collect()
}

fn report(j: f64) -> IntelligenceReport {
    IntelligenceReport {
        j,
        terms: terms(j),
        weights: JWeights::default(),
        dpi_ceiling: j + 1.0,
        dpi_headroom: 1.0,
        provisional_excluded: 0,
        gradient: Vec::new(),
        next_best_action: None,
        goodhart_last: None,
        ts: 1_785_700_427,
        availability: ReportAvailability::Available,
    }
}

fn unavailable_report() -> IntelligenceReport {
    IntelligenceReport {
        j: f64::NAN,
        availability: ReportAvailability::Unavailable {
            code: "CALYX_ANNEAL_J_INVALID_METRIC".to_string(),
            message: "synthetic unavailable report".to_string(),
            remediation: "fix synthetic fixture".to_string(),
        },
        ..report(0.0)
    }
}

fn terms(j: f64) -> JTerms {
    JTerms {
        w1_info: j,
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

fn sst_files(dir: &Path) -> Vec<String> {
    let mut files = fs::read_dir(dir)
        .expect("read sst dir")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension().and_then(|value| value.to_str()) == Some("sst"))
                .then(|| path.display().to_string())
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_prefix(bytes: &[u8], limit: usize) -> String {
    hex(&bytes[..bytes.len().min(limit)])
}
