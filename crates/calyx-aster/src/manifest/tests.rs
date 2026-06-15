use super::*;
use crate::sst::write_sst;
use crate::wal::{Wal, WalOptions};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn manifest_swap_uses_current_pointer_atomically() {
    let dir = test_dir("manifest-swap");
    write_manifest_assets(&dir);
    let store = ManifestStore::open(&dir);
    let first = manifest(1, 10);
    let second = manifest(2, 20);

    let first_write = store.write_current(&first).expect("write first manifest");
    write_atomic(
        &dir.join(manifest_filename(2)),
        &encode_manifest(&second).unwrap(),
    )
    .expect("write unpointed second");
    write_atomic(&dir.join(MANIFEST_FILE), &encode_manifest(&second).unwrap())
        .expect("mirror unpointed second");

    assert_eq!(store.current_pointer().unwrap(), first_write.pointer);
    assert_eq!(store.load_current().unwrap(), first);

    let second_write = store.write_current(&second).expect("write second manifest");
    assert_eq!(store.current_pointer().unwrap(), second_write.pointer);
    assert_eq!(store.load_current().unwrap(), second);
    cleanup(dir);
}

#[test]
fn recovery_replays_wal_after_manifest_durable_seq() {
    let dir = test_dir("recover-after-manifest");
    write_manifest_assets(&dir);
    fs::create_dir_all(dir.join("wal")).expect("wal dir");
    let mut wal = Wal::open(dir.join("wal"), WalOptions::default()).expect("open wal");
    wal.append(b"already-in-manifest-1").expect("append 1");
    wal.append(b"already-in-manifest-2").expect("append 2");
    wal.append(b"after-manifest-3").expect("append 3");
    drop(wal);
    ManifestStore::open(&dir)
        .write_current(&manifest(1, 2))
        .expect("write manifest");

    let recovered = recover_vault(&dir).expect("recover vault");

    assert_eq!(recovered.manifest.durable_seq, 2);
    assert_eq!(recovered.last_recovered_seq, 3);
    assert_eq!(recovered.torn_tail, None);
    assert_eq!(recovered.wal_records.len(), 1);
    assert_eq!(recovered.wal_records[0].payload, b"after-manifest-3");
    cleanup(dir);
}

#[test]
fn recovery_discards_torn_tail_but_keeps_last_acked_bytes() {
    let dir = test_dir("recover-torn-tail");
    write_manifest_assets(&dir);
    fs::create_dir_all(dir.join("wal")).expect("wal dir");
    let mut wal = Wal::open(dir.join("wal"), WalOptions::default()).expect("open wal");
    let acked = wal.append(b"acked-after-manifest").expect("append acked");
    drop(wal);
    let mut file = OpenOptions::new()
        .append(true)
        .open(&acked.segment_path)
        .expect("open segment");
    file.write_all(b"CXW1partial").expect("write torn bytes");
    file.sync_data().expect("fsync torn bytes");
    drop(file);
    ManifestStore::open(&dir)
        .write_current(&manifest(1, 0))
        .expect("write manifest");

    let recovered = recover_vault(&dir).expect("recover vault");
    let tail = recovered.torn_tail.expect("torn tail");

    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(recovered.wal_records.len(), 1);
    assert_eq!(recovered.wal_records[0].payload, b"acked-after-manifest");
    assert_eq!(
        fs::metadata(&acked.segment_path).unwrap().len(),
        acked.end_offset
    );
    cleanup(dir);
}

#[test]
fn unknown_manifest_major_fails_closed() {
    let dir = test_dir("manifest-version");
    let mut manifest = manifest(1, 0);
    manifest.version.major = SUPPORTED_MANIFEST_MAJOR + 1;
    fs::create_dir_all(&dir).expect("manifest dir");
    let pointer = manifest_filename(1);
    fs::write(dir.join(&pointer), serde_json::to_vec(&manifest).unwrap()).expect("manifest file");
    fs::write(dir.join(CURRENT_FILE), pointer).expect("current pointer");

    let error = ManifestStore::open(&dir)
        .load_current()
        .expect_err("unsupported major rejected");

    assert_eq!(error.code, "CALYX_FORMAT_VERSION_UNSUPPORTED");
    cleanup(dir);
}

