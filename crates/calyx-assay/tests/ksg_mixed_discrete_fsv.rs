use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, entropy_bits, ksg_mi_continuous,
    ksg_mi_continuous_discrete, ksg_mi_continuous_discrete_with_anchor,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{Anchor, AnchorKind, AnchorValue, VaultId};
use serde_json::json;

const UCI_IRIS_SOURCE: &str =
    "https://archive.ics.uci.edu/ml/machine-learning-databases/iris/iris.data";
const UCI_IRIS_BLAKE3: &str = "8578940c6c00041901b00392034412b20e2f574eba595bcc6b979ca4148178e6";

#[test]
fn ksg_mixed_discrete_recovers_planted_signal_without_one_hot_scale_bias() {
    let (small_x, labels) = planted_multiclass_samples(1.0);
    let (large_x, _) = planted_multiclass_samples(1_000.0);
    let entropy = (3.0_f32).log2();

    let ross_small = ksg_mi_continuous_discrete(&small_x, &labels, 3).unwrap();
    let ross_large = ksg_mi_continuous_discrete(&large_x, &labels, 3).unwrap();
    let one_hot_small = one_hot_continuous_estimate(&small_x, &labels, 3);
    let one_hot_large = one_hot_continuous_estimate(&large_x, &labels, 3);

    assert!(ross_small.bits > 1.1, "{ross_small:?}");
    assert!(ross_small.bits <= entropy + 0.25, "{ross_small:?}");
    assert!(
        (ross_small.bits - ross_large.bits).abs() < 0.02,
        "{ross_small:?} vs {ross_large:?}"
    );
    assert!(
        one_hot_small.bits - one_hot_large.bits > 1.0,
        "{one_hot_small:?} vs {one_hot_large:?}"
    );

    maybe_write_fsv(json!({
        "source_of_truth": "public calyx_assay continuous-discrete estimator outputs read back from this JSON artifact",
        "formula": "Ross-style mixed estimator: same-label kth continuous radius, all-sample continuous neighbor count, digamma(n)+digamma(k)-digamma(class_size)-digamma(full_count)",
        "samples": {
            "classes": 3,
            "per_class": 80,
            "n": labels.len(),
            "k": 3,
            "x_scales": [1.0, 1000.0],
        },
        "expected_entropy_bits": entropy,
        "ross_mixed": {
            "small_scale": ross_small,
            "large_scale": ross_large,
            "scale_delta_abs": (ross_small.bits - ross_large.bits).abs(),
            "recovers_planted_signal": ross_small.bits > 1.1 && ross_small.bits <= entropy + 0.25,
        },
        "old_one_hot_continuous_regression": {
            "small_scale": one_hot_small,
            "large_scale": one_hot_large,
            "scale_drop_bits": one_hot_small.bits - one_hot_large.bits,
            "deviates_on_arbitrary_scale": one_hot_small.bits - one_hot_large.bits > 1.0,
        },
        "edge_case": underpowered_class_error_readback(),
    }));
}

#[test]
fn ksg_mixed_discrete_rejects_underpowered_discrete_classes() {
    let edge = underpowered_class_error_readback();
    assert_eq!(edge["after"]["error"], "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(edge["after"]["mentions_label"], true);
    assert_eq!(edge["after"]["mentions_class_size"], true);
}

#[test]
#[ignore = "manual FSV requires CALYX_STAGE5_CLASSIFICATION_CSV real dataset path"]
fn ksg_mixed_discrete_real_labeled_dataset_delta_fsv() {
    let dataset = std::env::var("CALYX_STAGE5_CLASSIFICATION_CSV")
        .map(PathBuf::from)
        .expect("CALYX_STAGE5_CLASSIFICATION_CSV must point at iris.data");
    let raw = fs::read(&dataset).expect("read real classification dataset");
    let dataset_hash = blake3::hash(&raw).to_hex().to_string();
    assert_eq!(dataset_hash, UCI_IRIS_BLAKE3);
    let (features, labels, class_counts) = parse_iris(&raw);
    let anchor_entropy_bits = entropy_bits(&labels);
    let anchor = uci_iris_species_anchor();
    let ross = ksg_mi_continuous_discrete_with_anchor(&features, &labels, 3, &anchor).unwrap();
    let one_hot = one_hot_continuous_estimate(&features, &labels, 3);

    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx-issue1207-ksg-mixed-real-fsv")
    });
    fs::create_dir_all(&root).unwrap();
    let cf_root = clean_dir(&root.join("issue1207-iris-assay-cf"));
    let mut router = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(
        55,
        "issue1207-uci-iris-species",
        vault_id(),
        AnchorKind::Label("uci-iris-species".to_string()),
    );
    store.put(
        key.clone(),
        AssaySubject::Panel,
        ross.clone(),
        "UCI Iris all numeric features vs species label through Ross mixed estimator",
        1207,
    );
    let persisted_rows = store.persist_to_aster(&mut router).unwrap();
    let raw_cf_rows = router.iter_cf(ColumnFamily::Assay).unwrap().len();
    let loaded = AssayStore::load_from_aster(&router).unwrap();
    let loaded_panel = loaded.get(&key, &AssaySubject::Panel).unwrap();

    let readback = json!({
        "source_of_truth": "UCI Iris dataset bytes plus Aster Assay CF row loaded after persisting Ross mixed estimate",
        "dataset": {
            "path": dataset.display().to_string(),
            "source": UCI_IRIS_SOURCE,
            "blake3": dataset_hash,
            "rows": labels.len(),
            "class_counts": class_counts,
        },
        "anchor": {
            "kind": "label:uci-iris-species",
            "source": anchor.source.clone(),
            "confidence": anchor.confidence,
            "entropy_bits": anchor_entropy_bits,
        },
        "estimators": {
            "ross_mixed": ross,
            "old_one_hot_continuous": one_hot,
            "abs_delta_bits": (ross.bits - one_hot.bits).abs(),
            "sufficiency_decision_source": "ross_mixed",
            "ross_ci_low_clears_entropy": ross.ci_low >= anchor_entropy_bits,
        },
        "assay_cf_readback": {
            "cf_root": cf_root.join("cf/assay").display().to_string(),
            "persisted_rows": persisted_rows,
            "raw_cf_rows": raw_cf_rows,
            "loaded_rows": loaded.len(),
            "loaded_panel": loaded_panel,
        },
    });
    let path = root.join("issue1207-real-labeled-ross-delta-readback.json");
    write_json_readback(&path, &readback);
    println!("ISSUE1207_REAL_LABELED_READBACK={}", path.display());
}

