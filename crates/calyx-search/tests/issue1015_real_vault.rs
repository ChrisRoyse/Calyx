use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, SlotId};
use calyx_search::PersistedSearchIndexes;
use calyx_sextant::index::{DiskAnnSearch, DiskAnnSearchParams, SextantIndex};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct IdMap {
    format: String,
    slot: u16,
    ids: Vec<CxId>,
}

#[test]
#[ignore = "manual FSV: set CALYX_ISSUE1015_VAULT to the real vault path"]
fn issue1015_real_vault_slot15_diskann_searches_from_physical_sidecars() {
    let vault = PathBuf::from(
        std::env::var_os("CALYX_ISSUE1015_VAULT")
            .expect("CALYX_ISSUE1015_VAULT must point to the real vault"),
    );
    let slot = SlotId::new(15);
    let before = read_slot_state(&vault, slot.get());
    let manifest = read_manifest(&vault);
    let entry = slot_entry(&manifest, slot.get());
    let graph_rel = entry["graph_rel"].as_str().expect("slot graph_rel");
    let id_map_rel = entry["id_map_rel"].as_str().expect("slot id_map_rel");
    let graph_path = vault.join(graph_rel);
    let id_map_path = vault.join(id_map_rel);
    let id_map = read_id_map(&id_map_path);
    assert_eq!(id_map.format, "calyx-search-index-idmap-v1");
    assert_eq!(id_map.slot, slot.get());
    assert_eq!(
        id_map.ids.len(),
        entry["len"].as_u64().expect("slot len") as usize
    );
    let query_id = *id_map
        .ids
        .first()
        .expect("slot 15 id map must not be empty");
    let diskann = DiskAnnSearch::open(
        slot,
        &graph_path,
        id_map.ids,
        None,
        DiskAnnSearchParams {
            beamwidth: 64,
            ef_search: 128,
            rescore_k: 128,
            rescore_from_raw: true,
        },
    )
    .expect("open slot 15 DiskANN sidecar");
    let query = diskann
        .vector(query_id)
        .expect("read stored slot 15 vector from DiskANN sidecar");
    let indexes = PersistedSearchIndexes::open(&vault).expect("open persisted search manifest");
    let hits = indexes
        .search(slot, &query, 10)
        .expect("search slot 15 persisted DiskANN index");
    assert!(
        hits.iter().any(|hit| hit.cx_id == query_id),
        "slot 15 search did not return the stored query id in top 10: {hits:?}"
    );
    let after = read_slot_state(&vault, slot.get());
    assert_eq!(
        before, after,
        "slot 15 read/search FSV must not mutate files"
    );

    maybe_write_fsv_json(&json!({
        "source_of_truth": "real vault idx/search manifest plus slot 15 DiskANN graph/id-map sidecars",
        "before": before,
        "after": after,
        "query_id": query_id,
        "hits": hits,
    }));
    println!(
        "ISSUE1015_REAL_VAULT_SLOT15_FSV {}",
        json!({
            "source_of_truth": "real vault idx/search manifest plus slot 15 DiskANN graph/id-map sidecars",
            "before": before,
            "after": after,
            "query_id": query_id,
            "hits": hits,
        })
    );
}

fn read_slot_state(vault: &Path, slot: u16) -> Value {
    let manifest_path = vault.join("idx/search/manifest.json");
    let manifest_bytes = fs::read(&manifest_path).expect("read manifest");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("decode manifest");
    let entry = slot_entry(&manifest, slot);
    let graph_rel = entry["graph_rel"].as_str().expect("slot graph_rel");
    let id_map_rel = entry["id_map_rel"].as_str().expect("slot id_map_rel");
    let graph_path = vault.join(graph_rel);
    let id_map_path = vault.join(id_map_rel);
    let graph_metadata = fs::metadata(&graph_path).expect("stat graph");
    let id_map_bytes = fs::read(&id_map_path).expect("read id map");
    let id_map: IdMap = serde_json::from_slice(&id_map_bytes).expect("decode id map");
    json!({
        "manifest_path": manifest_path.display().to_string(),
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "base_seq": manifest["base_seq"],
        "slot": slot,
        "kind": entry["kind"],
        "len": entry["len"],
        "dim": entry["dim"],
        "graph_path": graph_path.display().to_string(),
        "graph_exists": graph_path.is_file(),
        "graph_bytes": graph_metadata.len(),
        "graph_sha256": sha256_file(&graph_path),
        "id_map_path": id_map_path.display().to_string(),
        "id_map_bytes": id_map_bytes.len(),
        "id_map_sha256": sha256_hex(&id_map_bytes),
        "id_map_format": id_map.format,
        "id_map_slot": id_map.slot,
        "id_map_len": id_map.ids.len(),
        "id_map_first": id_map.ids.first().map(ToString::to_string),
    })
}

fn read_manifest(vault: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(vault.join("idx/search/manifest.json")).expect("read search manifest"),
    )
    .expect("decode search manifest")
}

fn slot_entry(manifest: &Value, slot: u16) -> Value {
    manifest["slots"]
        .as_array()
        .expect("manifest slots")
        .iter()
        .find(|entry| entry["slot"] == slot)
        .unwrap_or_else(|| panic!("manifest missing slot {slot}"))
        .clone()
}

fn read_id_map(path: &Path) -> IdMap {
    serde_json::from_slice(&fs::read(path).expect("read id map")).expect("decode id map")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sha256_file(path: &Path) -> String {
    let mut reader = BufReader::new(File::open(path).expect("open file for sha256"));
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let bytes = reader.read(&mut buffer).expect("read file for sha256");
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    format!("{:x}", hasher.finalize())
}

fn maybe_write_fsv_json(value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join("issue1015-real-vault-slot15-search.json"),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV artifact");
}
