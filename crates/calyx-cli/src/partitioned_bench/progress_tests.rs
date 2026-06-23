use super::*;
use std::path::{Path, PathBuf};

#[test]
fn partitioned_build_progress_file_records_complete_readback() {
    let root = temp_root("progress-complete");
    let corpus = root.join("corpus.i8bin");
    let vault = root.join("vault");
    let progress = root.join("progress.json");
    write_i8bin(&corpus, 2, &[&[10, 0], &[9, 1], &[0, 10], &[1, 9]]);

    run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&corpus),
        "--regions".into(),
        "2".into(),
        "--sample".into(),
        "4".into(),
        "--chunk".into(),
        "2".into(),
        "--m-max".into(),
        "2".into(),
        "--ef".into(),
        "4".into(),
        "--region-build-parallelism".into(),
        "1".into(),
        "--progress-file".into(),
        path_arg(&progress),
    ])
    .expect("build partitioned vault with progress readback");

    let snapshot = read_json(&progress);
    assert_eq!(snapshot["format"], "calyx-partitioned-build-progress-v1");
    assert_eq!(snapshot["trigger"], "calyx build-partitioned-vault");
    assert_eq!(snapshot["phase"], "complete");
    assert_eq!(snapshot["exit_code"], 0);
    assert!(snapshot["error_code"].is_null());
    assert_eq!(snapshot["vault"], path_arg(&vault));
    assert_eq!(snapshot["geometry"]["n_cx"], 4);
    assert_eq!(snapshot["geometry"]["dim"], 2);
    assert_eq!(snapshot["geometry"]["requested_regions"], 2);
    assert_eq!(snapshot["geometry"]["build_backend"], "cpu-vamana");
    assert_eq!(snapshot["geometry"]["distance_metric"], "unit-l2");
    assert!(snapshot["counts"]["manifest_exists"].as_bool().unwrap());
    assert!(snapshot["counts"]["final_ids_files"].as_u64().unwrap() > 0);
    assert!(snapshot["counts"]["graph_files"].as_u64().unwrap() > 0);
    assert!(vault.join("partitioned-manifest.json").is_file());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partitioned_build_progress_file_records_prepare_failure() {
    let root = temp_root("progress-failure");
    let unsupported = root.join("corpus.bin");
    let vault = root.join("vault");
    let progress = root.join("progress.json");
    std::fs::write(&unsupported, b"not a supported vector extension").unwrap();

    let err = run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&unsupported),
        "--regions".into(),
        "2".into(),
        "--progress-file".into(),
        path_arg(&progress),
    ])
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(!vault.exists());
    let snapshot = read_json(&progress);
    assert_eq!(snapshot["phase"], "failed");
    assert_eq!(snapshot["exit_code"], 2);
    assert_eq!(snapshot["error_code"], "CALYX_CLI_USAGE_ERROR");
    assert!(
        snapshot["error_message"]
            .as_str()
            .unwrap()
            .contains("must end in .fbin or .i8bin")
    );
    assert!(!snapshot["counts"]["manifest_exists"].as_bool().unwrap());
    assert_eq!(snapshot["counts"]["final_ids_files"], 0);
    assert_eq!(snapshot["counts"]["graph_files"], 0);
    let _ = std::fs::remove_dir_all(root);
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-partitioned-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_i8bin(path: &Path, dim: u32, rows: &[&[i8]]) {
    let mut bytes = Vec::with_capacity(8 + rows.len() * dim as usize);
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&dim.to_le_bytes());
    for row in rows {
        assert_eq!(row.len(), dim as usize);
        bytes.extend(row.iter().map(|value| *value as u8));
    }
    std::fs::write(path, bytes).unwrap();
}
