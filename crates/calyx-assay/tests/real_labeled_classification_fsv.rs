use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    AssayCacheKey, AssayGate, AssayStore, AssaySubject, TrustTag, admit_lens, bits_report,
    entropy_bits, panel_sufficiency, per_sensor_attribution,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, CxId, SlotId, VaultId};
use calyx_loom::{AbundanceReport, CeilingEstimate, LoomStore, NeffEstimate};
use serde_json::json;

#[derive(Clone, Debug)]
struct IrisRow {
    features: [f32; 4],
    label: String,
}

const UCI_IRIS_SOURCE: &str =
    "https://archive.ics.uci.edu/ml/machine-learning-databases/iris/iris.data";
const UCI_IRIS_BLAKE3: &str = "8578940c6c00041901b00392034412b20e2f574eba595bcc6b979ca4148178e6";

#[test]
#[ignore = "manual FSV requires CALYX_STAGE5_CLASSIFICATION_CSV real dataset path"]
fn real_iris_classification_assay_loom_fsv() {
    let dataset = dataset_path();
    let raw = fs::read(&dataset).expect("read real classification dataset");
    let dataset_hash = blake3::hash(&raw).to_hex().to_string();
    assert_eq!(dataset_hash, UCI_IRIS_BLAKE3);
    let rows = parse_iris(&raw);
    assert_eq!(rows.len(), 150);

    let (sepal, petal, combined, labels, class_counts) = build_samples(&rows);
    let gate = AssayGate::default();
    let sepal_signal = gate.lens_signal(&sepal, &labels).unwrap();
    let petal_signal = gate.lens_signal(&petal, &labels).unwrap();
    let combined_signal = gate.lens_signal(&combined, &labels).unwrap();
    let pair_gain = gate.pair_gain(&sepal, &petal, &labels).unwrap();
    assert!(petal_signal.estimate.bits > 0.8);
    assert!(combined_signal.estimate.bits > 0.8);
    assert!(petal_signal.estimate.bits >= sepal_signal.estimate.bits);
    admit_lens(petal_signal.estimate.bits, 0.2).unwrap();
    admit_lens(combined_signal.estimate.bits, 0.2).unwrap();

    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let assay_dir = clean_dir(&root.join("iris-assay-cf"));
    let xterm_dir = clean_dir(&root.join("iris-xterm-cf"));

    let mut assay_router = CfRouter::open(&assay_dir, 1_048_576).unwrap();
    let mut assay_store = AssayStore::default();
    let key = AssayCacheKey::scoped(
        30,
        "uci-iris-setosa",
        vault_id(),
        AnchorKind::Label("iris-setosa".to_string()),
    );
    assay_store.put(
        key.clone(),
        AssaySubject::Lens { slot: slot(1) },
        sepal_signal.estimate.clone(),
        "UCI Iris sepal features vs setosa label",
        340,
    );
    assay_store.put(
        key.clone(),
        AssaySubject::Lens { slot: slot(2) },
        petal_signal.estimate.clone(),
        "UCI Iris petal features vs setosa label",
        341,
    );
    assay_store.put(
        key.clone(),
        AssaySubject::Pair {
            a: slot(1),
            b: slot(2),
        },
        gate.pair_gain_estimate(&pair_gain),
        "UCI Iris sepal+petal pair gain vs setosa label",
        342,
    );
    assay_store.put(
        key.clone(),
        AssaySubject::Panel,
        combined_signal.estimate.clone(),
        "UCI Iris full feature panel vs setosa label",
        343,
    );
    let assay_persisted = assay_store.persist_to_aster(&mut assay_router).unwrap();
    let loaded_assay = AssayStore::load_from_aster(&assay_router).unwrap();
    assert_eq!(assay_persisted, 4);
    assert_eq!(assay_router.iter_cf(ColumnFamily::Assay).unwrap().len(), 4);
    assert_eq!(loaded_assay.rows().len(), 4);

    let mut loom = LoomStore::new(256);
    for (index, row) in rows.iter().enumerate() {
        loom.weave(cx(index), &slot_map(row)).unwrap();
    }
    let mut xterm_router = CfRouter::open(&xterm_dir, 1_048_576).unwrap();
    let xterm_persisted = loom.persist_xterms_to_aster(&mut xterm_router).unwrap();
    let loaded_xterm = LoomStore::load_xterms_from_aster(&xterm_router, 256).unwrap();
    assert_eq!(xterm_persisted, rows.len());
    assert_eq!(
        xterm_router.iter_cf(ColumnFamily::XTerm).unwrap().len(),
        rows.len()
    );
    assert_eq!(loaded_xterm.xterm_count(), rows.len());
    let attributions = per_sensor_attribution(
        &[
            (slot(1), sepal_signal.estimate.bits),
            (slot(2), petal_signal.estimate.bits),
        ],
        0.05,
    );
    let anchor_entropy_bits = entropy_bits(&labels);
    let sufficiency = panel_sufficiency(
        combined_signal.estimate.bits,
        anchor_entropy_bits,
        &attributions,
        TrustTag::Trusted,
    );
    let abundance = AbundanceReport::new(
        2,
        rows.len(),
        loaded_xterm.xterm_count(),
        NeffEstimate::Computed {
            value: 2.0,
            ci_low: 2.0,
            ci_high: 2.0,
        },
        CeilingEstimate::Computed {
            bits: anchor_entropy_bits,
        },
        rows.len() * 2,
        loaded_xterm.xterm_count(),
    );

    let assay_readback = json!({
        "dataset_path": dataset.display().to_string(),
        "dataset_source": UCI_IRIS_SOURCE,
        "dataset_hash_blake3": dataset_hash,
        "row_count": rows.len(),
        "class_counts": class_counts,
        "assay_cf_root": assay_dir.join("cf/assay").display().to_string(),
        "persisted_rows": assay_persisted,
        "raw_cf_rows": assay_router.iter_cf(ColumnFamily::Assay).unwrap().len(),
        "loaded_rows": loaded_assay.rows(),
        "expected_signal": {
            "petal_bits_gt_0_8": petal_signal.estimate.bits > 0.8,
            "panel_bits_gt_0_8": combined_signal.estimate.bits > 0.8,
            "petal_bits_gte_sepal_bits": petal_signal.estimate.bits >= sepal_signal.estimate.bits,
        },
        "bits_report": bits_report(attributions, TrustTag::Trusted),
        "sufficiency": sufficiency,
    });
    write_json(
        root.join("real-classification-assay-cf-readback.json"),
        &assay_readback,
    );

    let xterm_readback = json!({
        "dataset_hash_blake3": assay_readback["dataset_hash_blake3"],
        "row_count": rows.len(),
        "xterm_cf_root": xterm_dir.join("cf/xterm").display().to_string(),
        "persisted_rows": xterm_persisted,
        "raw_cf_rows": xterm_router.iter_cf(ColumnFamily::XTerm).unwrap().len(),
        "loaded_xterm_rows": loaded_xterm.xterm_count(),
        "agreement_graph": loaded_xterm.agreement_graph(),
        "sample_rows": loaded_xterm.xterm_rows().into_iter().take(5).collect::<Vec<_>>(),
        "abundance": abundance,
    });
    write_json(
        root.join("real-classification-xterm-cf-readback.json"),
        &xterm_readback,
    );

    let summary = json!({
        "dataset": "UCI Iris",
        "dataset_source": UCI_IRIS_SOURCE,
        "dataset_hash_blake3": assay_readback["dataset_hash_blake3"],
        "row_count": rows.len(),
        "anchor_entropy_bits": anchor_entropy_bits,
        "assay_rows": assay_persisted,
        "xterm_rows": xterm_persisted,
        "petal_bits": petal_signal.estimate.bits,
        "sepal_bits": sepal_signal.estimate.bits,
        "panel_bits": combined_signal.estimate.bits,
        "pair_gain_bits": pair_gain.gain_bits,
    });
    write_json(
        root.join("real-classification-summary-readback.json"),
        &summary,
    );
    println!("REAL_CLASSIFICATION_SUMMARY={}", root.display());
}

