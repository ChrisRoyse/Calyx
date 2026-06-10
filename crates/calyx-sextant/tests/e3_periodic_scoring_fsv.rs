use std::fs;
use std::path::{Path, PathBuf};

use calyx_sextant::{PeriodicOptions, score_e3_periodic};
use serde_json::{Value, json};

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEDNESDAY_2024_01_03_15H_UTC: i64 = TUESDAY_2024_01_02_14H_UTC + 86_400 + 3_600;

#[test]
fn e3_periodic_scoring_modes_fsv() {
    let (root, keep_root) = fsv_root();
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let cases = vec![
        run_case(
            "configured_target_ignores_query_time",
            TUESDAY_2024_01_02_14H_UTC,
            WEDNESDAY_2024_01_03_15H_UTC,
            PeriodicOptions::new(Some(14), Some(1)).expect("configured periodic"),
            0,
            1.0,
        ),
        run_case(
            "query_time_same_hour_day",
            TUESDAY_2024_01_02_14H_UTC,
            TUESDAY_2024_01_02_14H_UTC,
            PeriodicOptions::from_query_time(),
            0,
            1.0,
        ),
        run_case(
            "query_time_mismatched_hour_day",
            TUESDAY_2024_01_02_14H_UTC,
            WEDNESDAY_2024_01_03_15H_UTC,
            PeriodicOptions::from_query_time(),
            0,
            0.0,
        ),
    ];
    let mut readback = json!({
        "before": before,
        "cases": cases,
        "after": null,
    });
    let artifact = root.join("e3-periodic-scoring-readback.json");
    write_json(&artifact, &readback);
    readback["after"] = json!({"files_after_artifact_write": list_files(&root)});
    write_json(&artifact, &readback);
    let bytes = fs::read(&artifact).expect("read artifact");
    let reread: Value = serde_json::from_slice(&bytes).expect("parse artifact reread");
    write_blake3_sums(&root);

    assert_eq!(readback, reread);
    assert_score(&reread, "configured_target_ignores_query_time", 1.0);
    assert_score(&reread, "query_time_same_hour_day", 1.0);
    assert_score(&reread, "query_time_mismatched_hour_day", 0.0);

    println!("e3_periodic_scoring_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&reread).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn run_case(
    name: &str,
    event_time_secs: i64,
    query_time_secs: i64,
    options: PeriodicOptions,
    tz_offset_secs: i32,
    expected_score: f32,
) -> Value {
    let score = score_e3_periodic(event_time_secs, query_time_secs, &options, tz_offset_secs);
    json!({
        "name": name,
        "event_time_secs": event_time_secs,
        "query_time_secs": query_time_secs,
        "tz_offset_secs": tz_offset_secs,
        "options": options,
        "expected_score": expected_score,
        "actual_score": score,
        "matches_expected": (score - expected_score).abs() <= f32::EPSILON,
    })
}

fn assert_score(readback: &Value, name: &str, expected: f32) {
    let case = readback["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing case {name}"));
    let actual = case["actual_score"].as_f64().expect("actual score");
    assert!(
        (actual - f64::from(expected)).abs() <= f64::from(f32::EPSILON),
        "case {name} actual={actual} expected={expected}"
    );
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
    let keep = std::env::var_os("CALYX_E3_PERIODIC_FSV_ROOT").is_some();
    let dir = std::env::var_os("CALYX_E3_PERIODIC_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("calyx-e3-periodic-fsv-{}", std::process::id()))
        });
    (dir, keep)
}
