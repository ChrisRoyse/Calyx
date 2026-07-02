use std::fs;
use std::path::PathBuf;

use calyx_core::SlotId;
use calyx_sextant::RrfProfile;

use super::*;

mod bounded;
mod refused;
mod support;
use support::*;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<ProbeMatrixArgs> {
    match super::parse_probe_matrix(&toks(parts))? {
        Subcommand::ProbeMatrix(args) => Ok(args),
        _ => unreachable!("parse_probe_matrix must return ProbeMatrix"),
    }
}

#[test]
fn parses_probe_matrix_axes() {
    let args = parse(&[
        "corpus",
        "--frontier",
        "type 2 diabetes",
        "--slot",
        "8",
        "--slot",
        "14",
        "--weighted-profile",
        "bridge",
        "--phrasing",
        "clinical",
        "--length",
        "paragraph",
        "--top-k",
        "7",
        "--guard",
        "off",
        "--resident-addr",
        "127.0.0.1:8787",
        "--max-variants",
        "3",
        "--time-budget-ms",
        "5000",
    ])
    .unwrap();

    assert_eq!(args.vault, "corpus");
    assert_eq!(args.frontier, "type 2 diabetes");
    assert_eq!(args.slots, vec![SlotId::new(8), SlotId::new(14)]);
    assert_eq!(args.weighted_profiles, vec![RrfProfile::Bridge]);
    assert_eq!(args.phrasings, vec![ProbePhrasing::Clinical]);
    assert_eq!(args.lengths, vec![ProbeLength::Paragraph]);
    assert_eq!(args.top_k, 7);
    assert_eq!(args.resident_addr, Some("127.0.0.1:8787".parse().unwrap()));
    assert_eq!(args.max_variants, Some(3));
    assert_eq!(args.time_budget_ms, Some(5000));
}

#[test]
fn missing_frontier_fails_closed() {
    let err = parse(&["corpus", "--top-k", "3"]).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--frontier"));
}

#[test]
fn parses_calibrated_guard_tau_for_in_region() {
    let args = parse(&[
        "corpus",
        "--frontier",
        "x",
        "--guard",
        "in-region",
        "--guard-tau",
        "0.72",
    ])
    .unwrap();
    assert_eq!(args.guard, GuardChoice::InRegion);
    assert_eq!(args.guard_tau, Some(0.72));
}

#[test]
fn guard_tau_without_in_region_fails_closed() {
    let err = parse(&["corpus", "--frontier", "x", "--guard-tau", "0.72"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("--guard-tau requires --guard in-region")
    );
}

#[test]
fn out_of_range_guard_tau_fails_closed() {
    for bad in ["0", "1.5", "-0.1", "nan"] {
        let err = parse(&[
            "corpus",
            "--frontier",
            "x",
            "--guard",
            "in-region",
            "--guard-tau",
            bad,
        ])
        .unwrap_err();
        assert_eq!(
            err.code(),
            "CALYX_CLI_USAGE_ERROR",
            "tau {bad} must fail closed"
        );
        assert!(err.message().contains("--guard-tau"));
    }
}

#[test]
fn bad_profile_fails_closed() {
    let err = parse(&["corpus", "--frontier", "x", "--weighted-profile", "unknown"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("unknown --weighted-profile"));
}

#[test]
fn probe_matrix_open_options_use_latest_only_router_readback() {
    let (_home, vault_dir) = seed_home("off-open-options");
    let state = load_vault_panel_state(&vault_dir).unwrap();
    let options = super::probe_read_vault_options(&state.panel, GuardChoice::Off);

    assert!(
        !options.restore_mvcc_rows,
        "probe-matrix is read-only latest-state search and must not restore every MVCC row"
    );
    assert!(
        !options.restore_ledger_hook,
        "probe-matrix must not materialize the full ledger hook before latest-state search"
    );
    assert!(
        options.read_only,
        "probe-matrix opens must fail closed before any vault mutation"
    );
    assert_eq!(
        options.selected_cfs,
        Some(vec![
            calyx_aster::cf::ColumnFamily::Base,
            calyx_aster::cf::ColumnFamily::Anchors,
        ]),
        "probe-matrix provenance readback must enumerate only Base plus grounding Anchors CF"
    );
}

#[test]
fn in_region_probe_matrix_opens_panel_slot_cfs_for_guard_hydration() {
    let (_home, vault_dir) = seed_home("in-region-open-options");
    let state = load_vault_panel_state(&vault_dir).unwrap();
    let options = super::probe_read_vault_options(&state.panel, GuardChoice::InRegion);
    let selected = options.selected_cfs.expect("in-region must select CFs");

    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::Base));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::Anchors));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::slot(SlotId::new(8))));
    assert!(selected.contains(&calyx_aster::cf::ColumnFamily::slot(SlotId::new(14))));
    assert!(
        options.read_only,
        "probe-matrix in-region guard must still use read-only handles"
    );
}

