use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::SlotId;
use calyx_search::PersistedSearchIndexes;
use serde_json::{Value, json};

const MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

#[test]
#[ignore = "manual FSV: set CALYX_ISSUE1042_VAULT to the real vault path"]
fn issue1042_real_vault_segment_boundedness_matches_physical_manifest() {
    let vault = PathBuf::from(
        std::env::var_os("CALYX_ISSUE1042_VAULT")
            .expect("CALYX_ISSUE1042_VAULT must point to the real vault"),
    );
    let before = read_segment_state(&vault);
    let oversized = before["oversized_segments"].as_array().unwrap();
    let indexes = PersistedSearchIndexes::open(&vault).expect("open persisted search index");
    let slot10_result =
        indexes.ensure_search_bounded_for_slots(Some(&BTreeSet::from([SlotId::new(10)])));
    slot10_result.expect("non-multi slot 10 should not inspect slot 22");
    let empty_slots = BTreeSet::new();
    indexes
        .ensure_search_bounded_for_slots(Some(&empty_slots))
        .expect("empty selected-slot set should be a no-op");
    let result = indexes.ensure_search_bounded_for_slots(Some(&BTreeSet::from([SlotId::new(22)])));

    let outcome = match (oversized.is_empty(), result) {
        (true, Ok(())) => json!({"bounded": true}),
        (true, Err(error)) => panic!("bounded physical manifest failed: {}", error.message()),
        (false, Ok(())) => panic!("oversized physical manifest was accepted"),
        (false, Err(error)) => {
            assert_eq!(error.code(), "CALYX_SEARCH_MULTI_SIDECAR_UNBOUNDED");
            json!({
                "bounded": false,
                "error": {
                    "code": error.code(),
                    "message": error.message(),
                }
            })
        }
    };
    let after = read_segment_state(&vault);
    assert_eq!(
        before, after,
        "boundedness check must not mutate source files"
    );
    maybe_write_fsv_json(&json!({
        "source_of_truth": vault.display().to_string(),
        "before": before,
        "after": after,
        "outcome": outcome,
        "edge_checks": {
            "slot_10_only": "ok",
            "empty_selected_slots": "ok",
        },
    }));
}

fn read_segment_state(vault: &Path) -> Value {
    let manifest_path = vault.join("idx/search/manifest.json");
    let manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read search manifest"))
            .expect("decode search manifest");
    let slot = manifest["slots"]
        .as_array()
        .expect("manifest slots")
        .iter()
        .find(|slot| slot["slot"] == 22)
        .expect("slot 22 manifest entry");
    let segment_manifest_path = vault.join(slot["index_rel"].as_str().expect("segment manifest"));
    let segment_manifest: Value =
        serde_json::from_slice(&fs::read(&segment_manifest_path).expect("read segment manifest"))
            .expect("decode segment manifest");
    let token_dim = slot["token_dim"].as_u64().expect("token_dim") as u32;
    let segments = segment_manifest["segments"].as_array().expect("segments");
    let segment_rows = segments
        .iter()
        .map(|segment| {
            let rows = segment["row_count"].as_u64().expect("row_count");
            let tokens = segment["token_count"].as_u64().expect("token_count");
            let estimated_bytes = estimated_segment_bytes(token_dim, rows, tokens);
            json!({
                "index_rel": segment["index_rel"],
                "row_count": rows,
                "token_count": tokens,
                "estimated_bytes": estimated_bytes,
                "bounded": estimated_bytes <= MAX_SEGMENT_BYTES,
            })
        })
        .collect::<Vec<_>>();
    let oversized_segments = segment_rows
        .iter()
        .filter(|segment| !segment["bounded"].as_bool().unwrap())
        .cloned()
        .collect::<Vec<_>>();
    json!({
        "manifest_path": manifest_path.display().to_string(),
        "segment_manifest_path": segment_manifest_path.display().to_string(),
        "slot": 22,
        "kind": slot["kind"],
        "len": slot["len"],
        "token_dim": token_dim,
        "token_count": slot["token_count"],
        "segment_count": segments.len(),
        "max_segment_bytes": MAX_SEGMENT_BYTES,
        "segments": segment_rows,
        "oversized_segments": oversized_segments,
    })
}

fn estimated_segment_bytes(token_dim: u32, row_count: u64, token_count: u64) -> u64 {
    let header = 16 + 2 + 4 + 8 + 8 + 8;
    let row_headers = row_count * (16 + 4);
    let payload = token_count * token_dim as u64 * 4;
    header + row_headers + payload
}

fn maybe_write_fsv_json(value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join("issue1042-real-vault-segment-boundedness.json"),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV artifact");
}