fn planted_multiclass_samples(scale: f32) -> (Vec<Vec<f32>>, Vec<usize>) {
    let classes = 3;
    let per_class = 80;
    let mut x = Vec::with_capacity(classes * per_class);
    let mut labels = Vec::with_capacity(classes * per_class);
    for class in 0..classes {
        let center = class as f32 * 4.0;
        for index in 0..per_class {
            let tie_break = ((index * index + 7 * index) % 23) as f32 * 0.0001;
            let jitter = index as f32 * 0.01 + tie_break;
            x.push(vec![(center + jitter) * scale]);
            labels.push(class);
        }
    }
    (x, labels)
}

fn one_hot_continuous_estimate(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
) -> calyx_assay::MiEstimate {
    let mut classes = BTreeMap::<usize, usize>::new();
    for label in labels {
        let next = classes.len();
        classes.entry(*label).or_insert(next);
    }
    let y: Vec<Vec<f32>> = labels
        .iter()
        .map(|label| {
            let mut row = vec![0.0; classes.len()];
            row[classes[label]] = 1.0;
            row
        })
        .collect();
    ksg_mi_continuous(x, &y, k).unwrap()
}

fn underpowered_class_error_readback() -> serde_json::Value {
    let x: Vec<Vec<f32>> = (0..50).map(|index| vec![index as f32]).collect();
    let mut labels = vec![0; 47];
    labels.extend([7, 7, 7]);
    let error = ksg_mi_continuous_discrete(&x, &labels, 3).unwrap_err();
    json!({
        "case": "class_size_equal_to_k",
        "before": {
            "samples": x.len(),
            "majority_label_count": 47,
            "minority_label": 7,
            "minority_label_count": 3,
            "k": 3,
        },
        "after": {
            "error": error.code,
            "message": error.message,
            "mentions_label": error.message.contains("label=7"),
            "mentions_class_size": error.message.contains("class_size=3"),
        },
    })
}

fn parse_iris(raw: &[u8]) -> (Vec<Vec<f32>>, Vec<usize>, BTreeMap<String, usize>) {
    let mut class_ids = BTreeMap::<String, usize>::new();
    let mut class_counts = BTreeMap::<String, usize>::new();
    let mut features = Vec::new();
    let mut labels = Vec::new();
    for line in String::from_utf8_lossy(raw).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<_> = line.split(',').collect();
        assert_eq!(parts.len(), 5);
        let label_name = parts[4].trim().to_string();
        let next = class_ids.len();
        let label_id = *class_ids.entry(label_name.clone()).or_insert(next);
        *class_counts.entry(label_name).or_default() += 1;
        features.push(vec![
            parts[0].parse().unwrap(),
            parts[1].parse().unwrap(),
            parts[2].parse().unwrap(),
            parts[3].parse().unwrap(),
        ]);
        labels.push(label_id);
    }
    assert_eq!(labels.len(), 150);
    (features, labels, class_counts)
}

fn uci_iris_species_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Label("uci-iris-species".to_string()),
        value: AnchorValue::Enum("species".to_string()),
        source: UCI_IRIS_SOURCE.to_string(),
        observed_at: 1_785_400_000,
        confidence: 1.0,
    }
}

fn maybe_write_fsv(readback: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let dir = root.join("issue1207-ksg-mixed-discrete");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ross-mixed-discrete-readback.json");
    let bytes = serde_json::to_vec_pretty(&readback).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stored = fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    println!("ISSUE1207_KSG_MIXED_DISCRETE_READBACK={}", path.display());
}

fn clean_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn write_json_readback(path: &Path, value: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    fs::write(path, &bytes).unwrap();
    let stored = fs::read(path).unwrap();
    assert_eq!(stored, bytes);
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