#[test]
fn run_persists_matrix_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy");

    run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "happy".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap();

    let matrix_path = only_matrix(&vault_dir);
    let readback_bytes = fs::read(&matrix_path).unwrap();
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(artifact.schema_version, 7);
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Ok);
    assert!(artifact.run.complete);
    assert_eq!(artifact.run.completed_variant_count, 6);
    assert_eq!(artifact.run.next_variant_index, None);
    assert!(PathBuf::from(&artifact.run.progress_artifact).exists());
    assert!(PathBuf::from(&artifact.run.partial_matrix_artifact).exists());
    assert_eq!(artifact.vault, "happy");
    assert_eq!(artifact.active_slots, vec![SlotId::new(8), SlotId::new(14)]);
    assert_eq!(artifact.diagnostics.query_measurements.len(), 1);
    let query = &artifact.diagnostics.query_measurements[0];
    assert_eq!(query.measure_call_count, 1);
    assert_eq!(query.variant_use_count, 6);
    assert_eq!(query.measured_slot_count, 2);
    let cache = &artifact.diagnostics.search_result_cache;
    assert_eq!(
        (
            cache.entry_count,
            cache.lookup_count,
            cache.hit_count,
            cache.miss_count
        ),
        (1, 6, 5, 1)
    );
    assert_eq!((cache.stored_slot_count, cache.stored_hit_count), (2, 4));
    assert!(cache.last_key_sha256.is_some());
    assert_eq!(artifact.diagnostics.variant_guard_counts.len(), 6);
    assert_eq!(
        artifact.diagnostics.variant_guard_counts[0].search_cache_miss_count,
        1
    );
    assert!(
        artifact.diagnostics.variant_guard_counts[1..]
            .iter()
            .all(|row| row.search_cache_hit_count == 1)
    );
    assert!(artifact.diagnostics.variant_guard_counts.iter().all(|row| {
        row.pre_guard_hit_count.is_none()
            && row.post_guard_hit_count.is_none()
            && row.guard_filtered_hit_count.is_none()
    }));
    assert_eq!(artifact.log.schema_version, 1);
    assert_eq!(artifact.log.records.len(), 6);
    assert!(!artifact.log.productive.is_empty());
    assert!(
        artifact
            .log
            .records
            .iter()
            .all(|record| record.accepted_hit_count == 1)
    );
    assert!(artifact.log.records.iter().any(|record| {
        record.hits.iter().any(|hit| {
            hit.provenance
                .iter()
                .any(|p| p == "metadata:source_id=clinical")
        })
    }));
}

#[test]
fn requested_missing_slot_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("bad-slot");

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "bad-slot".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(123)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Clinical],
            lengths: vec![ProbeLength::Phrase],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    let progress_path = only_progress(&vault_dir);
    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["status"], "failed");
    assert_eq!(progress["phase"], "slot_validation_error");
    assert!(!progress_path.with_file_name("matrix.json").exists());
}
