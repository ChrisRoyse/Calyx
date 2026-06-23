use super::*;
use std::path::{Path, PathBuf};

#[test]
fn partitioned_search_parses_recall_floor() {
    let args = strings([
        "--vault",
        "vault",
        "--ground-truth",
        "200",
        "--ground-truth-file",
        "truth.i32bin",
        "--ground-truth-id-map",
        "ids.i32bin",
        "--recall-floor",
        "0.85",
    ]);

    let parsed = SearchArgs::parse(&args).unwrap();

    assert_eq!(parsed.ground_truth, 200);
    assert_eq!(
        parsed.ground_truth_id_map.as_deref(),
        Some(Path::new("ids.i32bin"))
    );
    assert_eq!(parsed.recall_floor, Some(0.85));
}

#[test]
fn partitioned_search_parses_tuner_status_flags() {
    let args = strings([
        "--vault",
        "vault",
        "--anneal-vault",
        "anneal",
        "--tuner-slo-us",
        "100",
    ]);

    let parsed = SearchArgs::parse(&args).unwrap();

    assert_eq!(parsed.anneal_vault, Some(PathBuf::from("anneal")));
    assert_eq!(parsed.tuner_slo_us, Some(100));
}

#[test]
fn partitioned_search_rejects_zero_tuner_slo() {
    let args = strings(["--vault", "vault", "--tuner-slo-us", "0"]);

    let err = match SearchArgs::parse(&args) {
        Ok(_) => panic!("zero tuner SLO accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--tuner-slo-us must be > 0"));
}

#[test]
fn partitioned_build_parses_region_build_parallelism() {
    let args = strings([
        "--vault",
        "vault",
        "--n-cx",
        "1000",
        "--regions",
        "8",
        "--region-build-parallelism",
        "3",
        "--final-assignment-probe",
        "128",
        "--final-assignment-cap",
        "8192",
        "--progress-file",
        "progress.json",
    ]);

    let parsed = BuildArgs::parse(&args).unwrap();

    assert_eq!(parsed.p.region_build_parallelism, 3);
    assert_eq!(parsed.p.final_assignment_probe, 128);
    assert_eq!(parsed.p.final_assignment_cap, Some(8192));
    assert_eq!(parsed.progress_file, Some(PathBuf::from("progress.json")));
    assert_eq!(
        parsed.distance_metric,
        calyx_sextant::index::PartitionDistanceMetric::UnitL2
    );
}

