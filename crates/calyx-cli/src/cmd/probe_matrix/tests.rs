use std::fs;
use std::path::{Path, PathBuf};

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
        "--stale-ok",
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
    assert!(args.stale_ok);
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
            stale_ok: false,
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

/// Reads back the persisted matrix.json for `vault_dir` and returns the parsed
/// artifact (physical source of truth on disk, not the in-memory value).
fn read_persisted_matrix(vault_dir: &Path) -> ProbeMatrixArtifact {
    let matrix_path = only_matrix(vault_dir);
    let bytes = fs::read(&matrix_path).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Extracts the `rank=N` value embedded in a probe hit's provenance strings.
fn hit_rank(hit: &calyx_lodestar::ProbeHit) -> usize {
    hit.provenance
        .iter()
        .find_map(|line| line.strip_prefix("rank="))
        .expect("probe hit provenance carries rank")
        .parse()
        .expect("rank is a usize")
}

fn corpus_args(vault: &str, top_k: usize) -> ProbeMatrixArgs {
    ProbeMatrixArgs {
        vault: vault.to_string(),
        frontier: "alpha".to_string(),
        slots: vec![SlotId::new(8), SlotId::new(14)],
        weighted_profiles: vec![RrfProfile::Bridge],
        phrasings: vec![ProbePhrasing::Terse, ProbePhrasing::Clinical],
        lengths: vec![ProbeLength::Entity, ProbeLength::Phrase],
        top_k,
        guard: GuardChoice::Off,
        guard_tau: None,
        stale_ok: false,
        out: None,
        resident_addr: None,
        max_variants: None,
        time_budget_ms: None,
        search_miss_budget_ms: None,
        search_hit_budget_ms: None,
    }
}

/// #1846: with a corpus much larger than `top_k`, every persisted record must
/// carry at most `top_k` hits (pre-fix the engine leaked its internal 64/48
/// retrieval window), the hits must be deterministically ordered (rank
/// 1..=len, descending score, cx_id tie-break), and provenance must survive.
#[test]
fn probe_matrix_enforces_final_top_k_across_all_fusion_variants() {
    const TOP_K: usize = 5;
    const DOC_COUNT: usize = 50;
    let (home, vault_dir) = seed_home_with_corpus("topk-contract", DOC_COUNT);

    run_probe_matrix_with_home(&home, corpus_args("topk-contract", TOP_K)).unwrap();

    let artifact = read_persisted_matrix(&vault_dir);
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Ok);
    assert!(artifact.run.complete);
    // 2 phrasings x 2 lengths x (KernelFirst + Rrf + WeightedRrf + 2xSingleLens
    // + Pipeline) = 4 x 6 = 24 variants — a real multi-lens matrix.
    assert_eq!(artifact.log.records.len(), 24);

    let mut saw_full_topk = false;
    for record in &artifact.log.records {
        assert!(
            record.hits.len() <= TOP_K,
            "variant {} persisted {} hits but top_k={TOP_K}",
            record.variant.id,
            record.hits.len(),
        );
        assert!(
            record.accepted_hit_count <= TOP_K,
            "variant {} accepted {} hits over top_k={TOP_K}",
            record.variant.id,
            record.accepted_hit_count,
        );
        if record.hits.len() == TOP_K {
            saw_full_topk = true;
        }
        // Deterministic ordering: rank 1..=len, non-increasing score.
        for (index, hit) in record.hits.iter().enumerate() {
            assert_eq!(hit_rank(hit), index + 1, "ranks must be dense 1..=len");
            if index > 0 {
                let prev = &record.hits[index - 1];
                assert!(
                    prev.score >= hit.score,
                    "hits must be sorted by descending score: {} then {}",
                    prev.score,
                    hit.score,
                );
                // Tie-break: equal scores ordered by ascending cx_id.
                if prev.score == hit.score {
                    assert!(
                        prev.cx_id < hit.cx_id,
                        "equal-score ties must order by ascending cx_id",
                    );
                }
            }
            // Provenance survives truncation (grounded corpus).
            assert!(
                hit.grounded,
                "every corpus doc is anchored; hit {} must stay grounded",
                hit.cx_id,
            );
            assert!(
                hit.provenance
                    .iter()
                    .any(|line| line.starts_with("ledger_seq=")),
                "hit provenance must retain ledger_seq",
            );
        }
    }
    assert!(
        saw_full_topk,
        "corpus of {DOC_COUNT} >> top_k={TOP_K} must fill at least one variant to the top-k bound",
    );

    // Issue's explicit acceptance: total accepted hits bounded by records*top_k.
    let total_accepted: usize = artifact
        .log
        .records
        .iter()
        .map(|record| record.accepted_hit_count)
        .sum();
    assert!(
        total_accepted <= artifact.log.records.len() * TOP_K,
        "total accepted {total_accepted} must be <= {} (records x top_k)",
        artifact.log.records.len() * TOP_K,
    );

    let _ = fs::remove_dir_all(&home);
}

