use super::helpers::*;
use super::*;

#[test]
fn segmented_multi_manifest_id_count_mismatch_fails_closed() {
    let root = scratch("bad-segment-ids");
    rebuild_from_docs(&root, &mixed_docs(), 27).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let manifest_path = root.join("idx").join("search").join("manifest.json");
    let segment_manifest_rel = entry.index_rel.as_ref().expect("segment manifest rel");
    let segment_manifest_path = root.join(segment_manifest_rel);
    let before_manifest = read_multi_segment_manifest(&root, entry);
    let mut segment_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&segment_manifest_path).unwrap()).unwrap();
    let original_ids = segment_manifest["segments"][0]["ids"]
        .as_array()
        .expect("segment ids")
        .clone();
    assert!(original_ids.len() > 1);
    segment_manifest["segments"][0]["ids"] = json!([original_ids[0].clone()]);
    fs::write(
        &segment_manifest_path,
        serde_json::to_vec(&segment_manifest).unwrap(),
    )
    .unwrap();

    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for slot in manifest["slots"].as_array_mut().unwrap() {
        if slot["slot"] == 2 {
            slot["sha256"] = json!(sha256_hex(&fs::read(&segment_manifest_path).unwrap()));
        }
    }
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let corrupted = PersistedSearchIndexes::open(&root).expect("open tampered manifest");
    let err = corrupted
        .search(SlotId::new(2), &multi(2, [[1.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("id count 1 != row_count 3"));
    maybe_write_fsv_json(
        "issue1015-segmented-multi-id-count-edge.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "tamper segmented multi manifest ids to contain fewer ids than row_count while matching top-level sha",
            "before": before_manifest,
            "after": segment_manifest,
            "error": error_json(&err),
        }),
    );
    cleanup(root);
}

#[test]
fn oversized_segment_manifest_fails_before_binary_sidecar_read() {
    let root = scratch("oversized-segment");
    rebuild_from_docs(&root, &mixed_docs(), 27).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let manifest_path = root.join("idx").join("search").join("manifest.json");
    let segment_manifest_path = root.join(entry.index_rel.as_ref().unwrap());
    let before = read_multi_segment_manifest(&root, entry);
    let first_segment_rel = first_segment_rel(&before);
    let first_segment_path = root.join(&first_segment_rel);
    assert!(first_segment_path.exists());

    let mut segment_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&segment_manifest_path).unwrap()).unwrap();
    let oversized_tokens = 100_000_000usize;
    segment_manifest["token_count"] = json!(oversized_tokens);
    segment_manifest["segments"][0]["token_count"] = json!(oversized_tokens);
    fs::write(
        &segment_manifest_path,
        serde_json::to_vec(&segment_manifest).unwrap(),
    )
    .unwrap();
    fs::remove_file(&first_segment_path).unwrap();

    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for slot in manifest["slots"].as_array_mut().unwrap() {
        if slot["slot"] == 2 {
            slot["token_count"] = json!(oversized_tokens);
            slot["sha256"] = json!(sha256_hex(&fs::read(&segment_manifest_path).unwrap()));
        }
    }
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let corrupted = PersistedSearchIndexes::open(&root).expect("open tampered manifest");
    let err = corrupted
        .ensure_search_bounded_for_slots(Some(&BTreeSet::from([SlotId::new(2)])))
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_SEARCH_MULTI_SIDECAR_UNBOUNDED");
    assert!(err.message().contains("persistent binary multi segment"));
    assert!(
        err.message()
            .contains("exceeds search binary segment limit")
    );
    maybe_write_fsv_json(
        "issue1042-oversized-segment-fail-closed.json",
        &json!({
            "source_of_truth": root.display().to_string(),
            "trigger": "tamper segmented multi manifest to describe an oversized segment, then remove the binary sidecar",
            "before": before,
            "after": segment_manifest,
            "binary_sidecar_exists_after": first_segment_path.exists(),
            "error": error_json(&err),
        }),
    );
    cleanup(root);
}
