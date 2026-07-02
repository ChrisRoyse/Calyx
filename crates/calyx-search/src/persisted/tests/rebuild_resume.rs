//! Issue #1089: an externally killed post-ingest index rebuild must leave a
//! durable rebuild-required marker, fail searches closed with the marker
//! context, and resume from staged slot artifacts instead of rebuilding
//! everything from scratch.

use calyx_core::{SlotId, SlotVector, SparseEntry, VaultId};
use serde_json::json;
use ulid::Ulid;

use super::*;

#[test]
fn killed_rebuild_leaves_marker_and_resumes_from_staged_artifacts() {
    let root = scratch("rebuild-resume");
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x47; 16]));
    let salt = b"rebuild-resume-staged".to_vec();
    let vault = AsterVault::new_durable(&root, vault_id, salt, VaultOptions::default())
        .expect("open durable vault");
    let mut first = constellation(cx(71), vec![1.0, 0.0]);
    first.vault_id = vault_id;
    first.slots.insert(SlotId::new(1), sparse(16, &[(3, 1.0)]));
    first
        .slots
        .insert(SlotId::new(2), multi(2, &[&[1.0, 0.0], &[0.5, 0.5]]));
    let mut second = constellation(cx(72), vec![0.0, 1.0]);
    second.vault_id = vault_id;
    second.slots.insert(SlotId::new(1), sparse(16, &[(7, 2.0)]));
    second
        .slots
        .insert(SlotId::new(2), multi(2, &[&[0.0, 1.0]]));
    let ids = vault
        .put_batch(vec![first, second])
        .expect("write durable constellations");
    let manifest_path = root.join("idx/search/manifest.json");
    let marker_path = rebuild_required_marker_path(&root);

    // Simulate the external 900s-timeout kill at the exact incident point:
    // every slot and the filter are staged, the manifest publish never runs.
    let mut first_run_phases = Vec::new();
    let kill = rebuild_for_vault_with_fallible_progress(&root, &vault, |event| {
        first_run_phases.push(event.phase.to_string());
        if event.phase == "filter_ok" {
            return Err(stale("injected kill after filter_ok"));
        }
        Ok(())
    })
    .expect_err("injected kill must abort the rebuild");

    let marker_after_kill = read_rebuild_required_marker(&root)
        .expect("read marker")
        .expect("marker must survive the killed rebuild");
    let staged_after_kill = staged_artifact_names(&root);
    let open_error = PersistedSearchIndexes::open(&root)
        .expect_err("missing manifest must fail closed after the kill");
    assert!(kill.message().contains("injected kill after filter_ok"));
    assert!(!manifest_path.exists());
    assert!(marker_path.is_file());
    assert_eq!(marker_after_kill.source, "search_index_rebuild");
    assert_eq!(
        marker_after_kill.required_base_seq,
        Some(vault.latest_seq()),
        "marker must record the exact durable seq the rebuild had to cover"
    );
    assert!(
        staged_after_kill.len() >= 4,
        "expected staged records for 3 slots plus the filter, found {staged_after_kill:?}"
    );
    assert!(
        open_error
            .message()
            .contains("rebuild-required marker present"),
        "fail-closed error must carry the marker context: {}",
        open_error.message()
    );
    assert!(open_error.message().contains("search_index_rebuild"));

    // Resume: every staged slot revalidates at the same pinned seq and is
    // reused; only the manifest publish and prune remain.
    let mut resume_phases = Vec::new();
    rebuild_for_vault_with_progress(&root, &vault, |event| {
        resume_phases.push(event.phase.to_string());
    })
    .expect("resumed rebuild");

    let indexes = PersistedSearchIndexes::open(&root).expect("open indexes after resume");
    let dense_hits = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0]), 1)
        .expect("dense search after resume");
    let staged_after_resume = staged_artifact_names(&root);
    assert_eq!(
        resume_phases
            .iter()
            .filter(|phase| phase.as_str() == "slot_reuse_ok")
            .count(),
        3,
        "all three staged slots must be reused: {resume_phases:?}"
    );
    assert!(resume_phases.contains(&"rebuild_marker_preserved".to_string()));
    assert!(resume_phases.contains(&"filter_reuse_ok".to_string()));
    assert!(resume_phases.contains(&"manifest_write_ok".to_string()));
    assert!(resume_phases.contains(&"rebuild_marker_cleared".to_string()));
    assert!(
        !resume_phases.contains(&"slot_index_write_start".to_string()),
        "no slot may be rebuilt from scratch on resume: {resume_phases:?}"
    );
    assert!(!resume_phases.contains(&"filter_start".to_string()));
    assert!(manifest_path.is_file());
    assert_eq!(indexes.base_seq(), vault.latest_seq());
    assert_eq!(dense_hits[0].cx_id, ids[0]);
    assert!(
        !marker_path.exists(),
        "marker must be cleared after the manifest is durably republished"
    );
    assert!(
        staged_after_resume.is_empty(),
        "staged records must be pruned after publish: {staged_after_resume:?}"
    );
    println!(
        "REBUILD_RESUME_FSV {}",
        json!({
            "source_of_truth": "idx/search physical files (manifest, rebuild-required marker, staged records) plus durable Aster CF rows",
            "kill_error": kill.message(),
            "marker_after_kill": {
                "path": marker_path,
                "source": marker_after_kill.source,
                "required_base_seq": marker_after_kill.required_base_seq,
            },
            "staged_after_kill": staged_after_kill,
            "open_error_after_kill": open_error.message(),
            "resume_phases": resume_phases,
            "manifest_base_seq_after_resume": indexes.base_seq(),
            "vault_latest_seq": vault.latest_seq(),
            "dense_hit": dense_hits[0].cx_id.to_string(),
            "staged_after_resume": staged_after_resume,
            "marker_exists_after_resume": marker_path.exists(),
        })
    );
    cleanup(root);
}

