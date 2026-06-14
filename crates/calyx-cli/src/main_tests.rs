use super::*;
use crate::cli_support::hex_lines;
use crate::dispatch::run;
use calyx_anneal::TripwireRegistry;
use std::path::PathBuf;

#[test]
fn crate_metadata_is_present() {
    assert_eq!(env!("CARGO_PKG_NAME"), "calyx-cli");
}

#[test]
fn hex_lines_match_xxd_plain_chunks() {
    let bytes: Vec<_> = (0u8..=34).collect();

    assert_eq!(
        hex_lines(&bytes),
        vec![
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "202122",
        ]
    );
}

#[test]
fn display_relative_root_is_dot() {
    let root = PathBuf::from("/tmp/calyx-readback");

    assert_eq!(vault_tree::display_relative(&root, &root), ".");
}

#[test]
fn temporal_search_readback_command_executes() {
    run(vec![
        "readback".into(),
        "temporal_search".into(),
        "--explain".into(),
        "--clock-fixed".into(),
        "1000000".into(),
        "--tz-offset".into(),
        "0".into(),
    ])
    .expect("temporal search readback");
}

#[test]
fn dedup_check_readback_rejects_invalid_cosine_arg() {
    let error = run(vec![
        "readback".into(),
        "dedup-check".into(),
        "--vault".into(),
        "missing".into(),
        "--cx-id".into(),
        "00000000000000000000000000000000".into(),
        "--slot".into(),
        "0".into(),
        "--tau".into(),
        "2.0".into(),
        "--near-cos".into(),
        "0.95".into(),
        "--distinct-cos".into(),
        "0.85".into(),
        "--vault-id".into(),
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        "--salt".into(),
        "s".into(),
    ])
    .expect_err("invalid tau");

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--tau"));
}