#[test]
fn invalid_mutable_refs_fail_closed() {
    let hash = "a".repeat(64);

    let absolute =
        ImmutableRef::new("/panel/current.json", hash.clone()).expect_err("absolute ref rejected");
    let mutable = ImmutableRef::new("CURRENT", hash).expect_err("control file rejected");

    assert_eq!(absolute.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert_eq!(mutable.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn manifest_ref_hash_mismatch_fails_closed() {
    let dir = test_dir("manifest-ref-corrupt");
    write_manifest_assets(&dir);
    ManifestStore::open(&dir)
        .write_current(&manifest(1, 1))
        .expect("write manifest");
    fs::write(dir.join("panel/panel-0001.json"), b"panel-corrupt").expect("corrupt panel ref");

    let error = ManifestStore::open(&dir)
        .load_current()
        .expect_err("corrupt manifest ref rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("manifest immutable ref"));
    cleanup(dir);
}

#[test]
fn quarantine_records_roundtrip_and_match_ranges() {
    let dir = test_dir("quarantine");
    write_manifest_assets(&dir);
    let store = ManifestStore::open(&dir);
    store
        .write_current(&manifest(1, 10))
        .expect("write manifest");
    let record = QuarantineRecord::new(5, 10, 7, 123).expect("quarantine record");

    let updated = store
        .append_quarantine(record.clone())
        .expect("append quarantine");
    let loaded = store.load_current().expect("load quarantined manifest");

    assert_eq!(updated.manifest_seq, 2);
    assert_eq!(loaded.quarantines, vec![record]);
    assert!(is_quarantined(&loaded, 5));
    assert!(is_quarantined(&loaded, 9));
    assert!(!is_quarantined(&loaded, 10));
    assert!(is_vault_seq_quarantined(&dir, 7).unwrap());
    cleanup(dir);
}

#[test]
fn corrupt_base_shard_read_fails_closed_with_restore_guidance() {
    let dir = test_dir("base-corrupt");
    let path = dir.join("cf").join("base").join("base.sst");
    fs::create_dir_all(path.parent().unwrap()).expect("base dir");
    let key = b"0123456789abcdef";
    let value = b"constellation-header";
    write_sst(&path, [(key.as_slice(), value.as_slice())]).expect("write base sst");
    assert_eq!(read_base_shard(&path, key).unwrap().unwrap(), value);

    let mut bytes = fs::read(&path).expect("read base sst");
    let value_at = bytes
        .windows(value.len())
        .position(|window| window == value)
        .expect("value position");
    bytes[value_at] ^= 0x01;
    fs::write(&path, bytes).expect("write corrupt base sst");

    let error = read_base_shard(&path, key).expect_err("corrupt base fails closed");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert_eq!(error.remediation, "restore from restic/snapshot");
    cleanup(dir);
}

fn manifest(manifest_seq: u64, durable_seq: u64) -> VaultManifest {
    VaultManifest::new(
        manifest_seq,
        durable_seq,
        ImmutableRef::from_bytes("panel/panel-0001.json", b"panel").unwrap(),
        vec![ImmutableRef::from_bytes("codebooks/slot_00.cb", b"codebook").unwrap()],
    )
    .unwrap()
}

fn write_manifest_assets(dir: &Path) {
    fs::create_dir_all(dir.join("panel")).expect("panel dir");
    fs::create_dir_all(dir.join("codebooks")).expect("codebook dir");
    fs::write(dir.join("panel/panel-0001.json"), b"panel").expect("panel bytes");
    fs::write(dir.join("codebooks/slot_00.cb"), b"codebook").expect("codebook bytes");
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-manifest-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
