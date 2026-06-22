use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const TABLES: &[&str] = &[
    "creator_databases",
    "queries",
    "billing",
    "marketplace",
    "outbox",
];

#[test]
fn issue612_cli_validates_latency_and_widened_pg_tables() {
    let root = reset_temp_root("calyx-issue612");
    write_fixture(&root, FixtureMode::Pass);
    let out = root.join("evidence.json");

    let output = run_issue612(&root, &out);
    assert!(output.status.success(), "{}", stderr(&output));

    let stdout_json: Value = serde_json::from_str(&stdout(&output)).expect("stdout json");
    let artifact: Value =
        serde_json::from_slice(&fs::read(&out).expect("read evidence")).expect("evidence json");
    assert_eq!(stdout_json["latency"]["baseline_p99_us"], json!(1098));
    assert_eq!(stdout_json["latency"]["flipped_p99_us"], json!(998));
    assert_eq!(
        artifact["control_plane"]["matched_tables"],
        json!(TABLES.len())
    );
    for table in TABLES {
        assert!(
            artifact["control_plane"]["tables"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["table"] == json!(table)),
            "missing table {table}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn issue612_cli_edges_fail_closed_without_output() {
    let cases = [
        (FixtureMode::EmptyLatency, "CALYX_LATENCY_SAMPLE_EMPTY"),
        (FixtureMode::LatencyRegression, "CALYX_LATENCY_REGRESSION"),
        (
            FixtureMode::MissingMarketplace,
            "CALYX_PG_SNAPSHOT_INCOMPLETE",
        ),
        (FixtureMode::OutboxChanged, "CALYX_PG_STATE_CHANGED"),
    ];

    for (mode, code) in cases {
        let root = reset_temp_root(&format!("calyx-issue612-{mode:?}"));
        write_fixture(&root, mode);
        let out = root.join("evidence.json");
        let before = file_state(&out);
        let output = run_issue612(&root, &out);
        let after = file_state(&out);

        assert!(
            !output.status.success(),
            "case {mode:?} unexpectedly passed"
        );
        assert!(
            stderr(&output).contains(code),
            "case {mode:?}: {}",
            stderr(&output)
        );
        assert_eq!(before["exists"], json!(false));
        assert_eq!(after["exists"], json!(false));

        let _ = fs::remove_dir_all(root);
    }
}

#[test]
#[ignore = "requires CALYX_ISSUE612_FSV_ROOT in a manual verification run"]
fn issue612_latency_and_pg_snapshot_fsv_writes_readbacks() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE612_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    write_fixture(&root, FixtureMode::Pass);

    let out = root.join("evidence.json");
    let before = file_state(&out);
    let output = run_issue612(&root, &out);
    let after = file_state(&out);
    assert!(output.status.success(), "{}", stderr(&output));

    let edge_root = root.join("edges");
    fs::create_dir_all(&edge_root).expect("create edge root");
    let edges = json!({
        "empty_latency": run_edge(&edge_root, FixtureMode::EmptyLatency),
        "latency_regression": run_edge(&edge_root, FixtureMode::LatencyRegression),
        "missing_marketplace": run_edge(&edge_root, FixtureMode::MissingMarketplace),
        "outbox_changed": run_edge(&edge_root, FixtureMode::OutboxChanged),
    });
    let evidence = json!({
        "issue": 612,
        "trigger": "calyx leapable issue612-fsv over latency samples and widened pg_dump directories",
        "source_of_truth": {
            "root": display(&root),
            "artifact": display(&out),
            "pg_before": display(&root.join("pg_before")),
            "pg_after": display(&root.join("pg_after")),
        },
        "known_io": {
            "baseline_p99_us": 1098,
            "flipped_p99_us": 998,
            "allowed_p99_us": 1152.9,
            "required_tables": TABLES,
        },
        "happy": {
            "before": before,
            "after": after,
            "stdout": output_state(output),
            "artifact": serde_json::from_slice::<Value>(&fs::read(&out).unwrap()).unwrap(),
        },
        "edges": edges,
    });
    let readback = root.join("issue612-fsv-readback.json");
    write_json(&readback, &evidence);
    let manifest = write_blake3_manifest(&root);

    assert_eq!(
        evidence["happy"]["artifact"]["latency"]["baseline_p99_us"],
        json!(1098)
    );
    assert_eq!(
        evidence["happy"]["artifact"]["latency"]["flipped_p99_us"],
        json!(998)
    );
    assert_eq!(
        evidence["happy"]["artifact"]["control_plane"]["matched_tables"],
        json!(TABLES.len())
    );

    println!("ISSUE612_FSV_ROOT={}", root.display());
    println!("ISSUE612_EVIDENCE={}", readback.display());
    println!("ISSUE612_BLAKE3={}", manifest.display());
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
}

#[derive(Clone, Copy, Debug)]
enum FixtureMode {
    Pass,
    EmptyLatency,
    LatencyRegression,
    MissingMarketplace,
    OutboxChanged,
}

fn write_fixture(root: &Path, mode: FixtureMode) {
    fs::create_dir_all(root).expect("create root");
    let baseline: Vec<u64> = (1000..1100).collect();
    let flipped: Vec<u64> = match mode {
        FixtureMode::EmptyLatency => Vec::new(),
        FixtureMode::LatencyRegression => (1200..1300).collect(),
        _ => (900..1000).collect(),
    };
    write_json(
        &root.join("baseline_latency.json"),
        &json!({"path": "sqlite-vec", "samples_us": baseline}),
    );
    write_json(
        &root.join("flipped_latency.json"),
        &json!({"path": "calyx-flipped", "samples_us": flipped}),
    );
    let before = root.join("pg_before");
    let after = root.join("pg_after");
    fs::create_dir_all(&before).expect("create before");
    fs::create_dir_all(&after).expect("create after");
    for table in TABLES {
        if matches!(mode, FixtureMode::MissingMarketplace) && *table == "marketplace" {
            write_dump(&before, table, "marketplace row\n");
            continue;
        }
        let mut content = format!("table={table}\nvault=issue612_fixture\nrow_count=1\n");
        if matches!(mode, FixtureMode::OutboxChanged) && *table == "outbox" {
            write_dump(&before, table, &content);
            content.push_str("changed_after=1\n");
            write_dump(&after, table, &content);
            continue;
        }
        write_dump(&before, table, &content);
        write_dump(&after, table, &content);
    }
}

fn run_issue612(root: &Path, out: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("leapable")
        .arg("issue612-fsv")
        .arg("--baseline-latency")
        .arg(root.join("baseline_latency.json"))
        .arg("--flipped-latency")
        .arg(root.join("flipped_latency.json"))
        .arg("--pg-before")
        .arg(root.join("pg_before"))
        .arg("--pg-after")
        .arg(root.join("pg_after"))
        .arg("--out")
        .arg(out)
        .output()
        .expect("run issue612 fsv")
}

fn run_edge(root: &Path, mode: FixtureMode) -> Value {
    let case_root = root.join(format!("{mode:?}"));
    write_fixture(&case_root, mode);
    let out = case_root.join("evidence.json");
    let before = file_state(&out);
    let output = run_issue612(&case_root, &out);
    let after = file_state(&out);
    json!({
        "before": before,
        "after": after,
        "stdout": stdout(&output),
        "stderr": stderr(&output),
        "success": output.status.success(),
    })
}

fn write_dump(root: &Path, table: &str, content: &str) {
    fs::write(root.join(format!("{table}.dump")), content).expect("write dump")
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create json parent");
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json")
}

fn file_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({"path": display(path), "exists": false});
    }
    let bytes = fs::read(path).expect("read file");
    json!({
        "path": display(path),
        "exists": true,
        "len": bytes.len(),
        "blake3": blake3::hash(&bytes).to_string(),
        "hex_prefix": bytes.iter().take(64).map(|byte| format!("{byte:02x}")).collect::<String>(),
    })
}

fn output_state(output: Output) -> Value {
    let stdout = stdout(&output);
    json!({
        "success": output.status.success(),
        "status": output.status.code(),
        "stdout_json": serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null),
        "stdout": stdout,
        "stderr": stderr(&output),
    })
}

fn write_blake3_manifest(root: &Path) -> PathBuf {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut manifest = String::new();
    for path in files {
        let bytes = fs::read(&path).expect("read manifest input");
        let relative = path
            .strip_prefix(root)
            .expect("relative")
            .display()
            .to_string()
            .replace('\\', "/");
        manifest.push_str(&format!("{}  {relative}\n", blake3::hash(&bytes)));
    }
    let path = root.join("BLAKE3SUMS.txt");
    fs::write(&path, manifest).expect("write manifest");
    path
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(current).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path != root.join("BLAKE3SUMS.txt") {
            files.push(path);
        }
    }
}

fn reset_temp_root(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");
    root
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE612_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE612_FSV_ROOT to a fresh manual verification path")
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
