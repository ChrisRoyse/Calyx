use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{FixedClock, VaultId};
use proptest::prelude::*;

use crate::collection::{
    DedupPolicy, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::vault::VaultOptions;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn blob_collection() -> Collection {
    Collection {
        name: "blobs".to_string(),
        mode: CollectionMode::Blob,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

/// Deterministic synthetic payload: byte i = (i mod 251). Known input → known
/// bytes, so reassembly and hashing are hand-verifiable.
fn synthetic(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn put_get_round_trips_and_manifest_matches_hash() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    let id = BlobId::from_text("b1");
    let data = b"hello world".repeat(10);

    layer.blob_put(&col, id, &data).unwrap();

    // Source of truth: the manifest row, read independently.
    let manifest = layer.blob_manifest(&col, id).unwrap().unwrap();
    assert_eq!(manifest.total_bytes, data.len() as u64);
    assert_eq!(manifest.chunk_count, 1); // 110 bytes < 256 KiB
    assert_eq!(&manifest.content_hash, blake3::hash(&data).as_bytes());
    assert!(!manifest.cold_tier);
    assert_eq!(manifest.created_at_ms, Some(1));

    assert_eq!(layer.blob_get(&col, id).unwrap(), Some(data));
}

#[test]
fn payload_spanning_three_chunks_reassembles() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    let id = BlobId::from_bytes([7; 16]);
    let data = synthetic(BLOB_CHUNK_SIZE * 2 + 1);

    layer.blob_put(&col, id, &data).unwrap();

    let manifest = layer.blob_manifest(&col, id).unwrap().unwrap();
    assert_eq!(manifest.chunk_count, 3);
    assert_eq!(manifest.total_bytes, (BLOB_CHUNK_SIZE * 2 + 1) as u64);
    assert_eq!(manifest.created_at_ms, Some(1));

    // Independent read of each chunk row proves the physical split.
    let snap = vault.latest_seq();
    let c0 = vault
        .read_cf_at(snap, ColumnFamily::Blob, &chunk_key(&col, id, 0))
        .unwrap()
        .unwrap();
    let c2 = vault
        .read_cf_at(snap, ColumnFamily::Blob, &chunk_key(&col, id, 2))
        .unwrap()
        .unwrap();
    assert_eq!(c0.len(), BLOB_CHUNK_SIZE);
    assert_eq!(c2.len(), 1);

    assert_eq!(layer.blob_get(&col, id).unwrap(), Some(data));
}

#[test]
fn stream_chunks_yields_chunks_in_order() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    let id = BlobId::from_text("stream");
    let data = synthetic(BLOB_CHUNK_SIZE + 123);

    layer.blob_put(&col, id, &data).unwrap();

    let mut reassembled = Vec::new();
    let mut count = 0;
    for chunk in layer.blob_stream_chunks(&col, id).unwrap() {
        reassembled.extend_from_slice(&chunk.unwrap());
        count += 1;
    }
    assert_eq!(count, 2);
    assert_eq!(reassembled, data);
}

#[test]
fn delete_tombstones_all_rows() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    let id = BlobId::from_text("doomed");
    let data = synthetic(BLOB_CHUNK_SIZE + 5); // 2 chunks

    layer.blob_put(&col, id, &data).unwrap();
    assert!(layer.blob_get(&col, id).unwrap().is_some());

    layer.blob_delete(&col, id).unwrap();
    assert_eq!(layer.blob_get(&col, id).unwrap(), None);
    assert_eq!(layer.blob_manifest(&col, id).unwrap(), None);
    // Chunk rows are tombstoned (filtered) too.
    let snap = vault.latest_seq();
    assert_eq!(
        vault
            .read_cf_at(snap, ColumnFamily::Blob, &chunk_key(&col, id, 0))
            .unwrap(),
        None
    );
}

#[test]
fn edge_cases_fail_closed_with_exact_codes() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();

    // (1) empty payload -> 0 chunks, manifest total_bytes=0, get == Some(b"").
    let empty_id = BlobId::from_text("empty");
    layer.blob_put(&col, empty_id, b"").unwrap();
    let manifest = layer.blob_manifest(&col, empty_id).unwrap().unwrap();
    assert_eq!(manifest.chunk_count, 0);
    assert_eq!(manifest.total_bytes, 0);
    assert_eq!(&manifest.content_hash, blake3::hash(b"").as_bytes());
    assert_eq!(manifest.created_at_ms, Some(1));
    assert_eq!(layer.blob_get(&col, empty_id).unwrap(), Some(Vec::new()));

    // (2) absent blob -> None.
    assert_eq!(
        layer.blob_get(&col, BlobId::from_text("ghost")).unwrap(),
        None
    );

    // (3) flip one byte in a chunk row -> corrupt on get (hash mismatch).
    let corrupt_id = BlobId::from_text("corrupt");
    let data = synthetic(1000);
    layer.blob_put(&col, corrupt_id, &data).unwrap();
    let mut tampered = data.clone();
    tampered[0] ^= 0xff;
    vault
        .write_cf(ColumnFamily::Blob, chunk_key(&col, corrupt_id, 0), tampered)
        .unwrap();
    assert_eq!(
        layer.blob_get(&col, corrupt_id).unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );

    // (4) wrong collection mode -> invalid argument.
    let mut wrong = col.clone();
    wrong.mode = CollectionMode::KV;
    assert_eq!(
        layer
            .blob_put(&wrong, BlobId::from_text("x"), b"y")
            .unwrap_err()
            .code,
        CALYX_INVALID_ARGUMENT
    );

    // (5) corrupt manifest length -> fail closed.
    let bad_manifest = BlobId::from_text("badmanifest");
    vault
        .write_cf(
            ColumnFamily::Blob,
            manifest_key(&col, bad_manifest),
            vec![0; 3],
        )
        .unwrap();
    assert_eq!(
        layer.blob_manifest(&col, bad_manifest).unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );
}