/// #1846 cache parity: a cache-cold run and a cache-warm rerun over the same
/// vault must produce byte-identical bounded hit payloads per variant.
#[test]
fn probe_matrix_topk_output_is_identical_cold_and_warm() {
    const TOP_K: usize = 5;
    let (home, vault_dir) = seed_home_with_corpus("topk-cache-parity", 40);

    let first_path = vault_dir.join("first.json");
    let second_path = vault_dir.join("second.json");

    let mut cold = corpus_args("topk-cache-parity", TOP_K);
    cold.out = Some(first_path.clone());
    run_probe_matrix_with_home(&home, cold).unwrap();

    let mut warm = corpus_args("topk-cache-parity", TOP_K);
    warm.out = Some(second_path.clone());
    run_probe_matrix_with_home(&home, warm).unwrap();

    let first: ProbeMatrixArtifact =
        serde_json::from_slice(&fs::read(&first_path).unwrap()).unwrap();
    let second: ProbeMatrixArtifact =
        serde_json::from_slice(&fs::read(&second_path).unwrap()).unwrap();

    assert_eq!(first.log.records.len(), second.log.records.len());
    for (a, b) in first.log.records.iter().zip(second.log.records.iter()) {
        assert!(a.hits.len() <= TOP_K && b.hits.len() <= TOP_K);
        // Byte-compare the record hit payloads across the two runs.
        assert_eq!(
            serde_json::to_vec(&a.hits).unwrap(),
            serde_json::to_vec(&b.hits).unwrap(),
            "variant {} hit payload diverged between cold and warm runs",
            a.variant.id,
        );
    }

    let _ = fs::remove_dir_all(&home);
}

/// #1846 edge case: top_k=1 must yield at most one hit per variant.
#[test]
fn probe_matrix_top_k_one_returns_single_hit() {
    let (home, vault_dir) = seed_home_with_corpus("topk-one", 30);
    run_probe_matrix_with_home(&home, corpus_args("topk-one", 1)).unwrap();

    let artifact = read_persisted_matrix(&vault_dir);
    for record in &artifact.log.records {
        assert!(
            record.hits.len() <= 1,
            "variant {} returned {} hits at top_k=1",
            record.variant.id,
            record.hits.len(),
        );
    }
    let _ = fs::remove_dir_all(&home);
}

/// #1846 edge case: top_k larger than the corpus returns at most the corpus
/// size and never exceeds top_k.
#[test]
fn probe_matrix_top_k_larger_than_corpus_is_bounded_by_corpus() {
    const DOC_COUNT: usize = 12;
    const TOP_K: usize = 1000;
    let (home, vault_dir) = seed_home_with_corpus("topk-huge", DOC_COUNT);
    run_probe_matrix_with_home(&home, corpus_args("topk-huge", TOP_K)).unwrap();

    let artifact = read_persisted_matrix(&vault_dir);
    for record in &artifact.log.records {
        assert!(
            record.hits.len() <= DOC_COUNT,
            "variant {} returned {} hits over corpus size {DOC_COUNT}",
            record.variant.id,
            record.hits.len(),
        );
        assert!(record.hits.len() <= TOP_K);
    }
    let _ = fs::remove_dir_all(&home);
}

/// #1846 edge case: an empty corpus fails closed (no grounded hits) rather than
/// persisting a completed, over-budget record.
#[test]
fn probe_matrix_empty_corpus_fails_closed() {
    let (home, _vault_dir) = seed_home_with_corpus("topk-empty", 0);
    let err = run_probe_matrix_with_home(&home, corpus_args("topk-empty", 5)).unwrap_err();
    // Fails closed with a structured code (no silent completion).
    assert!(
        !err.code().is_empty() && err.code().starts_with("CALYX_"),
        "empty corpus must fail closed with a CALYX_* code, got {}",
        err.code(),
    );
    let _ = fs::remove_dir_all(&home);
}

/// #1846 defense in depth: if the engine ever regresses and returns more than
/// the declared top_k, probe-matrix's own validation must fail closed instead
/// of persisting the over-budget record.
#[test]
fn validate_response_rejects_over_budget_hits() {
    use calyx_lodestar::{
        ProbeFusionMode, ProbeHit, ProbeLength, ProbeLensEmphasis, ProbePhrasing, ProbeResponse,
        ProbeVariant,
    };

    let variant = ProbeVariant {
        id: 3,
        frontier: "alpha".to_string(),
        query_text: "alpha".to_string(),
        fusion: ProbeFusionMode::Rrf,
        phrasing: ProbePhrasing::Terse,
        length: ProbeLength::Entity,
        lens_emphasis: ProbeLensEmphasis::Balanced,
        top_k: 10,
    };
    let hits = (0..64u8)
        .map(|seed| ProbeHit {
            cx_id: calyx_core::CxId::from_bytes([seed; 16]),
            score: 1.0 - f32::from(seed) / 100.0,
            grounded: true,
            provenance: vec![format!("rank={}", seed as usize + 1)],
        })
        .collect();
    let response = ProbeResponse {
        hits,
        refusals: Vec::new(),
    };

    let err = super::validate_response(&variant, &response).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_RUNTIME_ERROR");
    assert!(
        err.message().contains("top-k contract violated")
            && err.message().contains("top_k=10")
            && err.message().contains("64 hits"),
        "message must name the breach: {}",
        err.message(),
    );

    // Within budget passes.
    let ok_variant = ProbeVariant {
        top_k: 64,
        ..variant
    };
    assert!(super::validate_response(&ok_variant, &response).is_ok());
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
            stale_ok: false,
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
