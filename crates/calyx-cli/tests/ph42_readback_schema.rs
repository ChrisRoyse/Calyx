use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const ARTIFACT_SCHEMA_VERSION: u64 = 1;
const ARTIFACT_SOURCE_OF_TRUTH: &str = "PH42 persisted artifact";
const SCHEMA_ERROR: &str = "CALYX_PH42_ARTIFACT_SCHEMA";

#[test]
fn ph42_readback_rejects_mismatched_or_malformed_artifacts() {
    let root = reset_temp_root("calyx-ph42-readback-schema");
    let cases = schema_reject_cases();

    for case in cases {
        let artifact = write_artifact(&root, case.name, &case.artifact);
        let output = readback(case.surface, &artifact, Some("metrics.value"));
        assert!(
            !output.status.success(),
            "case {} unexpectedly passed",
            case.name
        );
        let stderr = stderr(&output);
        assert!(
            stderr.contains(SCHEMA_ERROR),
            "case {} missing schema error: {stderr}",
            case.name
        );
        assert!(
            stderr.contains(case.expected_detail),
            "case {} missing detail {}: {stderr}",
            case.name,
            case.expected_detail
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
#[ignore = "manual FSV for #654 PH42 artifact schema validation"]
fn issue654_ph42_artifact_schema_fsv_writes_readbacks() {
    let root = reset_fsv_root();
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts).expect("create artifacts root");

    let valid_path = write_artifact(
        &artifacts,
        "valid-assay",
        &valid_artifact("assay-report", 42),
    );
    let cases = schema_reject_cases();
    let reject_paths: Vec<_> = cases
        .iter()
        .map(|case| write_artifact(&artifacts, case.name, &case.artifact))
        .collect();

    let mut evidence = json!({
        "schema_contract": {
            "schema_version": ARTIFACT_SCHEMA_VERSION,
            "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
            "artifact_kind_pattern": "ph42.<surface>.v1",
        },
        "happy": {
            "before": artifact_state(&valid_path),
            "run": output_state(readback("assay-report", &valid_path, Some("metrics.value"))),
            "after": artifact_state(&valid_path),
        },
        "rejects": {},
    });

    for (case, path) in cases.iter().zip(reject_paths.iter()) {
        evidence["rejects"][case.name] = json!({
            "expected_detail": case.expected_detail,
            "before": artifact_state(path),
            "run": output_state(readback(case.surface, path, Some("metrics.value"))),
            "after": artifact_state(path),
        });
    }

    let readback_path = root.join("issue654-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .expect("write readback");
    let manifest_path = write_blake3_manifest(
        &root,
        &all_artifact_files(&valid_path, &reject_paths, &readback_path),
    );

    assert_eq!(evidence["happy"]["run"]["status"], json!(0));
    assert_eq!(evidence["happy"]["run"]["stdout_json"]["value"], json!(42));
    assert_same_artifact_state(&evidence["happy"]);
    for case in cases {
        let result = &evidence["rejects"][case.name];
        assert_eq!(result["run"]["success"], json!(false));
        assert!(
            result["run"]["stderr"]
                .as_str()
                .unwrap()
                .contains(SCHEMA_ERROR)
        );
        assert!(
            result["run"]["stderr"]
                .as_str()
                .unwrap()
                .contains(case.expected_detail)
        );
        assert_same_artifact_state(result);
    }

    println!("ISSUE654_PH42_SCHEMA_FSV_ROOT={}", root.display());
    println!("ISSUE654_PH42_SCHEMA_READBACK={}", readback_path.display());
    println!("ISSUE654_PH42_SCHEMA_BLAKE3={}", manifest_path.display());
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
}

struct RejectCase {
    name: &'static str,
    surface: &'static str,
    artifact: Value,
    expected_detail: &'static str,
}

fn schema_reject_cases() -> Vec<RejectCase> {
    vec![
        RejectCase {
            name: "mismatched-surface",
            surface: "assay-report",
            artifact: valid_artifact("kernel-weights", 10),
            expected_detail: "surface mismatch",
        },
        RejectCase {
            name: "missing-kind",
            surface: "assay-report",
            artifact: json!({
                "schema_version": ARTIFACT_SCHEMA_VERSION,
                "surface": "assay-report",
                "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
                "metrics": {"value": 11},
            }),
            expected_detail: "missing required string field artifact_kind",
        },
        RejectCase {
            name: "wrong-kind",
            surface: "assay-report",
            artifact: json!({
                "schema_version": ARTIFACT_SCHEMA_VERSION,
                "surface": "assay-report",
                "artifact_kind": artifact_kind("kernel-weights"),
                "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
                "metrics": {"value": 13},
            }),
            expected_detail: "artifact_kind mismatch",
        },
        RejectCase {
            name: "invalid-schema-version",
            surface: "assay-report",
            artifact: json!({
                "schema_version": 2,
                "surface": "assay-report",
                "artifact_kind": artifact_kind("assay-report"),
                "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
                "metrics": {"value": 12},
            }),
            expected_detail: "schema_version mismatch",
        },
        RejectCase {
            name: "arbitrary-json",
            surface: "assay-report",
            artifact: json!(["not", "a", "ph42", "artifact"]),
            expected_detail: "root must be a JSON object",
        },
    ]
}

fn valid_artifact(surface: &str, value: u64) -> Value {
    json!({
        "schema_version": ARTIFACT_SCHEMA_VERSION,
        "surface": surface,
        "artifact_kind": artifact_kind(surface),
        "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
        "metrics": {
            "value": value,
            "verdict": "byte-readback",
        },
    })
}

fn artifact_kind(surface: &str) -> String {
    format!("ph42.{surface}.v1")
}

fn write_artifact(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(format!("{name}.json"));
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).expect("write artifact");
    path
}

fn readback(surface: &str, artifact: &Path, field: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    command
        .arg("readback")
        .arg(surface)
        .arg("--artifact")
        .arg(artifact);
    if let Some(field) = field {
        command.arg("--field").arg(field);
    }
    command.output().expect("run calyx readback")
}

fn output_state(output: Output) -> Value {
    let stdout = stdout(&output);
    let stdout_json = serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null);
    json!({
        "status": output.status.code(),
        "success": output.status.success(),
        "stdout": stdout,
        "stdout_json": stdout_json,
        "stderr": stderr(&output),
    })
}

fn artifact_state(path: &Path) -> Value {
    let bytes = fs::read(path).expect("read artifact bytes");
    let json_value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    json!({
        "path": display(path),
        "len": bytes.len(),
        "blake3": blake3::hash(&bytes).to_string(),
        "hex_prefix": bytes.iter().take(64).map(|byte| format!("{byte:02x}")).collect::<String>(),
        "json": json_value,
    })
}

fn assert_same_artifact_state(value: &Value) {
    assert_eq!(value["before"]["len"], value["after"]["len"]);
    assert_eq!(value["before"]["blake3"], value["after"]["blake3"]);
    assert_eq!(value["before"]["hex_prefix"], value["after"]["hex_prefix"]);
}

fn all_artifact_files(valid: &Path, rejects: &[PathBuf], readback: &Path) -> Vec<PathBuf> {
    let mut files = vec![valid.to_path_buf(), readback.to_path_buf()];
    files.extend(rejects.iter().cloned());
    files
}

fn write_blake3_manifest(root: &Path, files: &[PathBuf]) -> PathBuf {
    let mut sorted = files.to_vec();
    sorted.sort();
    let mut manifest = String::new();
    for path in sorted {
        let bytes = fs::read(&path).expect("read manifest input");
        let relative = path
            .strip_prefix(root)
            .expect("manifest path under root")
            .display()
            .to_string()
            .replace('\\', "/");
        manifest.push_str(&format!("{}  {relative}\n", blake3::hash(&bytes)));
    }
    let path = root.join("BLAKE3SUMS.txt");
    fs::write(&path, manifest).expect("write BLAKE3 manifest");
    path
}

fn reset_temp_root(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    reset_root(root)
}

fn reset_fsv_root() -> PathBuf {
    let root = std::env::var_os("CALYX_CLI_ISSUE654_FSV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| reset_temp_root("calyx-issue654-ph42-schema-fsv"));
    reset_root(root)
}

fn reset_root(root: PathBuf) -> PathBuf {
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create root");
    root
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
