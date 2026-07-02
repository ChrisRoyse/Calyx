//! #1109 — a rebuild must never require prior derived sidecars as input.
//!
//! The 2026-07-02 calyx15000 incident: a post-commit rebuild hard-failed
//! `CALYX_STALE_DERIVED` because the previous manifest's slot-22
//! `.multi.segments.json` was physically absent, even though every committed
//! Base CF row needed for a fresh build was present. Prior segments are an
//! append optimization (#1015), not a precondition: when they are missing or
//! corrupt the rebuild must decline reuse with a structured progress event
//! and build everything fresh from the source rows.

use super::helpers::*;
use super::*;

const MULTI_SLOT: u16 = 2;

fn multi_entry(indexes: &PersistedSearchIndexes) -> SearchIndexEntry {
    indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == MULTI_SLOT)
        .expect("multi entry")
        .clone()
}

fn append_fourth_doc(docs: &mut BTreeMap<CxId, Constellation>) {
    docs.insert(
        cx(4),
        constellation(
            cx(4),
            [
                (SlotId::new(0), dense(vec![0.4, 0.6])),
                (SlotId::new(1), sparse(8, [4])),
                (SlotId::new(2), multi(2, [[0.25, 0.75]])),
            ],
        ),
    );
}

/// Verifies the published manifest against physical disk state: the multi
/// entry's segments manifest and every `.multi.bin` it references must exist
/// and hash-verify, and search over the rebuilt index must work.
fn assert_multi_index_intact(root: &std::path::Path, expected_rows: usize) -> serde_json::Value {
    let indexes = PersistedSearchIndexes::open(root).expect("open rebuilt indexes");
    let entry = multi_entry(&indexes);
    let segments_manifest = read_multi_segment_manifest(root, &entry);
    assert_eq!(segments_manifest["row_count"], expected_rows);
    for segment in segments_manifest["segments"].as_array().expect("segments") {
        let rel = segment["index_rel"].as_str().expect("segment rel");
        let bytes = fs::read(root.join(rel)).expect("referenced segment must exist");
        assert_eq!(
            segment["sha256"].as_str().expect("segment sha"),
            sha256_hex(&bytes),
            "published manifest must only reference hash-verified sidecars: {rel}"
        );
    }
    let hits = indexes
        .search(SlotId::new(MULTI_SLOT), &multi(2, [[0.25, 0.75]]), 4)
        .expect("search over rebuilt multi index");
    assert_eq!(hits[0].cx_id, cx(4));
    segments_manifest
}

#[test]
fn rebuild_succeeds_when_prior_segments_manifest_is_missing() {
    let root = scratch("missing-prior-segments-manifest");
    let mut docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 40).expect("first rebuild");
    let first = PersistedSearchIndexes::open(&root).expect("open first");
    let prior_entry = multi_entry(&first);
    let prior_rel = prior_entry.index_rel.as_ref().expect("prior rel").clone();
    fs::remove_file(root.join(&prior_rel)).expect("simulate missing prior sidecar");

    append_fourth_doc(&mut docs);
    let summary = rebuild_from_docs(&root, &docs, 41)
        .expect("#1109: rebuild must not require the prior segments manifest");

    assert_eq!(summary.total_rows, 12, "3 slots x 4 rows");
    let after = assert_multi_index_intact(&root, 4);
    maybe_write_fsv_json(
        "issue1109-missing-prior-segments-manifest.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "delete prior .multi.segments.json, ingest one more row, rebuild",
            "deleted_prior_sidecar": prior_rel,
            "after_manifest": after,
        }),
    );
    cleanup(root);
}

#[test]
fn rebuild_succeeds_when_prior_binary_segment_is_missing() {
    let root = scratch("missing-prior-binary-segment");
    let mut docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 42).expect("first rebuild");
    let first = PersistedSearchIndexes::open(&root).expect("open first");
    let prior_manifest = read_multi_segment_manifest(&root, &multi_entry(&first));
    let prior_segment = first_segment_rel(&prior_manifest);
    fs::remove_file(root.join(&prior_segment)).expect("simulate missing prior binary segment");

    append_fourth_doc(&mut docs);
    let summary = rebuild_from_docs(&root, &docs, 43)
        .expect("#1109: rebuild must not require prior binary segments");

    assert_eq!(summary.total_rows, 12);
    let after = assert_multi_index_intact(&root, 4);
    for segment in after["segments"].as_array().expect("segments") {
        assert_ne!(
            segment["index_rel"].as_str().expect("rel"),
            prior_segment,
            "a fresh build must not reference the vanished prior segment"
        );
    }
    cleanup(root);
}

#[test]
fn reuse_decline_emits_structured_progress_event() {
    let root = scratch("reuse-decline-event");
    let docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 44).expect("first rebuild");
    let first = PersistedSearchIndexes::open(&root).expect("open first");
    let prior_entry = multi_entry(&first);
    let prior_rel = prior_entry.index_rel.as_ref().expect("prior rel").clone();
    fs::remove_file(root.join(&prior_rel)).expect("simulate missing prior sidecar");

    let idx_root = root.join("idx").join("search");
    let rows = multi::collect(&docs)
        .expect("collect multi rows")
        .remove(&SlotId::new(MULTI_SLOT))
        .expect("multi slot rows");
    let mut events: Vec<(String, Option<u16>, Option<String>)> = Vec::new();
    let entry = multi::write(
        &root,
        &idx_root,
        SlotId::new(MULTI_SLOT),
        rows,
        45,
        Some(&prior_entry),
        &mut |event| {
            events.push((
                event.phase.to_string(),
                event.slot.map(|slot| slot.get()),
                event.detail.clone(),
            ));
            Ok(())
        },
    )
    .expect("write declines reuse and builds fresh");

    multi::validate_entry(&root, &entry, 45, SlotId::new(MULTI_SLOT))
        .expect("fresh entry validates");
    let decline = events
        .iter()
        .find(|(phase, _, _)| phase == "multi_segment_reuse_declined")
        .expect("decline event must be emitted");
    assert_eq!(decline.1, Some(MULTI_SLOT));
    let detail = decline.2.as_deref().expect("decline detail");
    assert!(
        detail.contains(&prior_rel),
        "detail names the artifact: {detail}"
    );
    assert!(
        detail.contains("rebuilding slot 2 fresh from source rows"),
        "detail states the recovery action: {detail}"
    );
    assert!(
        detail.contains("CALYX_STALE_DERIVED"),
        "detail carries the underlying error: {detail}"
    );
    maybe_write_fsv_json(
        "issue1109-reuse-decline-event.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "multi::write with a previous entry whose sidecar is deleted",
            "events": events
                .iter()
                .map(|(phase, slot, detail)| json!({
                    "phase": phase,
                    "slot": slot,
                    "detail": detail,
                }))
                .collect::<Vec<_>>(),
        }),
    );
    cleanup(root);
}
