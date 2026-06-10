use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CALYX_TEMPORAL_NEGATIVE_WEIGHT, CALYX_TEMPORAL_WEIGHT_SUM, FusionWeights};
use proptest::prelude::*;
use serde_json::{Value, json};

#[test]
fn fusion_weights_validation_matrix_readback() {
    let (root, keep_root) = fsv_root();
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let cases = validation_cases()
        .into_iter()
        .map(run_case)
        .collect::<Vec<_>>();
    let mut readback = json!({
        "before": before,
        "cases": cases,
        "after": null,
    });
    let artifact = root.join("fusion-weights-validation-readback.json");
    write_json(&artifact, &readback);
    readback["after"] = json!({"files_after_artifact_write": list_files(&root)});
    write_json(&artifact, &readback);
    let bytes = fs::read(&artifact).expect("read artifact bytes");
    let reread: Value = serde_json::from_slice(&bytes).expect("parse artifact reread");
    write_blake3_sums(&root);

    assert_eq!(readback, reread);
    assert_case(&reread, "valid_default", None);
    assert_case(&reread, "valid_convex", None);
    assert_case(
        &reread,
        "negative_recency_sum_one",
        Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
    );
    assert_case(
        &reread,
        "negative_sequence_sum_one",
        Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
    );
    assert_case(
        &reread,
        "negative_periodic_sum_one",
        Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
    );
    assert_case(&reread, "nan_recency", Some(CALYX_TEMPORAL_WEIGHT_SUM));
    assert_case(
        &reread,
        "infinite_sequence",
        Some(CALYX_TEMPORAL_WEIGHT_SUM),
    );
    assert_case(&reread, "sum_not_one", Some(CALYX_TEMPORAL_WEIGHT_SUM));

    println!("fusion_weights_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&reread).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn generated_convex_weights_validate(a in 0_u16..=1000, b in 0_u16..=1000) {
        let recency = f32::from(a) / 2000.0;
        let sequence = f32::from(b) / 2000.0;
        let periodic = 1.0 - recency - sequence;

        prop_assert!(periodic >= 0.0);
        prop_assert!(FusionWeights::new(recency, sequence, periodic).is_ok());
    }

    #[test]
    fn generated_negative_components_fail_closed(component in 0_usize..3, magnitude in 1_u16..=1000) {
        let negative = -(f32::from(magnitude) / 1000.0);
        let mut values = [0.0_f32, 0.0, 0.0];
        values[component] = negative;
        values[(component + 1) % values.len()] = 1.0 - negative;

        let error = FusionWeights {
            recency: values[0],
            sequence: values[1],
            periodic: values[2],
        }
        .validate()
        .expect_err("negative component rejected");

        prop_assert_eq!(error.code, CALYX_TEMPORAL_NEGATIVE_WEIGHT);
    }
}

struct ValidationCase {
    name: &'static str,
    weights: FusionWeights,
    expected_code: Option<&'static str>,
}

fn validation_cases() -> [ValidationCase; 8] {
    [
        case("valid_default", FusionWeights::default(), None),
        case("valid_convex", weights(0.4, 0.4, 0.2), None),
        case(
            "negative_recency_sum_one",
            weights(-0.1, 0.6, 0.5),
            Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
        ),
        case(
            "negative_sequence_sum_one",
            weights(0.6, -0.1, 0.5),
            Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
        ),
        case(
            "negative_periodic_sum_one",
            weights(0.6, 0.5, -0.1),
            Some(CALYX_TEMPORAL_NEGATIVE_WEIGHT),
        ),
        case(
            "nan_recency",
            weights(f32::NAN, 0.5, 0.5),
            Some(CALYX_TEMPORAL_WEIGHT_SUM),
        ),
        case(
            "infinite_sequence",
            weights(0.5, f32::INFINITY, 0.5),
            Some(CALYX_TEMPORAL_WEIGHT_SUM),
        ),
        case(
            "sum_not_one",
            weights(0.4, 0.4, 0.3),
            Some(CALYX_TEMPORAL_WEIGHT_SUM),
        ),
    ]
}

fn run_case(case: ValidationCase) -> Value {
    let result = case.weights.validate();
    let expected_ok = case.expected_code.is_none();
    let actual = match result {
        Ok(()) => json!({"ok": true, "error": null}),
        Err(error) => json!({
            "ok": false,
            "error": {
                "code": error.code,
                "message": error.message,
                "remediation": error.remediation,
            }
        }),
    };
    let expected = json!({
        "ok": expected_ok,
        "code": case.expected_code,
    });
    json!({
        "name": case.name,
        "input": weight_labels(case.weights),
        "sum": f32_label(case.weights.recency + case.weights.sequence + case.weights.periodic),
        "expected": expected,
        "actual": actual,
    })
}

fn assert_case(readback: &Value, name: &str, expected_code: Option<&str>) {
    let case = readback["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing case {name}"));
    assert_eq!(case["actual"]["ok"], json!(expected_code.is_none()));
    assert_eq!(case["actual"]["error"]["code"], json!(expected_code));
}

fn case(
    name: &'static str,
    weights: FusionWeights,
    expected_code: Option<&'static str>,
) -> ValidationCase {
    ValidationCase {
        name,
        weights,
        expected_code,
    }
}

fn weights(recency: f32, sequence: f32, periodic: f32) -> FusionWeights {
    FusionWeights {
        recency,
        sequence,
        periodic,
    }
}

fn weight_labels(weights: FusionWeights) -> Value {
    json!({
        "recency": f32_label(weights.recency),
        "sequence": f32_label(weights.sequence),
        "periodic": f32_label(weights.periodic),
    })
}

fn f32_label(value: f32) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f32::INFINITY {
        "Infinity".to_string()
    } else if value == f32::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        value.to_string()
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json");
}

fn write_blake3_sums(root: &Path) {
    let mut lines = Vec::new();
    for relative in list_files(root) {
        if relative == "BLAKE3SUMS.txt" {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksum input");
        lines.push(format!("{}  {}", blake3::hash(&bytes), relative));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines.join("\n")).expect("write sums");
}

fn list_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if let Ok(relative) = path.strip_prefix(root) {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn reset_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).expect("create fsv root");
}

fn fsv_root() -> (PathBuf, bool) {
    let keep = std::env::var_os("CALYX_FUSION_WEIGHTS_FSV_ROOT").is_some();
    let dir = std::env::var_os("CALYX_FUSION_WEIGHTS_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("calyx-fusion-weights-fsv-{}", std::process::id()))
        });
    (dir, keep)
}
