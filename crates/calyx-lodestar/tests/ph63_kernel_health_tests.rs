//! PH63 — engine-level `kernel_health(kernel_id)` aggregate (issue #644).
//!
//! Health must be assembled by reading the persisted kernel artifact; missing,
//! stale, or corrupt artifacts fail closed with structured `CALYX_*` codes.

use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{
    FsKernelStore, GroundednessReport, Kernel, KernelArtifactStore, KernelParams, KernelTrust,
    LodestarError, RecallPassMode, RecallReport, RecallTestParams, build_kernel_pipeline,
    kernel_health, read_kernel_artifact, write_kernel_artifact,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph63-kernel-health"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH63_KERNEL_HEALTH_READBACK={}", path.display());
}

fn store(case: &str) -> FsKernelStore {
    let root = std::env::temp_dir()
        .join(format!("calyx-ph63-store-{}", std::process::id()))
        .join(case);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create store root");
    FsKernelStore::new(root)
}

/// Hand-known kernel: 3 members, 2 kernel-graph nodes, 1 unanchored member
/// (grounded fraction 2/3), recall 0.9 over 10 queries against a 0.95 gate.
fn known_kernel() -> Kernel {
    Kernel {
        kernel_id: cx(42),
        panel_version: 7,
        anchor_kind: Some("synthetic_outcome".to_string()),
        corpus_shard_hash: [7; 32],
        members: vec![cx(1), cx(2), cx(3)],
        kernel_graph: vec![cx(1), cx(2)],
        groundedness: GroundednessReport {
            reached_anchor: 2.0 / 3.0,
            unanchored_members: vec![cx(3)],
        },
        recall: RecallReport {
            kernel_only: 0.9,
            full: 1.0,
            ratio: 0.9,
            approx_factor: 2.0,
            tau_star_estimate: 4,
            tau_star_exact: false,
            recall_test_params: Some(RecallTestParams::default()),
            corpus_name: Some("synthetic".to_string()),
            n_queries_tested: 10,
            held_out: vec![cx(9)],
            warning: None,
        },
        built_at_millis: 12345,
        estimator_provenance: "ph32::Tournament2Approx; trust=anchored".to_string(),
        warnings: Vec::new(),
    }
}

#[test]
fn health_reads_back_persisted_kernel_byte_for_byte() {
    let store = store("known-io");
    let kernel = known_kernel();
    write_kernel_artifact(&kernel, &store).expect("write artifact");

    // FSV: independent read of the source of truth (the persisted file),
    // not the function's own return value.
    let persisted_path = store.kernel_file_path(kernel.kernel_id);
    let persisted_bytes = fs::read(&persisted_path).expect("persisted kernel.json exists");
    let persisted: serde_json::Value =
        serde_json::from_slice(&persisted_bytes).expect("persisted json");
    assert_eq!(persisted["format_version"], 1);
    assert_eq!(persisted["kernel"]["members"].as_array().unwrap().len(), 3);

    let health = kernel_health(kernel.kernel_id, &store).expect("health");
    assert_eq!(health.kernel_id, kernel.kernel_id);
    assert_eq!(health.size, 3);
    assert_eq!(health.kernel_graph_size, 2);
    assert_eq!(health.grounded_fraction, 2.0 / 3.0);
    assert_eq!(health.unanchored_count, 1);
    assert_eq!(health.approx_factor, 2.0);
    assert_eq!(health.tau_star_estimate, 4);
    assert!(!health.tau_star_exact);
    assert_eq!(health.built_at_millis, 12345);
    assert_eq!(health.panel_version, 7);
    assert_eq!(health.anchor_kind.as_deref(), Some("synthetic_outcome"));
    assert_eq!(health.corpus_shard_hash, "07".repeat(32));
    assert_eq!(health.trust, KernelTrust::Anchored);
    assert_eq!(health.recall.raw, 0.9);
    assert_eq!(health.recall.ratio, 0.9);
    assert_eq!(health.recall.min_recall_ratio, 0.95);
    assert_eq!(health.recall.n_queries_tested, 10);
    assert_eq!(health.recall.pass_mode, RecallPassMode::BelowGate);

    // Health values must equal the persisted bytes, field for field.
    assert_eq!(
        json!(health.size),
        json!(persisted["kernel"]["members"].as_array().unwrap().len())
    );
    let persisted_grounded = persisted["kernel"]["groundedness"]["reached_anchor"]
        .as_f64()
        .expect("persisted reached_anchor") as f32;
    assert_eq!(health.grounded_fraction, persisted_grounded);
    let persisted_raw = persisted["kernel"]["recall"]["kernel_only"]
        .as_f64()
        .expect("persisted kernel_only") as f32;
    assert_eq!(health.recall.raw, persisted_raw);

    write_readback(
        "known-io",
        "kernel-health-known-io.json",
        json!({
            "persisted_path": persisted_path.display().to_string(),
            "persisted": persisted,
            "health": health,
        }),
    );
}

