use std::collections::BTreeSet;
use std::fs;

use calyx_core::SlotId;
use serde_json::json;

use super::*;

#[path = "mixed_tests/helpers.rs"]
mod helpers;
use helpers::*;
#[path = "mixed_tests/missing_prior_sidecar.rs"]
mod missing_prior_sidecar;
#[path = "mixed_tests/pinned_semantics.rs"]
mod pinned_semantics;
#[path = "mixed_tests/segment_edges.rs"]
mod segment_edges;

#[test]
fn rebuild_writes_sparse_and_multi_sidecars_and_searches() {
    let root = scratch("mixed");
    let docs = mixed_docs();

    let summary = rebuild_from_docs(&root, &docs, 21).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let sparse_hits = indexes
        .search(SlotId::new(1), &sparse(8, [1]), 2)
        .expect("sparse search");
    let multi_hits = indexes
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 2)
        .expect("multi search");
    let sparse_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .expect("sparse entry");
    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let sparse_json: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(sparse_entry.index_rel.as_ref().unwrap())).unwrap(),
    )
    .unwrap();
    let multi_manifest = read_multi_segment_manifest(&root, multi_entry);
    let first_segment = first_segment_rel(&multi_manifest);
    let multi_bytes = fs::read(root.join(&first_segment)).unwrap();

    assert_eq!(summary.slots, 3);
    assert_eq!(summary.total_rows, 9);
    assert_eq!(sparse_entry.kind, "sparse_inverted");
    assert_eq!(sparse_entry.dim, Some(8));
    assert_eq!(multi_entry.kind, "multi_maxsim_segments");
    assert_eq!(multi_entry.token_dim, Some(2));
    assert_eq!(multi_entry.token_count, Some(5));
    assert!(
        multi_entry
            .index_rel
            .as_ref()
            .unwrap()
            .ends_with(".multi.segments.json")
    );
    assert_eq!(sparse_json["format"], "calyx-search-sparse-index-v1");
    assert_eq!(sparse_json["doc_lengths"].as_object().unwrap().len(), 3);
    assert!(sparse_json["avg_doc_len"].as_f64().unwrap() > 0.0);
    assert_eq!(
        multi_manifest["format"],
        "calyx-search-multi-maxsim-segments-v1"
    );
    assert_eq!(multi_manifest["row_count"], 3);
    assert_eq!(multi_manifest["token_count"], 5);
    assert_eq!(multi_manifest["segments"].as_array().unwrap().len(), 1);
    assert_eq!(
        multi_manifest["segments"][0]["ids"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        multi_manifest["segments"][0]["sha256"].as_str().unwrap(),
        sha256_hex(&multi_bytes)
    );
    assert!(multi_bytes.starts_with(b"CYX_MULTI_BIN_V1"));
    assert_eq!(sparse_hits[0].cx_id, cx(3));
    assert_eq!(sparse_hits[1].cx_id, cx(1));
    assert_eq!(multi_hits[0].cx_id, cx(2));
    maybe_write_fsv_json(
        "issue980-binary-multi-happy-path.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "rebuild_from_docs over mixed dense/sparse/multi constellations",
            "manifest": &indexes.manifest,
            "multi_sidecar": {
                "manifest_path": &multi_entry.index_rel,
                "segment_path": first_segment,
                "manifest": multi_manifest,
                "bytes": multi_bytes.len(),
                "magic": String::from_utf8_lossy(&multi_bytes[..16]).to_string(),
                "sha256": sha256_hex(&multi_bytes),
            },
            "search": {
                "sparse_hits": sparse_hits.iter().map(|hit| hit.cx_id.to_string()).collect::<Vec<_>>(),
                "multi_hits": multi_hits.iter().map(|hit| hit.cx_id.to_string()).collect::<Vec<_>>(),
            }
        }),
    );
    cleanup(root);
}

