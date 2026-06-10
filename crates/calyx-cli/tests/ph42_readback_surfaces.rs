use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const SURFACES: &[&str] = &[
    "assay-report",
    "temporal-cross-term",
    "kernel-weights",
    "kernel-window",
    "ward-novelty",
    "compression-ratio",
    "anneal-schedule",
];

const ARTIFACT_SCHEMA_VERSION: u64 = 1;
const ARTIFACT_SOURCE_OF_TRUTH: &str = "PH42 persisted artifact";

#[test]
fn ph42_readback_surfaces_are_helped_and_read_artifact_bytes() {
    let root = reset_root();
    let help = command(&["--help"]);
    assert_success(&help);
    let help_stdout = stdout(&help);
    for surface in SURFACES {
        assert!(help_stdout.contains(surface), "missing {surface} in help");
    }

    for (index, surface) in SURFACES.iter().enumerate() {
        let artifact = write_artifact(&root, surface, index as u64);
        let output = command(&[
            "readback",
            surface,
            "--artifact",
            &display(&artifact),
            "--field",
            "metrics.value",
        ]);
        assert_success(&output);
        let readback: Value = serde_json::from_slice(&output.stdout).expect("parse readback");
        let artifact_bytes = fs::read(&artifact).expect("read artifact");
        assert_eq!(readback["surface"], json!(surface));
        assert_eq!(readback["artifact_kind"], json!(artifact_kind(surface)));
        assert_eq!(readback["schema_version"], json!(ARTIFACT_SCHEMA_VERSION));
        assert_eq!(readback["artifact_len"], json!(artifact_bytes.len()));
        assert_eq!(
            readback["artifact_blake3"],
            json!(blake3::hash(&artifact_bytes).to_string())
        );
        assert_eq!(readback["field"], json!("metrics.value"));
        assert_eq!(readback["value"], json!(index as u64));
    }

    let missing = command(&[
        "readback",
        "assay-report",
        "--artifact",
        &display(&root.join("assay-report.json")),
        "--field",
        "missing.path",
    ]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("missing segment"));

    let _ = fs::remove_dir_all(root);
}

fn write_artifact(root: &Path, surface: &str, value: u64) -> PathBuf {
    let path = root.join(format!("{surface}.json"));
    let artifact = json!({
        "schema_version": ARTIFACT_SCHEMA_VERSION,
        "surface": surface,
        "artifact_kind": artifact_kind(surface),
        "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
        "metrics": {
            "value": value,
            "verdict": "byte-readback",
        },
    });
    fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).expect("write artifact");
    path
}

fn artifact_kind(surface: &str) -> String {
    format!("ph42.{surface}.v1")
}

fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("run calyx")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn reset_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-ph42-readback-surfaces-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create root");
    root
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
