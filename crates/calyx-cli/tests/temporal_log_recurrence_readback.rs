use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

#[test]
fn temporal_log_recurrence_readback_writes_real_log_artifact_shape() {
    let root = reset_root("happy");
    let log = root.join("machine_temperature.csv");
    write_regular_log(&log, 12);
    let vault = root.join("vault");
    let out = root.join("temporal-log-recurrence.json");

    let output = command(&[
        "readback",
        "temporal-log-recurrence",
        "--log",
        &display(&log),
        "--vault",
        &display(&vault),
        "--out",
        &display(&out),
        "--rows",
        "12",
        "--expected-cadence-secs",
        "300",
        "--confidence-ceiling",
        "0.91",
    ]);
    assert_success(&output);
    assert!(out.exists(), "artifact should be written");

    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout json");
    let artifact: Value =
        serde_json::from_slice(&fs::read(&out).expect("read artifact")).expect("artifact json");
    assert_eq!(stdout, artifact);
    assert_eq!(
        artifact["artifact_kind"],
        json!("ph70.temporal-real-log-recurrence.v1")
    );
    assert_eq!(artifact["input_log"]["selected_rows"], json!(12));
    assert_eq!(artifact["expected"]["cadence_secs"], json!(300));
    assert_eq!(
        artifact["expected"]["next_occurrence_epoch_secs"],
        json!(1_704_207_600_i64)
    );
    assert_eq!(
        artifact["actual"]["recurrence"]["series"]["cadence_secs"],
        json!(300.0)
    );
    assert_eq!(
        artifact["actual"]["recurrence"]["periodic_fit"]["dominant_period_secs"],
        json!(300.0)
    );
    assert_eq!(
        artifact["actual"]["prediction"]["t_hat"],
        json!(1_704_207_600_i64)
    );
    assert_eq!(artifact["checks"]["recurrence_signature_count"], json!(11));
    assert_eq!(
        artifact["actual"]["raw_cf"]["recurrence"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        artifact["actual"]["raw_cf"]["ledger"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn temporal_log_recurrence_edges_fail_closed_without_artifact() {
    let root = reset_root("edges");
    let cases = [
        ("empty", "timestamp,value\n", "CALYX_TEMPORAL_LOG_EMPTY"),
        (
            "bad_timestamp",
            "timestamp,value\nnot-a-time,1\n2024-01-02 14:05:00,2\n2024-01-02 14:10:00,3\n",
            "CALYX_TEMPORAL_LOG_BAD_TIMESTAMP",
        ),
        (
            "non_ascii_timestamp",
            "timestamp,value\n2024-01-02 14:1é00,1\n2024-01-02 14:05:00,2\n2024-01-02 14:10:00,3\n",
            "CALYX_TEMPORAL_LOG_BAD_TIMESTAMP",
        ),
        (
            "non_monotonic",
            "timestamp,value\n2024-01-02 14:00:00,1\n2024-01-02 14:05:00,2\n2024-01-02 14:05:00,3\n",
            "CALYX_TEMPORAL_LOG_NON_MONOTONIC",
        ),
        (
            "cadence_mismatch",
            "timestamp,value\n2024-01-02 14:00:00,1\n2024-01-02 14:05:00,2\n2024-01-02 14:20:00,3\n",
            "CALYX_TEMPORAL_LOG_CADENCE_MISMATCH",
        ),
    ];

    for (name, body, code) in cases {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("case dir");
        let log = dir.join("log.csv");
        let vault = dir.join("vault");
        let out = dir.join("artifact.json");
        fs::write(&log, body).expect("write log");

        let output = command(&[
            "readback",
            "temporal-log-recurrence",
            "--log",
            &display(&log),
            "--vault",
            &display(&vault),
            "--out",
            &display(&out),
            "--rows",
            "12",
            "--expected-cadence-secs",
            "300",
            "--confidence-ceiling",
            "0.91",
        ]);
        assert!(
            !output.status.success(),
            "{name} should fail, stdout={}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(code), "{name} stderr={stderr}");
        assert!(!out.exists(), "{name} must not write artifact");
    }
}

fn write_regular_log(path: &Path, rows: usize) {
    let mut text = String::from("timestamp,value\n");
    for index in 0..rows {
        text.push_str(&format!(
            "2024-01-02 14:{:02}:00,{}\n",
            index * 5,
            70 + index
        ));
    }
    fs::write(path, text).expect("write regular log");
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
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reset_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-temporal-log-recurrence-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create root");
    root
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