#[test]
fn segmented_multi_rebuild_reuses_existing_segment_for_append() {
    let root = scratch("segmented-reuse");
    let mut docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 30).expect("first rebuild");
    let first_indexes = PersistedSearchIndexes::open(&root).expect("open first");
    let first_entry = first_indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("first multi entry");
    let first_manifest = read_multi_segment_manifest(&root, first_entry);
    let first_segment = first_segment_rel(&first_manifest);

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
    rebuild_from_docs(&root, &docs, 31).expect("second rebuild");
    let second_indexes = PersistedSearchIndexes::open(&root).expect("open second");
    let second_entry = second_indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("second multi entry");
    let second_manifest = read_multi_segment_manifest(&root, second_entry);
    let segments = second_manifest["segments"].as_array().unwrap();
    let reused_segment = segments
        .iter()
        .find(|segment| segment["index_rel"].as_str() == Some(first_segment.as_str()))
        .expect("old segment reused");
    let new_segment = segments
        .iter()
        .find(|segment| segment["index_rel"].as_str() != Some(first_segment.as_str()))
        .expect("new segment written");
    let reused_segment_json = reused_segment.clone();
    let new_segment_json = new_segment.clone();
    let hits = second_indexes
        .search(SlotId::new(2), &multi(2, [[0.25, 0.75]]), 4)
        .expect("segmented search");

    assert_eq!(second_entry.kind, "multi_maxsim_segments");
    assert_eq!(second_entry.token_count, Some(6));
    assert_eq!(segments.len(), 2);
    assert_eq!(reused_segment["row_count"], 3);
    assert_eq!(reused_segment["ids"].as_array().unwrap().len(), 3);
    assert_eq!(new_segment["row_count"], 1);
    assert_eq!(new_segment["ids"].as_array().unwrap().len(), 1);
    assert_eq!(second_manifest["row_count"], 4);
    assert_eq!(second_manifest["token_count"], 6);
    assert!(root.join(&first_segment).exists());
    assert_eq!(hits[0].cx_id, cx(4));
    maybe_write_fsv_json(
        "issue1015-segmented-multi-reuse.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "two rebuilds where the second appends one multi-vector row",
            "before": {
                "manifest": first_manifest,
                "segment": sidecar_state(&root, &first_segment),
            },
            "after": {
                "manifest": second_manifest,
                "reused_segment": reused_segment_json,
                "new_segment": new_segment_json,
                "hits": hits.iter().map(|hit| hit.cx_id.to_string()).collect::<Vec<_>>(),
            }
        }),
    );
    cleanup(root);
}

#[test]
fn filtered_sparse_and_multi_search_use_candidate_sidecars() {
    let root = scratch("mixed-filtered");
    let docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 22).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let candidates = BTreeSet::from([cx(1), cx(2)]);

    let sparse_hits = indexes
        .search_filtered(SlotId::new(1), &sparse(8, [1]), 3, &candidates)
        .expect("filtered sparse");
    let multi_hits = indexes
        .search_filtered(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 3, &candidates)
        .expect("filtered multi");

    assert_eq!(
        sparse_hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        vec![cx(1)]
    );
    assert_eq!(
        multi_hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        vec![cx(2), cx(1)]
    );
    cleanup(root);
}

#[test]
fn missing_sparse_sidecar_fails_closed() {
    let root = scratch("missing-sparse");
    rebuild_from_docs(&root, &mixed_docs(), 23).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .unwrap();
    fs::remove_file(root.join(entry.index_rel.as_ref().unwrap())).unwrap();

    let err = indexes
        .search(SlotId::new(1), &sparse(8, [1]), 1)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sparse sidecar missing"));
    cleanup(root);
}

#[test]
fn sparse_dim_mismatch_and_corrupt_sidecar_fail_closed() {
    let root = scratch("bad-sparse");
    rebuild_from_docs(&root, &mixed_docs(), 24).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let dim_err = indexes
        .search(SlotId::new(1), &sparse(9, [1]), 1)
        .unwrap_err();

    assert_eq!(dim_err.code(), "CALYX_STALE_DERIVED");
    assert!(dim_err.message().contains("index dim 8 != query dim 9"));

    let mut corrupted = PersistedSearchIndexes::open(&root).expect("open");
    let pos = corrupted
        .manifest
        .slots
        .iter()
        .position(|entry| entry.slot == 1)
        .unwrap();
    let path = root.join(corrupted.manifest.slots[pos].index_rel.as_ref().unwrap());
    fs::write(&path, b"{not json").unwrap();
    corrupted.manifest.slots[pos].sha256 = Some(sha256_hex(&fs::read(&path).unwrap()));

    let corrupt_err = corrupted
        .search(SlotId::new(1), &sparse(8, [1]), 1)
        .unwrap_err();

    assert_eq!(corrupt_err.code(), "CALYX_STALE_DERIVED");
    assert!(corrupt_err.message().contains("not valid JSON"));
    cleanup(root);
}

