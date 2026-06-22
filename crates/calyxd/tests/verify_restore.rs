//! PH67 T03 integration tests — real vaults and real bytes, no mocks.
//!
//! Two fixture families:
//! - a REAL vault written through `AsterVault::put` (WAL + ledger hook), so the
//!   happy path exercises the exact bytes a restic restore would contain;
//! - hand-built vaults (SSTs written with the production codecs) whose ledger
//!   chain and tip hash are known ahead of time, so corruption is injected at
//!   exact byte offsets and the expected outcome is computed independently.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{ColumnFamily, anchor_key, base_key, ledger_key, slot_key};
use calyx_aster::sst::write_sst;
use calyx_aster::vault::encode::{
    WriteRow, encode_anchor, encode_constellation_base, encode_slot_vector, encode_write_batch,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{Wal, WalOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, LedgerEntry, SubjectId, encode as encode_ledger_entry};
use calyxd::verify::verify_restore;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

const ACTOR: &str = "fsv-543";

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-verify-restore-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn constellation(seed: u8) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_000_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 4,
                data: vec![f32::from(seed); 4],
            },
        )]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: ACTOR.to_string(),
            observed_at: 1_000_000,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

/// Real write path: 3 constellations through `AsterVault::put` (WAL + ledger
/// hook + SST flush), then the vault directory is verified like a restore.
fn seeded_real_vault(dir: &Path) {
    let vault = AsterVault::new_durable(dir, vault_id(), b"fsv-543-salt", VaultOptions::default())
        .expect("open durable vault");
    for seed in 1..=3 {
        vault.put(constellation(seed)).expect("put constellation");
    }
    vault.flush().expect("flush vault");
}

/// Deterministic ledger chain whose tip hash is recomputed independently in
/// `independent_chain_hashes` (the 2+2=4 discipline).
fn ledger_chain(count: u64) -> Vec<LedgerEntry> {
    let mut prev = [0u8; 32];
    let mut entries = Vec::with_capacity(count as usize);
    for seq in 0..count {
        let entry = LedgerEntry::new(
            seq,
            prev,
            EntryKind::Ingest,
            SubjectId::Cx(CxId::from_bytes([seq as u8 + 1; 16])),
            format!("payload-{seq}").into_bytes(),
            ActorId::Service(ACTOR.to_string()),
            1_000 + seq,
        );
        prev = entry.entry_hash;
        entries.push(entry);
    }
    entries
}