#[test]
fn oversized_payload_fails_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    // 1 GiB + 1 byte. The size guard returns before any chunking/hashing, so
    // this allocation is the only cost.
    let oversized = vec![0_u8; MAX_BLOB_BYTES + 1];
    let error = layer
        .blob_put(&col, BlobId::from_text("huge"), &oversized)
        .unwrap_err();
    assert_eq!(error.code, CALYX_BLOB_TOO_LARGE);
    assert_eq!(
        layer
            .blob_manifest(&col, BlobId::from_text("huge"))
            .unwrap(),
        None
    );
}

#[test]
fn missing_manifest_with_orphan_chunks_reads_none_not_partial() {
    // Simulates a crash between phase 1 (chunks) and phase 2 (manifest): chunk
    // rows exist but no manifest. blob_get must return None, never partial data.
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    let id = BlobId::from_text("orphan");

    let data = synthetic(BLOB_CHUNK_SIZE + 10);
    let chunks: Vec<_> = data
        .chunks(BLOB_CHUNK_SIZE)
        .enumerate()
        .map(|(i, c)| {
            (
                ColumnFamily::Blob,
                chunk_key(&col, id, i as u32),
                c.to_vec(),
            )
        })
        .collect();
    vault.write_cf_batch(chunks).unwrap();

    // Chunk rows physically present...
    assert!(
        vault
            .read_cf_at(
                vault.latest_seq(),
                ColumnFamily::Blob,
                &chunk_key(&col, id, 0)
            )
            .unwrap()
            .is_some()
    );
    // ...but no manifest => blob is invisible.
    assert_eq!(layer.blob_manifest(&col, id).unwrap(), None);
    assert_eq!(layer.blob_get(&col, id).unwrap(), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn put_then_get_roundtrips(
        data in proptest::collection::vec(any::<u8>(), 0..(BLOB_CHUNK_SIZE * 2 + 10)),
    ) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1));
        let layer = BlobLayer::new(&vault);
        let col = blob_collection();
        let id = BlobId::from_bytes([0xab; 16]);
        layer.blob_put(&col, id, &data).unwrap();
        let manifest = layer.blob_manifest(&col, id).unwrap().unwrap();
        let expected_hash = *blake3::hash(&data).as_bytes();
        prop_assert_eq!(manifest.total_bytes, data.len() as u64);
        prop_assert_eq!(manifest.content_hash, expected_hash);
        prop_assert_eq!(layer.blob_get(&col, id).unwrap(), Some(data));
    }
}

#[test]
fn durable_blob_fsv_writes_readback_artifacts() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("blob-vault"))
        .unwrap_or_else(|| temp_dir("blob-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"blob-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = BlobLayer::new(&vault);
    let col = blob_collection();
    create_collection(&vault, col.clone()).unwrap();

    let id = BlobId::from_text("b1");
    let data = synthetic(2 * 1024 * 1024); // 2 MiB -> 8 chunks
    let expected_hash = *blake3::hash(&data).as_bytes();
    layer.blob_put(&col, id, &data).unwrap();

    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"blob-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_layer = BlobLayer::new(&reopened);

    let manifest = reopened_layer.blob_manifest(&col, id).unwrap().unwrap();
    assert_eq!(manifest.chunk_count, 8);
    assert_eq!(manifest.total_bytes, data.len() as u64);
    assert_eq!(manifest.content_hash, expected_hash);
    // Byte-exact round-trip across a cold reopen (the `cmp` equivalent).
    let roundtrip = reopened_layer.blob_get(&col, id).unwrap().unwrap();
    assert_eq!(roundtrip, data);

    let cf_files = physical_files(&dir.join("cf").join("blob"));
    assert!(!cf_files.is_empty(), "cf/blob must hold on-disk shards");

    let ck = chunk_key(&col, id, 0);
    let mk = manifest_key(&col, id);
    let readback = serde_json::json!({
        "issue": 454,
        "layer": "blob",
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::Blob.name(),
        "chunk_key_hex": hex_bytes(&ck),
        "chunk_disc": format!("{:#04x}", ck[0]),
        "chunk_kind": format!("{:#04x}", ck[1]),
        "manifest_key_hex": hex_bytes(&mk),
        "manifest_kind": format!("{:#04x}", mk[1]),
        "manifest_chunk_count": manifest.chunk_count,
        "manifest_total_bytes": manifest.total_bytes,
        "manifest_created_at_ms": manifest.created_at_ms,
        "content_hash_hex": hex_bytes(&manifest.content_hash),
        "roundtrip_byte_exact": roundtrip == data,
        "blob_cf_files": cf_files,
    });
    assert_eq!(readback["roundtrip_byte_exact"], serde_json::json!(true));

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-blob-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        println!("ph53_blob_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn physical_files(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            }));
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
