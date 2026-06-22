use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME,
    Modality, SlotId, SlotVector, VaultId, VaultStore,
};
mod fsv_support;
use fsv_support::{fsv_root_os, reset_dir};

const CHUNK_ID: &str = "chunk:PH64/001 with spaces";
const DATABASE_NAME: &str = "leapable_db_contract__stage15";

#[test]
fn string_metadata_survives_base_row_codec_and_durable_readback() {
    let root = temp_root("unit");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let cx = constellation(&vault, b"issue601 unit source", CHUNK_ID, DATABASE_NAME);
    let key = base_key(cx.cx_id);

    assert_eq!(vault.read_cf_at(0, ColumnFamily::Base, &key).unwrap(), None);
    vault.put(cx.clone()).expect("put metadata constellation");
    vault.flush().expect("flush");

    let base = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &key)
        .expect("read base")
        .expect("base row");
    let decoded = encode::decode_constellation_base(&base).expect("decode base");
    let got = vault.get(cx.cx_id, vault.snapshot()).expect("get cx");

    assert_eq!(decoded.chunk_id(), Some(CHUNK_ID));
    assert_eq!(decoded.database_name(), Some(DATABASE_NAME));
    assert_eq!(got.metadata, cx.metadata);
    assert!(String::from_utf8_lossy(&base).contains(CHUNK_ID));

    let mut legacy = cx.clone();
    legacy.metadata.clear();
    let legacy_bytes = encode::encode_constellation_base(&legacy).expect("encode legacy");
    let legacy_without_metadata = &legacy_bytes[..legacy_bytes.len() - 4];
    let legacy_decoded =
        encode::decode_constellation_base(legacy_without_metadata).expect("decode old format");
    assert!(legacy_decoded.metadata.is_empty());

    let mut corrupt = base.clone();
    corrupt.extend_from_slice(&1_u32.to_be_bytes());
    let error = encode::decode_constellation_base(&corrupt).expect_err("corrupt row fails");
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");

    cleanup(&root);
}

