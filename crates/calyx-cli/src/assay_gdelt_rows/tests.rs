use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::args::Args;
use super::convert::run;

#[test]
fn converts_real_shape_rows_with_event_times_and_manifest_hashes() {
    let root = temp_root("happy");
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("20240102000000.export.CSV"),
        format!(
            "{}\n{}\n",
            gdelt_line("100", "CAN", "", "CA", "Canada", "20240102000000"),
            gdelt_line("101", "USA", "", "US", "United States", "20240102001500")
        ),
    )
    .unwrap();
    let args = parse(&[
        "--source-dir",
        source.to_str().unwrap(),
        "--out",
        root.join("rows.jsonl").to_str().unwrap(),
        "--manifest",
        root.join("manifest.json").to_str().unwrap(),
        "--limit-per-class",
        "1",
    ]);

    let report = run(&args).unwrap();

    assert_eq!(report.rows, 2);
    assert!(root.join("rows.jsonl").exists());
    assert!(root.join("manifest.json").exists());
    let rows = fs::read_to_string(root.join("rows.jsonl")).unwrap();
    assert!(rows.contains("\"event_time\":\"2024-01-02T00:00:00Z\""));
    assert!(rows.contains("\"label\":1"));
    let first_row: serde_json::Value = serde_json::from_str(rows.lines().next().unwrap()).unwrap();
    assert_eq!(first_row["anchor_leaks_into_input"], true);
    assert_eq!(first_row["anchor_audit"]["grounded_gate_eligible"], false);
    let manifest = fs::read_to_string(root.join("manifest.json")).unwrap();
    assert!(manifest.contains("rows_jsonl_sha256"));
    assert!(manifest.contains("calyx-gdelt-rows-source-v1"));
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["anchor_leaks_into_input"], true);
    assert_eq!(manifest["anchor_audit"]["trivial_anchor"], true);
}

#[test]
fn existing_output_fails_before_mutating() {
    let root = temp_root("exists");
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("20240102000000.export.CSV"),
        gdelt_line("100", "CAN", "", "CA", "Canada", "20240102000000"),
    )
    .unwrap();
    let out = root.join("rows.jsonl");
    fs::write(&out, "keep-me").unwrap();
    let args = parse(&[
        "--source-dir",
        source.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--manifest",
        root.join("manifest.json").to_str().unwrap(),
    ]);

    let error = run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_GDELT_OUTPUT_EXISTS");
    assert_eq!(fs::read_to_string(out).unwrap(), "keep-me");
}

#[test]
fn malformed_row_fails_and_leaves_outputs_absent() {
    let root = temp_root("bad-row");
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("20240102000000.export.CSV"), "too\tshort\n").unwrap();
    let out = root.join("rows.jsonl");
    let manifest = root.join("manifest.json");
    let args = parse(&[
        "--source-dir",
        source.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);

    let error = run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_GDELT_ROW_MALFORMED");
    assert!(!out.exists());
    assert!(!manifest.exists());
}

#[test]
fn invalid_dateadded_fails_closed() {
    let root = temp_root("bad-date");
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("20240102000000.export.CSV"),
        gdelt_line("100", "CAN", "", "CA", "Canada", "bad-date"),
    )
    .unwrap();
    let args = parse(&[
        "--source-dir",
        source.to_str().unwrap(),
        "--out",
        root.join("rows.jsonl").to_str().unwrap(),
        "--manifest",
        root.join("manifest.json").to_str().unwrap(),
    ]);

    let error = run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_GDELT_INVALID_DATEADDED");
}

fn parse(args: &[&str]) -> Args {
    Args::parse(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>()).unwrap()
}

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("calyx-gdelt-{name}-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn gdelt_line(
    event_id: &str,
    actor1_country: &str,
    actor2_country: &str,
    action_country: &str,
    action_full: &str,
    date_added: &str,
) -> String {
    let mut fields = vec![""; 61];
    fields[0] = event_id;
    fields[1] = "20240102";
    fields[6] = "ACTOR1";
    fields[7] = actor1_country;
    fields[16] = "ACTOR2";
    fields[17] = actor2_country;
    fields[26] = "010";
    fields[28] = "01";
    fields[29] = "1";
    fields[30] = "0.0";
    fields[34] = "1.5";
    fields[52] = action_full;
    fields[53] = action_country;
    fields[59] = date_added;
    fields[60] = "https://example.invalid/source";
    fields.join("\t")
}
