//! Issue #1089: batch ingest must stake the durable rebuild-required marker
//! before its first mutation, record the exact committed seq before the
//! post-commit index rebuild, and only clear it once the rebuild republishes
//! the manifest — so an external kill can never leave a silently stale search
//! index.

use calyx_search::{
    PersistedSearchIndexes, RebuildRequiredMarker, read_rebuild_required_marker,
    rebuild_required_marker_path, write_rebuild_required_marker,
};

use super::*;

#[test]
fn batch_ingest_stakes_then_clears_rebuild_marker_and_publishes_fresh_manifest() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1089-marker-happy");
    let jsonl = resolved.path.join("marker-happy.jsonl");
    fs::write(
        &jsonl,
        "{\"text\":\"issue1089 alpha row\"}\n{\"text\":\"issue1089 beta row\"}\n",
    )
    .unwrap();
    let marker_path = rebuild_required_marker_path(&resolved.path);
    assert!(!marker_path.exists(), "clean vault must carry no marker");

    let summary = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let latest_seq = vault.snapshot();
    let indexes = PersistedSearchIndexes::open(&resolved.path).unwrap();
    let marker_after = read_rebuild_required_marker(&resolved.path).unwrap();
    assert_eq!(summary.new_count, 2);
    assert_eq!(
        marker_after, None,
        "completed ingest+rebuild must leave no marker"
    );
    assert!(!marker_path.exists());
    assert_eq!(
        indexes.base_seq(),
        latest_seq,
        "published manifest must cover the final durable seq"
    );
    println!(
        "ISSUE1089_MARKER_HAPPY_FSV {}",
        json!({
            "source_of_truth": "idx/search/rebuild-required.json absence, idx/search/manifest.json base_seq, and durable vault seq readback",
            "marker_path": marker_path,
            "marker_exists_after_ingest": marker_path.exists(),
            "manifest_base_seq": indexes.base_seq(),
            "vault_latest_seq": latest_seq,
            "new_count": summary.new_count,
        })
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn replay_only_batch_leaves_foreign_marker_in_place() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1089-marker-foreign");
    let jsonl = resolved.path.join("marker-foreign.jsonl");
    fs::write(&jsonl, "{\"text\":\"issue1089 replay row\"}\n").unwrap();
    ingest_batch_streaming(&resolved, &jsonl).unwrap();
    let mut foreign = RebuildRequiredMarker::new(
        "batch_ingest",
        "foreign interrupted run whose staleness record must survive",
    )
    .unwrap();
    foreign.process_id = std::process::id().wrapping_add(1);
    foreign.required_base_seq = Some(u64::MAX);
    write_rebuild_required_marker(&resolved.path, &foreign).unwrap();

    // Identical batch: replay-only (new_count == 0), the rebuild is skipped,
    // and only a marker owned by THIS process could be released.
    let replay = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let survivor = read_rebuild_required_marker(&resolved.path).unwrap();
    assert_eq!(replay.new_count, 0);
    assert_eq!(replay.already_count, 1);
    assert_eq!(
        survivor,
        Some(foreign),
        "replay-only batch must never mask a foreign interrupted-run marker"
    );
    println!(
        "ISSUE1089_MARKER_FOREIGN_FSV {}",
        json!({
            "source_of_truth": "idx/search/rebuild-required.json physical readback after replay-only batch",
            "replay_new_count": replay.new_count,
            "replay_already_count": replay.already_count,
            "foreign_marker_survived": survivor.is_some(),
        })
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn replay_only_batch_keeps_fresh_search_available() {
    // Issue #1100 regression: a replay-only idempotent batch appends
    // content-neutral ledger rows (durable seq advances) and skips the
    // rebuild; Fresh search must remain available because the derived-content
    // watermark does not move.
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1100-replay-fresh");
    let jsonl = resolved.path.join("replay-fresh.jsonl");
    fs::write(
        &jsonl,
        "{\"text\":\"issue1100 gamma row\"}\n{\"text\":\"issue1100 delta row\"}\n",
    )
    .unwrap();
    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let replay = ingest_batch_streaming(&resolved, &jsonl).unwrap();
    assert_eq!(replay.new_count, 0);
    assert_eq!(replay.already_count, 2);

    let vault = open_vault(&resolved).unwrap();
    let latest_seq = vault.snapshot();
    let derived_content_seq = vault.derived_content_seq();
    let indexes = PersistedSearchIndexes::open(&resolved.path).unwrap();
    assert!(
        latest_seq > indexes.base_seq(),
        "replay-only batch must reproduce the #1100 trigger: durable seq {latest_seq} past manifest base seq {}",
        indexes.base_seq()
    );
    assert!(
        derived_content_seq <= indexes.base_seq(),
        "replay-only batch must not advance the derived-content watermark: watermark {derived_content_seq}, manifest base seq {}",
        indexes.base_seq()
    );
    indexes
        .ensure_fresh_at_snapshot(latest_seq, derived_content_seq)
        .expect("Fresh search must stay available after a replay-only batch");
    println!(
        "ISSUE1100_REPLAY_FRESH_FSV {}",
        json!({
            "source_of_truth": "durable vault seq + derived-content watermark readback vs idx/search/manifest.json base_seq",
            "vault_latest_seq": latest_seq,
            "derived_content_seq": derived_content_seq,
            "manifest_base_seq": indexes.base_seq(),
            "replay_new_count": replay.new_count,
            "replay_already_count": replay.already_count,
        })
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn failed_post_commit_rebuild_leaves_marker_recording_committed_seq() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1089-marker-fail");
    let jsonl = resolved.path.join("marker-fail.jsonl");
    fs::write(&jsonl, "{\"text\":\"issue1089 rebuild failure row\"}\n").unwrap();
    // Corrupt pre-existing manifest makes the post-commit rebuild fail after
    // the Base rows are durable — the same partial-commit shape as the
    // incident's external timeout kill.
    let manifest_path = resolved.path.join("idx/search/manifest.json");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(&manifest_path, b"{not-json").unwrap();

    let error = ingest_batch_streaming(&resolved, &jsonl).unwrap_err();

    let vault = open_vault(&resolved).unwrap();
    let latest_seq = vault.snapshot();
    let marker = read_rebuild_required_marker(&resolved.path)
        .unwrap()
        .expect("failed rebuild must leave the marker in place");
    assert_eq!(error.code(), "CALYX_INGEST_INDEX_REBUILD_FAILED");
    assert!(
        error.message().contains("rebuild_required_marker="),
        "{}",
        error.message()
    );
    assert_eq!(marker.source, "batch_ingest");
    assert_eq!(
        marker.required_base_seq,
        Some(latest_seq),
        "marker must record the exact durable seq the commit reached"
    );
    assert_eq!(
        marker.batch_path.as_deref(),
        Some(jsonl.display().to_string().as_str())
    );
    println!(
        "ISSUE1089_MARKER_FAIL_FSV {}",
        json!({
            "source_of_truth": "idx/search/rebuild-required.json content plus durable vault seq after failed post-commit rebuild",
            "error_code": error.code(),
            "marker_source": marker.source,
            "marker_required_base_seq": marker.required_base_seq,
            "vault_latest_seq": latest_seq,
            "marker_batch_path": marker.batch_path,
        })
    );
    fs::remove_dir_all(root).ok();
}
