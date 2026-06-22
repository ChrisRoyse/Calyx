use std::fs;
use std::path::PathBuf;

use calyx_assay::{
    AssayCacheKey, AssayGate, AssayStore, AssaySubject, DEFAULT_BOOTSTRAP_RESAMPLES,
    DEFAULT_BOOTSTRAP_SEED, MiEstimate, bootstrap_mean_ci, ksg_mi_continuous, logistic_probe_mi,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, SlotId};
use serde_json::json;

#[allow(dead_code)]
mod stage5_helpers;
use stage5_helpers::{
    assay_vault, binary_samples, complementary_pair_samples, correlated_samples, gaussian_mi_bits,
    slot,
};

#[test]
fn public_estimator_paths_attach_seeded_bootstrap_ci() {
    let (x, y) = correlated_samples(120);
    let ksg = ksg_mi_continuous(&x, &y, 3).unwrap();
    assert_valid_ci(&ksg);

    let (samples, labels) = binary_samples(true);
    let logistic = logistic_probe_mi(&samples, &labels).unwrap();
    assert_valid_ci(&logistic.estimate);

    let gate = AssayGate::default();
    let signal = gate.lens_signal(&samples, &labels).unwrap();
    assert_valid_ci(&signal.estimate);

    let (left, right, pair_labels) = complementary_pair_samples();
    let gain = gate.pair_gain(&left, &right, &pair_labels).unwrap();
    let gain_estimate = gate.pair_gain_estimate(&gain);
    assert_valid_ci(&gain_estimate);
    assert_eq!(gain_estimate.ci_low, gain.ci_low);
    assert_eq!(gain_estimate.ci_high, gain.ci_high);
}

#[test]
#[ignore = "manual FSV writes Assay CF source-of-truth readbacks"]
fn bootstrap_ci_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let cf_root = root.join("bootstrap-ci-assay-cf");
    let _ = fs::remove_dir_all(&cf_root);
    let mut router = CfRouter::open(&cf_root, 1_048_576).unwrap();

    let (x, y) = correlated_samples(120);
    let ksg = ksg_mi_continuous(&x, &y, 3).unwrap();
    let known_bits = gaussian_mi_bits(&x, &y);
    let (samples, labels) = binary_samples(true);
    let logistic = logistic_probe_mi(&samples, &labels).unwrap();
    let gate = AssayGate::default();
    let signal = gate.lens_signal(&samples, &labels).unwrap();
    let (left, right, pair_labels) = complementary_pair_samples();
    let gain = gate.pair_gain(&left, &right, &pair_labels).unwrap();
    let gain_estimate = gate.pair_gain_estimate(&gain);

    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(
        28,
        "issue318-bootstrap-ci",
        assay_vault(),
        AnchorKind::Label("issue318-planted-passfail".to_string()),
    );
    store.put(
        key.clone(),
        AssaySubject::Lens { slot: slot(1) },
        signal.estimate.clone(),
        "issue318 public AssayGate::lens_signal seeded bootstrap CI",
        3181,
    );
    store.put(
        key.clone(),
        AssaySubject::Pair {
            a: slot(1),
            b: slot(2),
        },
        gain_estimate.clone(),
        "issue318 public AssayGate::pair_gain seeded bootstrap CI",
        3182,
    );
    store.put(
        key.clone(),
        AssaySubject::Lens { slot: slot(3) },
        ksg.clone(),
        "issue318 public ksg_mi_continuous seeded bootstrap CI",
        3183,
    );
    let persisted_rows = store.persist_to_aster(&mut router).unwrap();
    let raw_cf_rows = raw_assay_rows(&router);
    let loaded = AssayStore::load_from_aster(&router).unwrap();
    let loaded_lens = loaded
        .get(&key, &AssaySubject::Lens { slot: slot(1) })
        .unwrap();
    let loaded_pair = loaded
        .get(
            &key,
            &AssaySubject::Pair {
                a: slot(1),
                b: slot(2),
            },
        )
        .unwrap();
    let loaded_ksg = loaded
        .get(&key, &AssaySubject::Lens { slot: slot(3) })
        .unwrap();

    let readback = json!({
        "source_of_truth": "Aster Assay CF value bytes loaded after public estimator/gate paths",
        "cf_root": cf_root.join("cf/assay").display().to_string(),
        "bootstrap": {
            "resamples": DEFAULT_BOOTSTRAP_RESAMPLES,
            "seed": DEFAULT_BOOTSTRAP_SEED,
        },
        "happy_path": {
            "persisted_rows": persisted_rows,
            "loaded_rows": loaded.len(),
            "raw_cf_rows": raw_cf_rows,
            "ksg_estimate": ksg,
            "ksg_known_bits": known_bits,
            "ksg_point_inside_ci": ci_contains(&loaded_ksg.estimate, loaded_ksg.estimate.bits),
            "ksg_known_inside_ci": ci_contains(&loaded_ksg.estimate, known_bits),
            "logistic_estimate": logistic.estimate,
            "gate_lens_loaded": loaded_lens,
            "pair_gain": gain,
            "pair_gain_loaded": loaded_pair,
            "all_loaded_ci_fields_present": loaded.rows().iter().all(|row| valid_ci(&row.estimate)),
        },
        "edge_cases": edge_cases(&root),
    });
    let path = root.join("bootstrap-ci-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("BOOTSTRAP_CI_READBACK={}", path.display());
}

