//! Full State Verification for `as_of(t)` MVCC time-travel (PH72 T04).
//!
//! Sources of truth: (1) the `time_index` CF rows physically on disk
//! (`big_endian_u64(millis) || big_endian_u64(seqno)`); (2) the constellation
//! bytes returned by `as_of` at two different timestamps. Every check re-reads
//! the SoT independently of the call's return value. Run with `--nocapture` to
//! emit the evidence log.

use calyx_aster::timetravel::read_all;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use std::collections::BTreeMap;
use std::fs;

mod fsv_support;
use fsv_support::{named_fsv_root, reset_dir};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn constellation(vault: &AsterVault<impl Clock>, input: &[u8], tag: f32) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len().min(32)].copy_from_slice(&input[..input.len().min(32)]);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![tag, tag + 1.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 10,
        input_ref: InputRef {
            hash: input_hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [7; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

#[test]
fn timetravel_as_of_fsv() {
    let (root, keep) = named_fsv_root("CALYX_ASTER_TIMETRAVEL_FSV_ROOT", "timetravel-fsv");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"timetravel-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    println!("\n==================== TIMETRAVEL as_of FSV ====================");
    println!("SoT 1: time_index CF under {}", vault_dir.display());

    // ---- Trigger X: ingest C1, then (after the clock advances) C2. ---------
    let c1 = vault
        .put(constellation(&vault, b"c1", 1.0))
        .expect("ingest c1");
    let s1 = vault.latest_seq();
    std::thread::sleep(std::time::Duration::from_millis(3)); // distinct wall-clock millis
    let c2 = vault
        .put(constellation(&vault, b"c2", 2.0))
        .expect("ingest c2");
    let s2 = vault.latest_seq();
    vault.flush().expect("flush to disk");

    // ---- SoT 1: read the time-index back; 2+2=4 on the seqno mapping. ------
    let entries = read_all(&vault).expect("read time index");
    println!("\n[SoT 1: time_index entries]");
    for e in &entries {
        println!("  millis={} -> seqno={}", e.millis, e.seqno);
    }
    assert_eq!(entries.len(), 2, "one entry per ingest commit");
    assert!(
        entries[0].millis < entries[1].millis,
        "millis strictly ascending"
    );
    // The time-index entry seqnos must equal the seqs the ingests committed at.
    assert_eq!(entries[0].seqno, s1, "C1 time-index seqno == C1 commit seq");
    assert_eq!(entries[1].seqno, s2, "C2 time-index seqno == C2 commit seq");
    let (t1, t2) = (entries[0].millis, entries[1].millis);

    // ---- On-disk byte proof: big-endian (millis||seqno) keys in the SST. ---
    let cf_dir = vault_dir.join("cf").join("time_index");
    let mut found_key = false;
    for entry in fs::read_dir(&cf_dir).expect("time_index cf dir") {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).expect("read sst");
        let needle = {
            let mut k = t1.to_be_bytes().to_vec();
            k.extend_from_slice(&s1.to_be_bytes());
            k
        };
        if bytes.windows(needle.len()).any(|w| w == needle.as_slice()) {
            println!(
                "\n[on-disk] {} contains C1 key (millis||seqno) = {}",
                path.file_name().unwrap().to_string_lossy(),
                hex(&needle)
            );
            found_key = true;
        }
    }
    assert!(found_key, "C1's big-endian time-index key must be on disk");

    // ---- SoT 2: as_of at t1 sees only C1; at t2 sees both. -----------------
    println!("\n[SoT 2: as_of byte reads]");
    let at_t1 = vault.as_of(t1).expect("as_of t1");
    println!("  as_of({t1}) -> seqno {}", at_t1.seqno());
    assert!(at_t1.get_cx(c1).is_ok(), "C1 present at t1");
    let c2_missing = at_t1.get_cx(c2).unwrap_err();
    println!("  as_of({t1}).get_cx(C2) -> {} (absent)", c2_missing.code);
    assert!(at_t1.get_cx(c2).is_err(), "C2 absent at t1");

    let at_t2 = vault.as_of(t2).expect("as_of t2");
    assert!(
        at_t2.get_cx(c1).is_ok() && at_t2.get_cx(c2).is_ok(),
        "both present at t2"
    );
    println!("  as_of({t2}) -> both C1 and C2 present");

    // ---- Edge cases (before/after the SoT boundary). ----------------------
    println!("\n[EDGE cases]");
    let before = vault.as_of(0).unwrap_err();
    println!("  as_of(0) -> {} (no data before first write)", before.code);
    assert_eq!(before.code, "CALYX_TIMETRAVEL_NO_DATA");
    let just_before = vault.as_of(t1 - 1).unwrap_err();
    println!("  as_of(t1-1) -> {} (no entry <= t1-1)", just_before.code);
    assert_eq!(just_before.code, "CALYX_TIMETRAVEL_NO_DATA");
    // exactly t1 resolves to C1's seq.
    assert_eq!(vault.as_of(t1).unwrap().seqno(), s1);

    println!("\n==================== FSV PASS ====================\n");
    if keep {
        println!("timetravel_fsv_root={}", root.display());
    } else {
        let _ = fs::remove_dir_all(&root);
    }
}