fn parse_iris(raw: &[u8]) -> Vec<IrisRow> {
    String::from_utf8_lossy(raw)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts: Vec<_> = line.split(',').collect();
            assert_eq!(parts.len(), 5);
            IrisRow {
                features: [
                    parts[0].parse().unwrap(),
                    parts[1].parse().unwrap(),
                    parts[2].parse().unwrap(),
                    parts[3].parse().unwrap(),
                ],
                label: parts[4].trim().to_string(),
            }
        })
        .collect()
}

type Samples = (
    Vec<Vec<f32>>,
    Vec<Vec<f32>>,
    Vec<Vec<f32>>,
    Vec<bool>,
    BTreeMap<String, usize>,
);

fn build_samples(rows: &[IrisRow]) -> Samples {
    let mut sepal = Vec::with_capacity(rows.len());
    let mut petal = Vec::with_capacity(rows.len());
    let mut combined = Vec::with_capacity(rows.len());
    let mut labels = Vec::with_capacity(rows.len());
    let mut class_counts = BTreeMap::new();
    for row in rows {
        sepal.push(vec![row.features[0], row.features[1]]);
        petal.push(vec![row.features[2], row.features[3]]);
        combined.push(row.features.to_vec());
        labels.push(row.label == "Iris-setosa");
        *class_counts.entry(row.label.clone()).or_default() += 1;
    }
    (sepal, petal, combined, labels, class_counts)
}

fn slot_map(row: &IrisRow) -> BTreeMap<SlotId, Vec<f32>> {
    BTreeMap::from([
        (slot(1), vec![row.features[0], row.features[1]]),
        (slot(2), vec![row.features[2], row.features[3]]),
    ])
}

fn write_json(path: PathBuf, value: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    fs::write(&path, &bytes).unwrap();
    let readback = fs::read(&path).unwrap();
    assert_eq!(readback, bytes);
}

fn clean_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn dataset_path() -> PathBuf {
    std::env::var("CALYX_STAGE5_CLASSIFICATION_CSV")
        .map(PathBuf::from)
        .expect("CALYX_STAGE5_CLASSIFICATION_CSV must point at iris.data")
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue340-real-classification-fsv"))
}

fn cx(index: usize) -> CxId {
    CxId::from_bytes([index as u8; 16])
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