#[test]
fn unknown_kernel_id_fails_closed() {
    let store = store("unknown-id");
    let missing = cx(250);

    let error = kernel_health(missing, &store).expect_err("missing kernel must fail");
    assert_eq!(error.code(), "CALYX_KERNEL_NOT_FOUND");
    assert!(matches!(
        error,
        LodestarError::KernelNotFound { kernel_id } if kernel_id == missing
    ));

    write_readback(
        "unknown-id",
        "kernel-health-unknown-id.json",
        json!({
            "requested": missing.to_string(),
            "artifact_exists": store.kernel_file_path(missing).exists(),
            "error": error.to_string(),
        }),
    );
}

#[test]
fn stale_artifact_fails_closed() {
    let store = store("stale");
    let kernel = known_kernel();
    write_kernel_artifact(&kernel, &store).expect("write artifact");

    // Plant kernel 42's bytes under kernel 43's id: a stale/mismatched artifact.
    let bytes = store
        .read_kernel_bytes(kernel.kernel_id)
        .expect("read")
        .expect("artifact present");
    let other = cx(43);
    store
        .write_kernel_bytes(other, &bytes)
        .expect("plant stale artifact");

    let error = kernel_health(other, &store).expect_err("stale artifact must fail");
    assert_eq!(error.code(), "CALYX_KERNEL_ARTIFACT_CODEC");
    assert!(error.to_string().contains("stale"));

    write_readback(
        "stale",
        "kernel-health-stale.json",
        json!({
            "requested": other.to_string(),
            "stored_kernel_id": kernel.kernel_id.to_string(),
            "error": error.to_string(),
        }),
    );
}

#[test]
fn corrupt_artifact_fails_closed() {
    let store = store("corrupt");
    let kernel_id = cx(44);
    store
        .write_kernel_bytes(kernel_id, b"{not json")
        .expect("plant corrupt artifact");

    let error = kernel_health(kernel_id, &store).expect_err("corrupt artifact must fail");
    assert_eq!(error.code(), "CALYX_KERNEL_ARTIFACT_CODEC");

    write_readback(
        "corrupt",
        "kernel-health-corrupt.json",
        json!({
            "requested": kernel_id.to_string(),
            "error": error.to_string(),
        }),
    );
}

#[test]
fn ungrounded_kernel_surfaces_provisional_trust() {
    let store = store("provisional");
    let mut kernel = known_kernel();
    kernel.kernel_id = cx(45);
    kernel.groundedness = GroundednessReport {
        reached_anchor: 0.0,
        unanchored_members: kernel.members.clone(),
    };
    kernel.warnings =
        vec!["CALYX_KERNEL_UNGROUNDED: all kernel members are provisional".to_string()];
    kernel.estimator_provenance = "ph32::Tournament2Approx; trust=provisional".to_string();
    write_kernel_artifact(&kernel, &store).expect("write artifact");

    let health = kernel_health(kernel.kernel_id, &store).expect("health");
    assert_eq!(health.trust, KernelTrust::Provisional);
    assert_eq!(health.grounded_fraction, 0.0);
    assert_eq!(health.unanchored_count, 3);
    assert!(
        health
            .warnings
            .iter()
            .any(|warning| warning.starts_with("CALYX_KERNEL_UNGROUNDED"))
    );

    write_readback(
        "provisional",
        "kernel-health-provisional.json",
        json!({ "health": health }),
    );
}