#[test]
fn missing_multi_sidecar_token_dim_mismatch_and_corrupt_sidecar_fail_closed() {
    let root = scratch("bad-multi");
    rebuild_from_docs(&root, &mixed_docs(), 25).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let dim_err = indexes
        .search(SlotId::new(2), &multi(3, [[1.0, 0.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(dim_err.code(), "CALYX_STALE_DERIVED");
    assert!(
        dim_err
            .message()
            .contains("token_dim 2 != query token_dim 3")
    );

    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .unwrap();
    let segment_manifest = read_multi_segment_manifest(&root, entry);
    let segment_rel = first_segment_rel(&segment_manifest);
    let before = json!({
        "source_of_truth": root.display().to_string(),
        "manifest": &indexes.manifest,
        "sidecar": sidecar_state(&root, entry.index_rel.as_ref().unwrap()),
        "segment": sidecar_state(&root, &segment_rel),
    });
    let original = fs::read(root.join(&segment_rel)).unwrap();
    fs::remove_file(root.join(&segment_rel)).unwrap();
    let missing_err = indexes
        .search(SlotId::new(2), &multi(2, [[1.0, 0.0]]), 1)
        .unwrap_err();
    let after_missing = sidecar_state(&root, &segment_rel);

    assert_eq!(missing_err.code(), "CALYX_STALE_DERIVED");
    assert!(
        missing_err
            .message()
            .contains("segmented multi sidecar missing")
    );
    fs::write(root.join(&segment_rel), original).unwrap();

    let corrupted = PersistedSearchIndexes::open(&root).expect("open");
    let path = root.join(&segment_rel);
    fs::write(&path, b"{not a binary multi sidecar").unwrap();
    let corrupt_err = corrupted
        .search(SlotId::new(2), &multi(2, [[1.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(corrupt_err.code(), "CALYX_STALE_DERIVED");
    assert!(corrupt_err.message().contains("invalid magic"));
    maybe_write_fsv_json(
        "issue980-binary-multi-edge-cases.json",
        &json!({
            "trigger": "token-dim mismatch, missing sidecar, corrupt magic",
            "before": before,
            "after_missing": after_missing,
            "after_corrupt": sidecar_state(&root, &segment_rel),
            "errors": {
                "token_dim": error_json(&dim_err),
                "missing_sidecar": error_json(&missing_err),
                "corrupt_magic": error_json(&corrupt_err),
            }
        }),
    );
    cleanup(root);
}

#[test]
fn boundedness_check_is_scoped_to_selected_slots() {
    let root = scratch("bounded-selected");
    rebuild_from_docs(&root, &mixed_docs(), 26).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .unwrap();
    fs::remove_file(root.join(multi_entry.index_rel.as_ref().unwrap())).unwrap();

    indexes
        .ensure_search_bounded_for_slots(Some(&BTreeSet::from([SlotId::new(0)])))
        .expect("unselected corrupt multi sidecar is not opened");
    let err = indexes
        .ensure_search_bounded_for_slots(Some(&BTreeSet::from([SlotId::new(2)])))
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("multi sidecar missing"));
    cleanup(root);
}

#[test]
fn rebuild_prunes_stale_slot_and_filter_artifacts_after_manifest_swap() {
    let root = scratch("prune-stale");
    let stale_slot = root
        .join("idx")
        .join("search")
        .join("slot_00022_seq_00000000000000000001_n_0000000001.multi.json");
    let stale_filter = root
        .join("idx")
        .join("search")
        .join("filters_seq_00000000000000000001_n_0000000001.json");
    let stale_legacy_filter = root
        .join("idx")
        .join("search")
        .join("filter_seq_00000000000000000001_n_0000000001.json");
    let stale_ann = root
        .join("idx")
        .join("search")
        .join("slot_00000_seq_00000000000000000001_n_0000000001.ann");
    fs::create_dir_all(stale_ann.join("nested")).unwrap();
    fs::write(&stale_slot, b"old json").unwrap();
    fs::write(&stale_filter, b"old filter").unwrap();
    fs::write(&stale_legacy_filter, b"old legacy filter").unwrap();
    let before = json!({
        "stale_slot_exists": stale_slot.exists(),
        "stale_filter_exists": stale_filter.exists(),
        "stale_legacy_filter_exists": stale_legacy_filter.exists(),
        "stale_ann_exists": stale_ann.exists(),
    });

    rebuild_from_docs(&root, &mixed_docs(), 26).expect("rebuild");

    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    for entry in &indexes.manifest.slots {
        if let Some(rel) = &entry.index_rel {
            assert!(root.join(rel).exists(), "manifest sidecar missing: {rel}");
        }
        if let Some(rel) = &entry.graph_rel {
            assert!(root.join(rel).exists(), "manifest graph missing: {rel}");
        }
        if let Some(rel) = &entry.id_map_rel {
            assert!(root.join(rel).exists(), "manifest id map missing: {rel}");
        }
    }

    assert!(!stale_slot.exists());
    assert!(!stale_filter.exists());
    assert!(!stale_legacy_filter.exists());
    assert!(!stale_ann.exists());
    let active_filter = indexes
        .manifest
        .filter
        .as_ref()
        .expect("manifest filter")
        .index_rel
        .clone();
    assert!(root.join(&active_filter).exists());
    maybe_write_fsv_json(
        "issue983-prune-stale-filter-artifacts.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "manifest swap after stale slot/filter artifacts were present",
            "before": before,
            "after": {
                "stale_slot_exists": stale_slot.exists(),
                "stale_filter_exists": stale_filter.exists(),
                "stale_legacy_filter_exists": stale_legacy_filter.exists(),
                "stale_ann_exists": stale_ann.exists(),
                "active_filter": active_filter,
                "active_filter_exists": root.join(&active_filter).exists(),
                "manifest_refs": indexes.manifest.slots.iter().map(|entry| {
                    json!({
                        "slot": entry.slot,
                        "index_rel": &entry.index_rel,
                        "graph_rel": &entry.graph_rel,
                        "id_map_rel": &entry.id_map_rel,
                    })
                }).collect::<Vec<_>>(),
            }
        }),
    );
    cleanup(root);
}