#[test]
fn tripwire_config_readback_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-tripwire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create tripwire test vault");
    TripwireRegistry::load_from_vault(&root).expect("write default tripwire config");

    run(vec![
        "readback".into(),
        "config".into(),
        "tripwire".into(),
        "--vault".into(),
        root.display().to_string(),
    ])
    .expect("tripwire config readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_deficit_map_fixture_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-deficit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create deficit fixture dir");
    let fixture = root.join("assay.json");
    std::fs::write(
        &fixture,
        r#"{
  "clock_ts": 1785500418,
  "panel": ["01010101010101010101010101010101"],
  "anchors": [{
    "anchor_id": "outcome_positive",
    "entropy_h": 2.0,
    "panel_sufficiency": 0.3,
    "expected_modalities": ["text", "audio"],
    "bits_per_lens": [{
      "lens_id": "01010101010101010101010101010101",
      "bits": 0.3,
      "modality": "text"
    }]
  }]
}"#,
    )
    .expect("write deficit fixture");

    run(vec![
        "anneal".into(),
        "deficit-map".into(),
        "--anchor".into(),
        "outcome_positive".into(),
        "--fixture".into(),
        fixture.display().to_string(),
    ])
    .expect("deficit map readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_propose_preview_fixture_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-propose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create propose fixture dir");
    let deficit = root.join("deficit.json");
    let corpus = root.join("corpus.json");
    std::fs::write(
        &deficit,
        r#"{
  "computed_at": 1785500419,
  "top_gaps": [{
    "anchor_class": "temporal_latency",
    "entropy_h": 2.0,
    "mutual_info_i": 0.4,
    "gap": 1.6
  }],
  "underrepresented_modalities": ["structured"],
  "total_bits_deficit": 1.6
}"#,
    )
    .expect("write propose deficit fixture");
    std::fs::write(
        &corpus,
        r#"[{
  "cx_id": "01010101010101010101010101010101",
  "created_at": 100,
  "modality": "structured",
  "scalars": {"time_lag": 1.0},
  "metadata": {"fixture": "issue419"}
}]"#,
    )
    .expect("write propose corpus fixture");

    run(vec![
        "anneal".into(),
        "propose-preview".into(),
        "--anchor".into(),
        "temporal_latency".into(),
        "--deficit".into(),
        deficit.display().to_string(),
        "--corpus".into(),
        corpus.display().to_string(),
    ])
    .expect("propose preview readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_lens_proposal_log_fixture_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create gate fixture dir");
    let fixture = root.join("gate-log.json");
    std::fs::write(
        &fixture,
        r#"{
  "clock_ts": 1785500420,
  "events": [{
    "seq": 1,
    "candidate_lens_id": "c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8c8",
    "candidate": {
      "candidate_type": "commission",
      "spec": {
        "target_modality": "audio",
        "endpoint": null,
        "model_id": null,
        "description": "fixture candidate"
      }
    },
    "profile_bits": 0.12,
    "panel": ["01010101010101010101010101010101"],
    "correlations": [{
      "lens_id": "01010101010101010101010101010101",
      "corr": 0.45
    }]
  }]
}"#,
    )
    .expect("write gate fixture");

    run(vec![
        "anneal".into(),
        "lens-proposal-log".into(),
        "--fixture".into(),
        fixture.display().to_string(),
        "--last".into(),
        "5".into(),
    ])
    .expect("lens proposal log readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_intelligence_report_fixture_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-j-report-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create j report fixture dir");
    let fixture = root.join("j-report.json");
    std::fs::write(
        &fixture,
        r#"{
  "domain": "fixture",
  "panel_len": 4,
  "metrics": {
    "mutual_info_panel_anchor": 1.5,
    "n_eff": 3.5,
    "panel_sufficiency": 0.8,
    "kernel_recall": 0.7,
    "oracle_accuracy": 0.6,
    "mistake_rate": 0.1,
    "compression_yield": 0.4,
    "coverage": 0.3,
    "dpi_ceiling": 2.0,
    "provisional_count": 0
  }
}"#,
    )
    .expect("write j report fixture");

    run(vec![
        "anneal".into(),
        "intelligence-report".into(),
        "--fixture".into(),
        fixture.display().to_string(),
    ])
    .expect("intelligence report readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_intelligence_report_synthetic_recursion_attempt_fails() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-j-synthetic-recursion-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create j synthetic recursion fixture dir");
    let fixture = root.join("j-report.json");
    std::fs::write(
        &fixture,
        r#"{
  "domain": "fixture",
  "panel_len": 4,
  "metrics": {
    "mutual_info_panel_anchor": 1.5,
    "n_eff": 3.5,
    "panel_sufficiency": 0.8,
    "kernel_recall": 0.7,
    "oracle_accuracy": 0.6,
    "mistake_rate": 0.1,
    "compression_yield": 0.4,
    "coverage": 0.3,
    "dpi_ceiling": 2.0,
    "synthetic_recursion_credit_attempted": true
  }
}"#,
    )
    .expect("write j synthetic recursion fixture");

    let error = run(vec![
        "anneal".into(),
        "intelligence-report".into(),
        "--fixture".into(),
        fixture.display().to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_ANNEAL_J_SYNTHETIC_RECURSION");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_growth_curve_empty_vault_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-growth-curve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create growth curve fixture dir");
    calyx_aster::vault::AsterVault::new_durable(
        &root,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"calyx-anneal-intelligence-report".to_vec(),
        calyx_aster::vault::VaultOptions::default(),
    )
    .expect("create growth vault");

    run(vec![
        "anneal".into(),
        "growth-curve".into(),
        "--vault".into(),
        root.display().to_string(),
        "--last".into(),
        "10".into(),
    ])
    .expect("growth curve readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anneal_propose_lens_run_fixture_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-propose-lens-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create propose-lens fixture dir");
    let fixture = root.join("propose-lens.json");
    std::fs::write(
        &fixture,
        r#"{
  "anchor": "quality",
  "entropy": 1.0,
  "sufficiency": [0.20, 0.80],
  "profile_bits": 0.12,
  "corr": 0.45,
  "clock_ts": 1785500421,
  "panel": ["01010101010101010101010101010101"],
  "substrate": "promote",
  "hot_add": "succeed",
  "corpus_rows": 1
}"#,
    )
    .expect("write propose-lens fixture");

    run(vec![
        "anneal".into(),
        "propose-lens-run".into(),
        "--fixture".into(),
        fixture.display().to_string(),
    ])
    .expect("propose lens run readback");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn kernel_health_readback_command_executes() {
    use calyx_lodestar::{
        FsKernelStore, GroundednessReport, Kernel, RecallReport, write_kernel_artifact,
    };

    let root = std::env::temp_dir().join(format!("calyx-cli-kernel-health-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create kernel health store root");

    let kernel_id = calyx_core::CxId::from_bytes([0xab; 16]);
    let kernel = Kernel {
        kernel_id,
        panel_version: 1,
        anchor_kind: Some("synthetic".to_string()),
        corpus_shard_hash: [0; 32],
        members: vec![kernel_id],
        kernel_graph: vec![kernel_id],
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "test".to_string(),
        warnings: Vec::new(),
    };
    let store = FsKernelStore::new(&root);
    write_kernel_artifact(&kernel, &store).expect("write kernel artifact");

    run(vec![
        "readback".into(),
        "kernel-health".into(),
        "--root".into(),
        root.display().to_string(),
        "--kernel-id".into(),
        kernel_id.to_string(),
    ])
    .expect("kernel health readback");

    let error = run(vec![
        "readback".into(),
        "kernel-health".into(),
        "--root".into(),
        root.display().to_string(),
        "--kernel-id".into(),
        "00000000000000000000000000000000".into(),
    ])
    .expect_err("missing kernel must fail closed");
    assert_eq!(error.code(), "CALYX_KERNEL_NOT_FOUND");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resource_status_refuses_missing_vault_without_creating_it() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-resource-status-missing-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let error = run(vec![
        "resource-status".into(),
        "--vault".into(),
        root.display().to_string(),
    ])
    .expect_err("missing vault must fail closed");

    assert_eq!(error.code(), "CALYX_DISK_PRESSURE");
    assert!(!root.exists(), "status probe must not create vault state");
}

#[test]
fn resource_drill_rejects_zero_memtable_cap() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-resource-drill-zero-cap-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let error = run(vec![
        "resource-drill".into(),
        "--vault".into(),
        root.display().to_string(),
        "--ops".into(),
        "1".into(),
        "--value-bytes".into(),
        "8".into(),
        "--memtable-cap".into(),
        "0".into(),
        "--pin-max-age-ms".into(),
        "1000".into(),
    ])
    .expect_err("zero memtable cap must be rejected");

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--memtable-cap must be positive"));
    let _ = std::fs::remove_dir_all(&root);
}