#[test]
fn partitioned_build_rejects_zero_final_assignment_probe() {
    let args = strings([
        "--vault",
        "vault",
        "--n-cx",
        "1000",
        "--regions",
        "8",
        "--final-assignment-probe",
        "0",
    ]);

    let err = match BuildArgs::parse(&args) {
        Ok(_) => panic!("zero final assignment probe accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("--final-assignment-probe must be > 0")
    );
}

#[test]
fn partitioned_build_rejects_zero_final_assignment_cap() {
    let args = strings([
        "--vault",
        "vault",
        "--n-cx",
        "1000",
        "--regions",
        "8",
        "--final-assignment-cap",
        "0",
    ]);

    let err = match BuildArgs::parse(&args) {
        Ok(_) => panic!("zero final assignment cap accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--final-assignment-cap must be > 0"));
}

#[test]
fn recall_floor_requires_ground_truth_readback() {
    let err = enforce_recall_floor(Some(0.85), 0, None).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_GROUND_TRUTH_REQUIRED");
    assert!(err.message().contains("--ground-truth > 0"));
}

#[test]
fn recall_floor_rejects_low_true_recall() {
    let err = enforce_recall_floor(Some(0.85), 200, Some(0.84)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RECALL_BELOW_FLOOR");
    assert!(err.message().contains("ground_truth_recall_at_k=0.840000"));
}

#[test]
fn recall_floor_accepts_true_recall_at_floor() {
    enforce_recall_floor(Some(0.85), 200, Some(0.85)).unwrap();
}

#[test]
fn ground_truth_id_map_requires_ground_truth_file() {
    let args = strings(["--vault", "vault", "--ground-truth-id-map", "ids.i32bin"]);

    let err = match SearchArgs::parse(&args) {
        Ok(_) => panic!("id map without ground-truth file accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("--ground-truth-id-map requires --ground-truth-file")
    );
}

#[test]
fn i32bin_ground_truth_id_map_translates_ann_row_ids() {
    let root = temp_root("i32bin-id-map");
    let truth = root.join("truth.i32bin");
    let id_map = root.join("ids.i32bin");
    write_i32bin(&truth, 1, &[&[9001]]);
    write_i32bin(&id_map, 1, &[&[7000], &[9001]]);

    let ann = map_ann_rows_to_ground_truth_ids(&id_map, &[vec![1]], 2).unwrap();
    assert_eq!(ann, vec![vec![9001]]);
    assert_eq!(
        recall_from_i32bin_ground_truth(&truth, &ann, 1).unwrap(),
        1.0
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ground_truth_id_map_rejects_non_vector_map() {
    let root = temp_root("i32bin-id-map-width");
    let id_map = root.join("ids.i32bin");
    write_i32bin(&id_map, 2, &[&[1, 2], &[3, 4]]);

    let err = map_ann_rows_to_ground_truth_ids(&id_map, &[vec![1]], 2).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("width must be 1, got 2"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partitioned_search_rejects_zero_probe_count() {
    let args = strings(["--vault", "vault", "--n-probe", "0"]);

    let err = match SearchArgs::parse(&args) {
        Ok(_) => panic!("zero n-probe accepted"),
        Err(err) => err,
    };

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--n-probe must be > 0"));
}

#[test]
fn i8bin_build_and_search_uses_real_vector_files() {
    let root = temp_root("i8bin-happy");
    let corpus = root.join("corpus.i8bin");
    let queries = root.join("queries.i8bin");
    let vault = root.join("vault");
    write_i8bin(
        &corpus,
        3,
        &[
            &[10, 0, 0],
            &[9, 1, 0],
            &[0, 10, 0],
            &[0, 9, 1],
            &[0, 0, 10],
            &[1, 0, 9],
        ],
    );
    write_i8bin(&queries, 3, &[&[10, 0, 0], &[0, 10, 0], &[0, 0, 10]]);

    run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&corpus),
        "--regions".into(),
        "3".into(),
        "--sample".into(),
        "6".into(),
        "--chunk".into(),
        "2".into(),
        "--m-max".into(),
        "4".into(),
        "--ef".into(),
        "8".into(),
        "--region-build-parallelism".into(),
        "1".into(),
    ])
    .expect("build partitioned i8bin vault");

    let manifest_path = vault.join("partitioned-manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["n_cx"], 6);
    assert_eq!(manifest["dim"], 3);
    assert_eq!(manifest["n_regions"], 3);
    assert!(
        vault
            .join(manifest["root_graph_rel"].as_str().unwrap())
            .is_file()
    );
    assert!(
        vault
            .join(manifest["centroids_rel"].as_str().unwrap())
            .metadata()
            .map(|meta| meta.len() > 0)
            .unwrap_or(false)
    );
    assert!(
        manifest["regions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|region| {
                vault
                    .join(region["graph_rel"].as_str().unwrap())
                    .metadata()
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
            })
    );

    run_search(&[
        "--vault".into(),
        path_arg(&vault),
        "--queries".into(),
        path_arg(&queries),
        "--corpus".into(),
        path_arg(&corpus),
        "--n".into(),
        "3".into(),
        "--k".into(),
        "1".into(),
        "--n-probe".into(),
        "3".into(),
        "--region-beam".into(),
        "16".into(),
        "--ground-truth".into(),
        "3".into(),
        "--recall-floor".into(),
        "1.0".into(),
    ])
    .expect("search i8bin vault with exact recall");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn i8bin_build_rejects_bad_vector_inputs_before_creating_vault() {
    let root = temp_root("i8bin-bad-input");
    let unsupported = root.join("corpus.bin");
    let corrupt = root.join("corrupt.i8bin");
    let unsupported_vault = root.join("unsupported-vault");
    let corrupt_vault = root.join("corrupt-vault");
    std::fs::write(&unsupported, b"not a supported vector extension").unwrap();
    std::fs::write(&corrupt, [1_u8, 0, 0, 0, 3, 0, 0, 0, 1, 2]).unwrap();

    let err = run_build(&[
        "--vault".into(),
        path_arg(&unsupported_vault),
        "--vectors".into(),
        path_arg(&unsupported),
        "--regions".into(),
        "2".into(),
    ])
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("must end in .fbin or .i8bin"));
    assert!(!unsupported_vault.exists());

    let err = run_build(&[
        "--vault".into(),
        path_arg(&corrupt_vault),
        "--vectors".into(),
        path_arg(&corrupt),
        "--regions".into(),
        "2".into(),
    ])
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_INDEX_CORRUPT");
    assert!(err.message().contains("len 10 != expected 11"));
    assert!(!corrupt_vault.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn i8bin_search_rejects_query_dimension_mismatch() {
    let root = temp_root("i8bin-dim-mismatch");
    let corpus = root.join("corpus.i8bin");
    let queries = root.join("queries-dim2.i8bin");
    let vault = root.join("vault");
    write_i8bin(&corpus, 3, &[&[10, 0, 0], &[0, 10, 0], &[0, 0, 10]]);
    write_i8bin(&queries, 2, &[&[10, 0]]);
    run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&corpus),
        "--regions".into(),
        "2".into(),
        "--sample".into(),
        "3".into(),
        "--chunk".into(),
        "2".into(),
        "--m-max".into(),
        "2".into(),
        "--ef".into(),
        "4".into(),
        "--region-build-parallelism".into(),
        "1".into(),
    ])
    .expect("build baseline i8bin vault");

    let err = run_search(&[
        "--vault".into(),
        path_arg(&vault),
        "--queries".into(),
        path_arg(&queries),
        "--n".into(),
        "1".into(),
    ])
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("query dim 2 != vault dim 3"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_l2_i8bin_search_accepts_i32bin_ground_truth() {
    let root = temp_root("i8bin-raw-l2");
    let corpus = root.join("corpus.i8bin");
    let queries = root.join("queries.i8bin");
    let truth = root.join("truth.i32bin");
    let vault = root.join("vault");
    write_i8bin(&corpus, 2, &[&[100, 0], &[9, 1], &[0, 100]]);
    write_i8bin(&queries, 2, &[&[10, 0]]);
    write_i32bin(&truth, 1, &[&[1]]);

    run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&corpus),
        "--regions".into(),
        "1".into(),
        "--distance-metric".into(),
        "raw-l2".into(),
        "--sample".into(),
        "3".into(),
        "--chunk".into(),
        "3".into(),
        "--m-max".into(),
        "2".into(),
        "--ef".into(),
        "4".into(),
        "--region-build-parallelism".into(),
        "1".into(),
    ])
    .expect("build raw-l2 partitioned i8bin vault");

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(vault.join("partitioned-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["distance_metric"], "raw-l2");

    run_search(&[
        "--vault".into(),
        path_arg(&vault),
        "--queries".into(),
        path_arg(&queries),
        "--ground-truth-file".into(),
        path_arg(&truth),
        "--n".into(),
        "1".into(),
        "--k".into(),
        "1".into(),
        "--n-probe".into(),
        "1".into(),
        "--region-beam".into(),
        "8".into(),
        "--ground-truth".into(),
        "1".into(),
        "--recall-floor".into(),
        "1.0".into(),
    ])
    .expect("raw-l2 i32bin ground truth should pass");
    let _ = std::fs::remove_dir_all(root);
}

fn strings(items: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
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

fn write_i32bin(path: &Path, width: u32, rows: &[&[i32]]) {
    let mut bytes = Vec::with_capacity(8 + rows.len() * width as usize * 4);
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    for row in rows {
        assert_eq!(row.len(), width as usize);
        for value in *row {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}
