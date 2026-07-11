//! Manual full-state verification for issue #1312.

use std::{fs, path::PathBuf};

use calyx_assay::{
    CALYX_TC_INSUFFICIENT_SAMPLES, IISign, TotalCorrelationConfig,
    interaction_information_with_config, total_correlation_with_config,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::FixedClock;
use serde_json::json;

mod ph52_signal_support;

use ph52_signal_support::noise;

const ASSAY_KEY: &[u8] = b"issue1312/tc-ii/no-replacement/v1";

#[test]
#[ignore = "manual FSV persists and reopens deterministic TC/II Assay CF bytes"]
fn issue1312_tc_subsample_manual_fsv() {
    let root = fsv_root().join("issue1312-tc-subsample");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let cf_root = root.join("aster");
    let report_path = root.join("issue1312-tc-subsample-readback.json");
    let config = TotalCorrelationConfig {
        bootstrap_resamples: 64,
        bootstrap_seed: 0x1312_2026,
        ..TotalCorrelationConfig::default()
    };
    let clock = FixedClock::new(1_786_100_312);

    let panel = redundant_panel(240);
    let tc = total_correlation_with_config(&panel, &clock, &config).unwrap();
    let triple = redundant_triple(260);
    let ii = interaction_information_with_config(&triple.0, &triple.1, &triple.2, &clock, &config)
        .unwrap();
    assert!(tc.tc > 0.0 && tc.ci_95.0 <= tc.tc && tc.tc <= tc.ci_95.1);
    assert_eq!(ii.sign, IISign::Redundant);

    let tc_repeat = total_correlation_with_config(&panel, &clock, &config).unwrap();
    let ii_repeat =
        interaction_information_with_config(&triple.0, &triple.1, &triple.2, &clock, &config)
            .unwrap();
    assert_eq!(tc, tc_repeat);
    assert_eq!(ii, ii_repeat);

    let edges = edge_readbacks(&clock, &config);
    let persisted = json!({
        "schema_version": "calyx.issue1312.tc_subsample.v1",
        "source_of_truth": "deterministic public TC/II results persisted in Aster Assay CF",
        "config": {
            "k": config.k,
            "subsample_fraction": "4/5 without replacement",
            "resamples": config.bootstrap_resamples,
            "seed": config.bootstrap_seed,
        },
        "happy_path": {
            "tc": tc,
            "ii": ii,
            "repeat_byte_stable": true,
        },
        "edges": edges,
    });
    let persisted_bytes = serde_json::to_vec(&persisted).unwrap();

    let mut router = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let before = router.get(ColumnFamily::Assay, ASSAY_KEY).unwrap();
    assert!(before.is_none());
    router
        .put(ColumnFamily::Assay, ASSAY_KEY, &persisted_bytes)
        .unwrap();
    router.flush_cf(ColumnFamily::Assay).unwrap();
    drop(router);

    let reopened = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let reopened_bytes = reopened
        .get(ColumnFamily::Assay, ASSAY_KEY)
        .unwrap()
        .expect("persisted issue1312 Assay row");
    let raw_rows = reopened.iter_cf(ColumnFamily::Assay).unwrap();
    assert_eq!(reopened_bytes, persisted_bytes);
    assert_eq!(raw_rows.len(), 1);
    let reopened_json: serde_json::Value = serde_json::from_slice(&reopened_bytes).unwrap();
    assert_eq!(reopened_json, persisted);

    let report = json!({
        "source_of_truth": {
            "column_family": "assay",
            "key_utf8": String::from_utf8_lossy(ASSAY_KEY),
            "cf_root": cf_root,
        },
        "before": {"key_present": false},
        "action": {
            "serialized_bytes": persisted_bytes.len(),
            "flushed_then_closed": true,
        },
        "after": {
            "reopened_raw_rows": raw_rows.len(),
            "reopened_bytes": reopened_bytes.len(),
            "byte_for_byte_match": reopened_bytes == persisted_bytes,
            "decoded_value": reopened_json,
        },
    });
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let report_readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report_readback["after"]["byte_for_byte_match"], true);
    println!("ISSUE1312_TC_SUBSAMPLE_READBACK={}", report_path.display());
}

fn edge_readbacks(clock: &FixedClock, config: &TotalCorrelationConfig) -> serde_json::Value {
    let duplicate_tc = total_correlation_with_config(&duplicate_panel(180), clock, config)
        .unwrap_err()
        .code;
    let duplicate = duplicate_triple(180);
    let duplicate_ii = interaction_information_with_config(
        &duplicate.0,
        &duplicate.1,
        &duplicate.2,
        clock,
        config,
    )
    .unwrap_err()
    .code;
    let zero_k = TotalCorrelationConfig { k: 0, ..*config };
    let invalid_k = total_correlation_with_config(&redundant_panel(180), clock, &zero_k)
        .unwrap_err()
        .code;
    let below_quorum = total_correlation_with_config(&redundant_panel(40), clock, config).unwrap();

    assert_eq!(duplicate_tc, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert_eq!(duplicate_ii, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert_eq!(invalid_k, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(
        below_quorum.error_code.as_deref(),
        Some(CALYX_TC_INSUFFICIENT_SAMPLES)
    );
    json!([
        {"case": "tc_exact_duplicate_rows", "code": duplicate_tc},
        {"case": "ii_exact_duplicate_rows", "code": duplicate_ii},
        {"case": "zero_k", "code": invalid_k},
        {"case": "below_quorum", "code": below_quorum.error_code},
    ])
}

fn redundant_panel(n: usize) -> Vec<Vec<f32>> {
    let base = signal(n, 13);
    vec![
        base.clone(),
        jittered(&base, 17, 0.015),
        jittered(&base, 29, 0.015),
    ]
}

fn redundant_triple(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let base = signal(n, 71);
    (
        base.clone(),
        jittered(&base, 89, 0.012),
        jittered(&base, 107, 0.012),
    )
}

fn duplicate_panel(n: usize) -> Vec<Vec<f32>> {
    let mut panel = redundant_panel(n);
    for slot in &mut panel {
        for index in 1..=3 {
            slot[index] = slot[0];
        }
    }
    panel
}

fn duplicate_triple(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut triple = redundant_triple(n);
    for index in 1..=3 {
        triple.0[index] = triple.0[0];
        triple.1[index] = triple.1[0];
        triple.2[index] = triple.2[0];
    }
    triple
}

fn signal(n: usize, salt: u64) -> Vec<f32> {
    (0..n)
        .map(|t| 4.0 * normalish(t as u64, salt) + (t as f32 / 19.0).sin())
        .collect()
}

fn jittered(base: &[f32], salt: u64, scale: f32) -> Vec<f32> {
    base.iter()
        .enumerate()
        .map(|(index, &value)| value + scale * normalish(index as u64, salt))
        .collect()
}

fn normalish(t: u64, salt: u64) -> f32 {
    (0..6).map(|offset| noise(t, salt + offset)).sum::<f32>() - 3.0
}

fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx-issue1312-tc-subsample-fsv")
    })
}