/// Independent reimplementation of the canonical entry-hash framing
/// (length-prefixed BLAKE3 over seq | prev | kind | tagged subject | payload |
/// tagged actor | ts) so the tip hash is cross-checked against a second
/// implementation, not the library hashing itself.
fn independent_chain_tip_hex(count: u64) -> String {
    fn frame(hasher: &mut blake3::Hasher, bytes: &[u8]) {
        hasher.update(&(bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    let mut prev = [0u8; 32];
    for seq in 0..count {
        let mut hasher = blake3::Hasher::new();
        frame(&mut hasher, &seq.to_be_bytes());
        frame(&mut hasher, &prev);
        frame(&mut hasher, &[0u8]); // EntryKind::Ingest wire code
        let mut subject = vec![0u8]; // SubjectId::Cx wire tag
        subject.extend_from_slice(&[seq as u8 + 1; 16]);
        frame(&mut hasher, &subject);
        frame(&mut hasher, format!("payload-{seq}").as_bytes());
        let mut actor = vec![1u8]; // ActorId::Service wire tag
        actor.extend_from_slice(&(ACTOR.len() as u64).to_be_bytes());
        actor.extend_from_slice(ACTOR.as_bytes());
        frame(&mut hasher, &actor);
        frame(&mut hasher, &(1_000 + seq).to_be_bytes());
        prev = *hasher.finalize().as_bytes();
    }
    prev.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_single_sst(dir: &Path, cf_name: &str, rows: &[(Vec<u8>, Vec<u8>)]) {
    let cf_dir = dir.join("cf").join(cf_name);
    fs::create_dir_all(&cf_dir).expect("create cf dir");
    let entries: Vec<(&[u8], &[u8])> = rows
        .iter()
        .map(|(key, value)| (key.as_slice(), value.as_slice()))
        .collect();
    write_sst(cf_dir.join("00000000000000000001.sst"), entries).expect("write sst");
}

fn write_ledger_sst(dir: &Path, encoded: &[(u64, Vec<u8>)]) {
    let rows: Vec<(Vec<u8>, Vec<u8>)> = encoded
        .iter()
        .map(|(seq, bytes)| (ledger_key(*seq), bytes.clone()))
        .collect();
    write_single_sst(dir, "ledger", &rows);
}

fn write_base_fixture(dir: &Path) {
    let cx = constellation(9);
    write_single_sst(
        dir,
        "base",
        &[(
            base_key(cx.cx_id),
            encode_constellation_base(&cx).expect("encode base"),
        )],
    );
    write_single_sst(
        dir,
        "slot_00",
        &[(
            slot_key(cx.cx_id),
            encode_slot_vector(&cx.slots[&SlotId::new(0)]).expect("encode slot"),
        )],
    );
    write_single_sst(
        dir,
        "anchors",
        &[(
            anchor_key(cx.cx_id, &cx.anchors[0].kind),
            encode_anchor(&cx.anchors[0]).expect("encode anchor"),
        )],
    );
}

/// Real WAL segment via the production writer, carrying one base row whose
/// bytes are identical to the SST row (a checkpointed write, the normal case).
fn write_wal_fixture(dir: &Path) {
    let cx = constellation(9);
    let payload = encode_write_batch(&[WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(cx.cx_id),
        value: encode_constellation_base(&cx).expect("encode base"),
    }])
    .expect("encode write batch");
    let mut wal = Wal::open(dir.join("wal"), WalOptions::default()).expect("open wal");
    wal.append(&payload).expect("append wal record");
}

fn handbuilt_vault(dir: &Path, chain_len: u64) -> Vec<(u64, Vec<u8>)> {
    let encoded: Vec<(u64, Vec<u8>)> = ledger_chain(chain_len)
        .iter()
        .map(|entry| (entry.seq, encode_ledger_entry(entry)))
        .collect();
    write_ledger_sst(dir, &encoded);
    write_base_fixture(dir);
    write_wal_fixture(dir);
    encoded
}

/// Manual FSV seeder: writes the 3-constellation vault to
/// `$CALYX_ISSUE543_FSV_ROOT/vault` and keeps it for byte-level inspection
/// with `calyx verify-restore` / `xxd` (issue #543 evidence).
#[test]
#[ignore = "manual FSV seeder for issue 543"]
fn fsv_seed_vault_for_manual_inspection() {
    let root = std::env::var("CALYX_ISSUE543_FSV_ROOT").expect("set CALYX_ISSUE543_FSV_ROOT");
    let dir = PathBuf::from(root).join("vault");
    fs::create_dir_all(&dir).expect("create fsv vault dir");
    seeded_real_vault(&dir);
    let report = verify_restore(&dir).expect("verify seeded vault");
    println!("fsv-seed report: {report:?}");
    assert!(report.success(), "seeded vault must verify");
}

#[test]
fn real_vault_round_trip_verifies_intact() {
    let dir = test_dir("real");
    seeded_real_vault(&dir);

    let report = verify_restore(&dir).expect("verify restored vault");

    println!("real-vault report: {report:?}");
    assert!(report.chain_intact, "chain must verify intact");
    assert_eq!(report.constellation_count, 3);
    assert_eq!(report.anchor_count, 3);
    assert!(report.ledger_entry_count >= 3);
    assert!(report.wal_bytes_present > 0, "WAL bytes must be present");
    assert_eq!(
        report.first_cx_id.as_deref(),
        Some("01".repeat(16).as_str())
    );
    assert_eq!(report.ledger_tip_hash.len(), 64);
    assert_eq!(report.error, None);
    assert!(report.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn handbuilt_vault_tip_hash_matches_independent_blake3() {
    let dir = test_dir("tiphash");
    handbuilt_vault(&dir, 5);

    let report = verify_restore(&dir).expect("verify handbuilt vault");

    println!("tip-hash report: {report:?}");
    assert!(report.chain_intact);
    assert_eq!(report.ledger_entry_count, 5);
    assert_eq!(report.ledger_tip_hash, independent_chain_tip_hex(5));
    assert!(report.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn flipped_prev_hash_byte_in_fifth_entry_breaks_chain() {
    let dir = test_dir("flip");
    let mut encoded = handbuilt_vault(&dir, 8);

    let before = verify_restore(&dir).expect("verify before corruption");
    println!("BEFORE byte flip: {before:?}");
    assert!(before.chain_intact, "fixture must verify before the flip");

    // Flip one byte of the 5th entry (seq 4): offset 8 is the first byte of
    // prev_hash in the canonical encoding.
    encoded[4].1[8] ^= 0x01;
    fs::remove_file(dir.join("cf/ledger/00000000000000000001.sst")).expect("remove sst");
    write_ledger_sst(&dir, &encoded);

    let after = verify_restore(&dir).expect("verify after corruption");
    println!("AFTER byte flip: {after:?}");
    assert!(!after.chain_intact);
    assert_eq!(
        after.error.as_deref(),
        Some("CALYX_LEDGER_CHAIN_BROKEN at seq=4")
    );
    assert!(!after.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn truncated_last_ledger_entry_reports_corrupt_at_break_seq() {
    let dir = test_dir("truncate");
    let mut encoded = handbuilt_vault(&dir, 3);

    let before = verify_restore(&dir).expect("verify before truncation");
    println!("BEFORE truncation: {before:?}");
    assert!(before.chain_intact);

    encoded[2].1.truncate(12);
    fs::remove_file(dir.join("cf/ledger/00000000000000000001.sst")).expect("remove sst");
    write_ledger_sst(&dir, &encoded);

    let after = verify_restore(&dir).expect("verify after truncation");
    println!("AFTER truncation: {after:?}");
    assert!(!after.chain_intact);
    let error = after.error.as_deref().expect("error code present");
    assert!(
        error.starts_with("CALYX_LEDGER_CORRUPT at seq=2"),
        "{error}"
    );
    assert!(!after.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn zero_constellations_fails_pass_predicate() {
    let dir = test_dir("nobase");
    let encoded: Vec<(u64, Vec<u8>)> = ledger_chain(2)
        .iter()
        .map(|entry| (entry.seq, encode_ledger_entry(entry)))
        .collect();
    write_ledger_sst(&dir, &encoded);
    // WAL carries the seq-0 ledger row with bytes IDENTICAL to the SST (a
    // checkpointed write), so the base CF stays genuinely empty.
    let payload = encode_write_batch(&[WriteRow {
        cf: ColumnFamily::Ledger,
        key: ledger_key(0),
        value: encoded[0].1.clone(),
    }])
    .expect("encode write batch");
    let mut wal = Wal::open(dir.join("wal"), WalOptions::default()).expect("open wal");
    wal.append(&payload).expect("append wal record");
    drop(wal);

    let report = verify_restore(&dir).expect("verify vault without base CF");

    println!("zero-constellation report: {report:?}");
    assert!(report.chain_intact, "chain itself is intact");
    assert_eq!(report.constellation_count, 0);
    assert_eq!(report.first_cx_id, None);
    assert!(!report.success());
    assert!(
        report
            .failure_reasons()
            .join("\n")
            .contains("constellation_count=0")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_wal_bytes_fails_pass_predicate() {
    let dir = test_dir("nowal");
    let encoded: Vec<(u64, Vec<u8>)> = ledger_chain(2)
        .iter()
        .map(|entry| (entry.seq, encode_ledger_entry(entry)))
        .collect();
    write_ledger_sst(&dir, &encoded);
    write_base_fixture(&dir);

    let report = verify_restore(&dir).expect("verify vault without WAL");

    println!("missing-WAL report: {report:?}");
    assert!(report.chain_intact);
    assert_eq!(report.wal_bytes_present, 0);
    assert!(!report.success());
    assert!(
        report
            .failure_reasons()
            .join("\n")
            .contains("wal_bytes_present=0")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn absent_optional_index_dirs_are_skipped_and_unreported() {
    let dir = test_dir("optdirs");
    handbuilt_vault(&dir, 2);

    let without = verify_restore(&dir).expect("verify without ann/kernel/guard");
    assert!(without.success());

    for optional in ["ann", "kernel", "guard"] {
        let opt_dir = dir.join(optional);
        fs::create_dir_all(&opt_dir).expect("create optional dir");
        fs::write(opt_dir.join("rebuildable.bin"), b"rebuildable").expect("write index stub");
    }
    let with = verify_restore(&dir).expect("verify with ann/kernel/guard");

    assert!(with.success());
    assert_eq!(without.constellation_count, with.constellation_count);
    assert_eq!(without.ledger_tip_hash, with.ledger_tip_hash);
    let json = serde_json::to_string(&with).expect("serialize report");
    for absent in ["ann", "kernel", "guard"] {
        assert!(
            !json.contains(&format!("\"{absent}\"")),
            "report must not mention {absent}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_vault_path_is_config_invalid() {
    let missing = std::env::temp_dir().join("calyx-verify-restore-missing-543-does-not-exist");
    let error = verify_restore(&missing).expect_err("missing path must fail");
    assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
    assert!(
        error
            .to_string()
            .contains("calyx-verify-restore-missing-543-does-not-exist"),
        "error must name the missing path: {error}"
    );
}

#[test]
fn empty_dir_without_aster_state_is_config_invalid() {
    let dir = test_dir("empty");
    let error = verify_restore(&dir).expect_err("empty dir must fail");
    assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
    assert!(
        error.to_string().contains("neither cf/ nor wal/"),
        "{error}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn divergent_sst_and_wal_ledger_row_fails_closed() {
    let dir = test_dir("diverge");
    let entries = ledger_chain(2);
    let encoded: Vec<(u64, Vec<u8>)> = entries
        .iter()
        .map(|entry| (entry.seq, encode_ledger_entry(entry)))
        .collect();
    write_ledger_sst(&dir, &encoded);
    write_base_fixture(&dir);

    // WAL carries DIFFERENT bytes for ledger seq 0 — append-only violation.
    let mut divergent = encoded[0].1.clone();
    let last = divergent.len() - 1;
    divergent[last] ^= 0x01;
    let payload = encode_write_batch(&[WriteRow {
        cf: ColumnFamily::Ledger,
        key: ledger_key(0),
        value: divergent,
    }])
    .expect("encode write batch");
    let mut wal = Wal::open(dir.join("wal"), WalOptions::default()).expect("open wal");
    wal.append(&payload).expect("append wal record");
    drop(wal);

    let report = verify_restore(&dir).expect("verify divergent vault");

    println!("divergent report: {report:?}");
    assert!(!report.chain_intact);
    let error = report.error.as_deref().expect("error present");
    assert!(error.contains("CALYX_LEDGER_CORRUPT"), "{error}");
    assert!(error.contains("divergent"), "{error}");
    assert!(!report.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn report_serializes_every_contract_field() {
    let dir = test_dir("shape");
    handbuilt_vault(&dir, 2);
    let report = verify_restore(&dir).expect("verify");
    let json = serde_json::to_value(&report).expect("serialize");
    let object = json.as_object().expect("object");
    for field in [
        "vault_path",
        "constellation_count",
        "anchor_count",
        "ledger_entry_count",
        "ledger_tip_hash",
        "chain_intact",
        "wal_bytes_present",
        "first_cx_id",
        "error",
    ] {
        assert!(object.contains_key(field), "missing report field {field}");
    }
    assert_eq!(
        object.len(),
        9,
        "report must hold exactly the contract fields"
    );
    let _ = fs::remove_dir_all(&dir);
}