#[test]
#[ignore = "manual FSV for issue #601 string metadata SoT readback"]
fn issue601_string_metadata_manual_fsv() {
    let root =
        fsv_root_os("CALYX_FSV_ROOT", "calyx-issue601-string-metadata-manual").join("issue601");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let cx = constellation(
        &vault,
        b"issue601 happy source bytes",
        CHUNK_ID,
        DATABASE_NAME,
    );
    let key = base_key(cx.cx_id);
    let before = vault.read_cf_at(0, ColumnFamily::Base, &key).unwrap();

    vault.put(cx.clone()).expect("put happy path");
    vault.flush().expect("flush happy path");

    let after = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &key)
        .expect("read after")
        .expect("base row after put");
    let decoded = encode::decode_constellation_base(&after).expect("decode happy base");
    let got = vault
        .get(cx.cx_id, vault.snapshot())
        .expect("get happy path");

    let empty_db = constellation(
        &vault,
        b"issue601 empty database name source",
        "chunk-empty-db",
        "",
    );
    let empty_key = base_key(empty_db.cx_id);
    let empty_before = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &empty_key)
        .unwrap();
    vault.put(empty_db.clone()).expect("put empty db name");
    vault.flush().expect("flush empty db name");
    let empty_after = vault
        .get(empty_db.cx_id, vault.snapshot())
        .expect("get empty db name");

    let mut changed = cx.clone();
    changed
        .metadata
        .insert(METADATA_CHUNK_ID.to_string(), "chunk:changed".to_string());
    let before_conflict_hash = blake3::hash(&after);
    let conflict_error = vault
        .put(changed)
        .expect_err("metadata conflict must fail closed");
    let after_conflict = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &key)
        .unwrap()
        .expect("base row after conflict");

    let mut legacy = cx.clone();
    legacy.metadata.clear();
    let legacy_bytes = encode::encode_constellation_base(&legacy).expect("encode legacy");
    let legacy_without_metadata = &legacy_bytes[..legacy_bytes.len() - 4];
    let legacy_decoded =
        encode::decode_constellation_base(legacy_without_metadata).expect("decode legacy");

    let mut corrupt = after.clone();
    corrupt.extend_from_slice(&1_u32.to_be_bytes());
    let corrupt_error =
        encode::decode_constellation_base(&corrupt).expect_err("trailing bytes must fail closed");

    let wal_path = vault_dir.join("wal/00000000000000000000.wal");
    let wal_bytes = fs::read(&wal_path).expect("read wal");
    let replay = calyx_aster::wal::replay_dir(vault_dir.join("wal")).expect("replay wal");
    let wal_rows = encode::decode_write_batch(&replay.records[0].payload).expect("decode wal");
    let base_row = wal_rows
        .iter()
        .find(|row| row.cf == ColumnFamily::Base && row.key == key)
        .expect("base row in wal");
    let sst_files = list_ssts(&vault_dir.join("cf/base"));

    let report = serde_json::json!({
        "issue": 601,
        "source_of_truth": {
            "vault_dir": vault_dir,
            "base_cf_key_hex": hex(&key),
            "wal_path": wal_path,
            "base_sst_files": sst_files,
        },
        "known_input": {
            "chunk_id": CHUNK_ID,
            "database_name": DATABASE_NAME,
            "expected_chunk_id_bytes_hex": hex(CHUNK_ID.as_bytes()),
            "expected_database_name_bytes_hex": hex(DATABASE_NAME.as_bytes()),
        },
        "happy_path": {
            "before_base_present": before.is_some(),
            "after_base_present": true,
            "decoded_chunk_id": decoded.chunk_id(),
            "decoded_database_name": decoded.database_name(),
            "vault_get_metadata": got.metadata.clone(),
            "base_contains_chunk_id": String::from_utf8_lossy(&after).contains(CHUNK_ID),
            "wal_base_contains_chunk_id": String::from_utf8_lossy(&base_row.value).contains(CHUNK_ID),
            "base_row_sha256": sha256_hex(&after),
            "wal_file_sha256": sha256_hex(&wal_bytes),
            "base_row_prefix_hex": hex(&after[..after.len().min(256)]),
        },
        "edges": {
            "empty_database_name": {
                "before_base_present": empty_before.is_some(),
                "after_database_name": empty_after.database_name(),
                "after_chunk_id": empty_after.chunk_id(),
            },
            "metadata_conflict_same_cxid": {
                "before_base_sha256": before_conflict_hash.to_hex().to_string(),
                "error_code": conflict_error.code,
                "after_base_sha256": blake3::hash(&after_conflict).to_hex().to_string(),
                "row_unchanged": after_conflict == after,
            },
            "legacy_no_metadata_bytes": {
                "before_trailing_metadata_count_removed": true,
                "after_metadata_empty": legacy_decoded.metadata.is_empty(),
            },
            "corrupt_trailing_bytes": {
                "error_code": corrupt_error.code,
            },
        },
    });
    let report_path = root.join("issue601-string-metadata-fsv.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    println!("ISSUE601_FSV_ROOT={}", root.display());
    println!("ISSUE601_FSV_REPORT={}", report_path.display());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    assert_eq!(before, None);
    assert_eq!(decoded.chunk_id(), Some(CHUNK_ID));
    assert_eq!(decoded.database_name(), Some(DATABASE_NAME));
    assert_eq!(got.metadata, cx.metadata);
    assert!(String::from_utf8_lossy(&after).contains(CHUNK_ID));
    assert!(String::from_utf8_lossy(&base_row.value).contains(CHUNK_ID));
    assert_eq!(empty_after.database_name(), Some(""));
    assert_eq!(conflict_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert_eq!(after_conflict, after);
    assert!(legacy_decoded.metadata.is_empty());
    assert_eq!(corrupt_error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

fn open_vault(path: &Path) -> AsterVault {
    AsterVault::new_durable(
        path,
        vault_id(),
        b"issue601-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn constellation(
    vault: &AsterVault,
    raw: &[u8],
    chunk_id: &str,
    database_name: &str,
) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    let mut metadata = BTreeMap::new();
    metadata.insert(METADATA_CHUNK_ID.to_string(), chunk_id.to_string());
    metadata.insert(
        METADATA_DATABASE_NAME.to_string(),
        database_name.to_string(),
    );
    Constellation {
        cx_id: vault.cx_id_for_input(raw, 64),
        vault_id: vault_id(),
        panel_version: 64,
        created_at: 1_786_000_601,
        input_ref: InputRef {
            hash: *blake3::hash(raw).as_bytes(),
            pointer: Some(format!("sqlite://chunks/{chunk_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-issue601-string-metadata-{name}-{}",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    fs::remove_dir_all(path).expect("cleanup");
}

fn list_ssts(path: &Path) -> Vec<String> {
    fs::read_dir(path)
        .expect("read base cf dir")
        .map(|entry| entry.expect("dir entry").path().display().to_string())
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