fn edge_cases(root: &std::path::Path) -> serde_json::Value {
    let (x, y) = correlated_samples(120);
    let short_error = ksg_mi_continuous(&x[..30], &y[..30], 3).unwrap_err().code;
    let mut nonfinite_samples = binary_samples(true).0;
    let labels = binary_samples(true).1;
    nonfinite_samples[0][0] = f32::NAN;
    let nonfinite_error = logistic_probe_mi(&nonfinite_samples, &labels)
        .unwrap_err()
        .code;
    let zero_resamples = bootstrap_mean_ci(&[0.8, 1.0, 1.2], 0, DEFAULT_BOOTSTRAP_SEED);
    let unscoped = unscoped_store_edge(root);
    json!([
        {
            "case": "ksg_short_sample_quorum",
            "before": {"x_samples": 30, "y_samples": 30, "k": 3},
            "after": {"error": short_error},
        },
        {
            "case": "logistic_non_finite_input",
            "before": {"samples": nonfinite_samples.len(), "first_value": "NaN"},
            "after": {"error": nonfinite_error},
        },
        {
            "case": "zero_bootstrap_resamples",
            "before": {"values": [0.8, 1.0, 1.2], "resamples": 0},
            "after": {"ci_returned": zero_resamples.is_some()},
        },
        unscoped,
    ])
}

fn unscoped_store_edge(root: &std::path::Path) -> serde_json::Value {
    let dir = root.join("unscoped-edge-cf");
    let _ = fs::remove_dir_all(&dir);
    let mut router = CfRouter::open(&dir, 1_048_576).unwrap();
    let mut store = AssayStore::default();
    #[allow(deprecated)]
    let key = AssayCacheKey::new(28, "unscoped-edge");
    store.put(
        key,
        AssaySubject::Lens {
            slot: SlotId::new(9),
        },
        MiEstimate::point(
            0.25,
            50,
            calyx_assay::EstimatorKind::LogisticProbe,
            calyx_assay::TrustTag::Provisional,
        ),
        "unscoped row should not persist",
        9,
    );
    let before = router.iter_cf(ColumnFamily::Assay).unwrap().len();
    let error = store.persist_to_aster(&mut router).unwrap_err().code;
    let after = router.iter_cf(ColumnFamily::Assay).unwrap().len();
    json!({
        "case": "unscoped_store_rejected",
        "before": {"store_rows": store.len(), "raw_cf_rows": before},
        "after": {"error": error, "raw_cf_rows": after},
    })
}

fn raw_assay_rows(router: &CfRouter) -> Vec<serde_json::Value> {
    router
        .iter_cf(ColumnFamily::Assay)
        .unwrap()
        .into_iter()
        .map(|entry| {
            let value_text = String::from_utf8_lossy(&entry.value);
            json!({
                "key_hex": hex(&entry.key),
                "value_hex": hex(&entry.value),
                "value_len": entry.value.len(),
                "ci_low_bytes_present": value_text.contains("\"ci_low\""),
                "ci_high_bytes_present": value_text.contains("\"ci_high\""),
                "value_json": serde_json::from_slice::<serde_json::Value>(&entry.value).unwrap(),
            })
        })
        .collect()
}

fn assert_valid_ci(estimate: &MiEstimate) {
    assert!(valid_ci(estimate), "{estimate:?}");
}

fn valid_ci(estimate: &MiEstimate) -> bool {
    estimate.ci_low <= estimate.bits && estimate.bits <= estimate.ci_high
}

fn ci_contains(estimate: &MiEstimate, value: f32) -> bool {
    estimate.ci_low <= value && value <= estimate.ci_high
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-bootstrap-ci-fsv"))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