#[test]
fn pass_mode_reflects_persisted_recall_state() {
    let store = store("pass-mode");

    let mut untested = known_kernel();
    untested.kernel_id = cx(50);
    untested.recall = RecallReport::default();
    write_kernel_artifact(&untested, &store).expect("write untested");

    let mut passed = known_kernel();
    passed.kernel_id = cx(51);
    passed.recall.kernel_only = 0.97;
    passed.recall.ratio = 0.97;
    write_kernel_artifact(&passed, &store).expect("write passed");

    let untested_health = kernel_health(untested.kernel_id, &store).expect("untested health");
    let passed_health = kernel_health(passed.kernel_id, &store).expect("passed health");

    assert_eq!(untested_health.recall.pass_mode, RecallPassMode::Untested);
    assert_eq!(untested_health.recall.n_queries_tested, 0);
    assert_eq!(passed_health.recall.pass_mode, RecallPassMode::Passed);
    assert_eq!(passed_health.recall.ratio, 0.97);

    write_readback(
        "pass-mode",
        "kernel-health-pass-mode.json",
        json!({
            "untested": untested_health,
            "passed": passed_health,
        }),
    );
}

#[test]
fn rebuild_overwrites_artifact_and_health_follows() {
    let store = store("rebuild");
    let kernel = known_kernel();
    write_kernel_artifact(&kernel, &store).expect("write v1");
    let before = kernel_health(kernel.kernel_id, &store).expect("health before");
    assert_eq!(before.recall.pass_mode, RecallPassMode::BelowGate);

    // Rebuild: same kernel, recall re-test now passes the gate.
    let mut rebuilt = kernel.clone();
    rebuilt.recall.kernel_only = 0.99;
    rebuilt.recall.ratio = 0.99;
    rebuilt.recall.n_queries_tested = 25;
    rebuilt.built_at_millis = 67890;
    write_kernel_artifact(&rebuilt, &store).expect("write v2");

    let after = kernel_health(kernel.kernel_id, &store).expect("health after");
    assert_eq!(after.recall.pass_mode, RecallPassMode::Passed);
    assert_eq!(after.recall.ratio, 0.99);
    assert_eq!(after.recall.n_queries_tested, 25);
    assert_eq!(after.built_at_millis, 67890);

    write_readback(
        "rebuild",
        "kernel-health-rebuild.json",
        json!({ "before": before, "after": after }),
    );
}

#[test]
fn pipeline_kernel_round_trips_through_artifact() {
    let store = store("pipeline");
    let mut builder = AssocGraph::builder();
    for seed in [1, 2, 3] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(1), cx(2), 1.0)
        .unwrap()
        .add_edge(cx(2), cx(3), 1.0)
        .unwrap()
        .add_edge(cx(3), cx(1), 1.0)
        .unwrap();
    let graph = builder.build();

    let kernel =
        build_kernel_pipeline(&graph, &[cx(1)], &KernelParams::default()).expect("pipeline");
    write_kernel_artifact(&kernel, &store).expect("write artifact");

    let loaded = read_kernel_artifact(kernel.kernel_id, &store).expect("read artifact");
    assert_eq!(loaded, kernel);

    let health = kernel_health(kernel.kernel_id, &store).expect("health");
    assert_eq!(health.size, kernel.members.len());
    assert_eq!(health.kernel_graph_size, kernel.kernel_graph.len());
    assert_eq!(health.grounded_fraction, kernel.groundedness.reached_anchor);
    assert_eq!(
        health.unanchored_count,
        kernel.groundedness.unanchored_members.len()
    );

    write_readback(
        "pipeline",
        "kernel-health-pipeline.json",
        json!({
            "kernel_id": kernel.kernel_id.to_string(),
            "members": kernel.members.len(),
            "health": health,
        }),
    );
}