#[test]
fn marker_clear_refuses_manifest_behind_required_seq() {
    let root = scratch("marker-clear-refusal");
    let mut marker =
        RebuildRequiredMarker::new("batch_ingest", "test intent").expect("build marker");
    marker.required_base_seq = Some(100);
    write_rebuild_required_marker(&root, &marker).expect("write marker");

    let refusal = clear_rebuild_required_marker(&root, 99)
        .expect_err("clearing below the required seq must fail");
    let still_there = read_rebuild_required_marker(&root).expect("read after refusal");
    let cleared = clear_rebuild_required_marker(&root, 100).expect("clear at required seq");
    let gone = read_rebuild_required_marker(&root).expect("read after clear");

    assert!(refusal.message().contains("requires base seq 100"));
    assert_eq!(still_there, Some(marker));
    assert_eq!(cleared, MarkerClearOutcome::Cleared);
    assert_eq!(gone, None);
    assert!(!rebuild_required_marker_path(&root).exists());
    println!(
        "MARKER_CLEAR_REFUSAL_FSV {}",
        json!({
            "source_of_truth": "idx/search/rebuild-required.json physical readbacks around clear attempts",
            "refusal_message": refusal.message(),
            "marker_survived_refusal": true,
            "marker_file_after_clear": rebuild_required_marker_path(&root).exists(),
        })
    );
    cleanup(root);
}

#[test]
fn foreign_marker_is_not_cleared_by_owned_clear() {
    let root = scratch("marker-foreign-owner");
    let mut marker =
        RebuildRequiredMarker::new("batch_ingest", "foreign intent").expect("build marker");
    marker.process_id = std::process::id().wrapping_add(1);
    write_rebuild_required_marker(&root, &marker).expect("write foreign marker");

    let outcome = clear_rebuild_required_marker_if_owned(&root).expect("owned clear");

    let survivor = read_rebuild_required_marker(&root).expect("read after owned clear");
    assert_eq!(outcome, MarkerClearOutcome::Absent);
    assert_eq!(survivor, Some(marker));
    println!(
        "MARKER_FOREIGN_OWNER_FSV {}",
        json!({
            "source_of_truth": "idx/search/rebuild-required.json physical readback after owned-clear attempt",
            "outcome": "left in place",
            "marker_process_id": survivor.map(|marker| marker.process_id),
            "this_process_id": std::process::id(),
        })
    );
    cleanup(root);
}

#[test]
fn corrupt_marker_fails_closed_on_read() {
    let root = scratch("marker-corrupt");
    let path = rebuild_required_marker_path(&root);
    fs::create_dir_all(path.parent().expect("marker parent")).expect("create idx/search");
    fs::write(&path, b"{ not json").expect("write corrupt marker");

    let error = read_rebuild_required_marker(&root).expect_err("corrupt marker must fail closed");

    assert_eq!(error.code(), "CALYX_STALE_DERIVED");
    assert!(error.message().contains("not valid JSON"));
    println!(
        "MARKER_CORRUPT_FSV {}",
        json!({
            "source_of_truth": "corrupt idx/search/rebuild-required.json bytes on disk",
            "error_code": error.code(),
            "error_message": error.message(),
        })
    );
    cleanup(root);
}

fn staged_artifact_names(root: &std::path::Path) -> Vec<String> {
    let dir = root.join("idx/search");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".staged.json"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn sparse(dim: u32, entries: &[(u32, f32)]) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}

fn multi(token_dim: u32, rows: &[&[f32]]) -> SlotVector {
    SlotVector::Multi {
        token_dim,
        tokens: rows.iter().map(|row| row.to_vec()).collect(),
    }
}

fn cleanup(root: std::path::PathBuf) {
    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}
