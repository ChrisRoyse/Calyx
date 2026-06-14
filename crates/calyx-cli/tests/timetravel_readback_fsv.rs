//! Full State Verification for the `readback time-index` and `readback as-of`
//! CLI commands (PH72 T04 follow-up, issue #689).
//!
//! Source of truth: the `time_index` column family physically on disk
//! (`big_endian_u64(millis) || big_endian_u64(seqno)` keys). The test ingests
//! three synthetic constellations at three distinct wall-clock instants through
//! the real `calyx_aster` write path, flushes to disk, then drives the actual
//! compiled `calyx` binary as a subprocess. Every assertion re-reads the SoT
//! (the on-disk CF bytes, independently of the command's JSON) and the
//! deterministic happy-path / boundary properties:
//!
//!   * `time-index` reports exactly one entry per commit, in ascending order,
//!     and every reported `(millis, seqno)` key is byte-present in the CF SST.
//!   * `as-of(t)` sees exactly the constellations committed at or before `t`
//!     (the MVCC prefix property), and fails closed with
//!     `CALYX_TIMETRAVEL_NO_DATA` before the first write.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    VaultId, VaultStore,
};
use serde_json::Value;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

/// A one-slot constellation whose dense vector encodes `tag`, so its identity is
/// a deterministic function of `input` (`cx_id_for_input`) — known input, known
/// expected cx_id.
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

/// Independently reads the raw `time_index` SST bytes and asserts the 16-byte
/// big-endian `(millis||seqno)` key is physically present — the SoT proof.
fn key_on_disk(vault_dir: &Path, millis: u64, seqno: u64) -> bool {
    let needle: Vec<u8> = millis
        .to_be_bytes()
        .iter()
        .chain(seqno.to_be_bytes().iter())
        .copied()
        .collect();
    let cf_dir = vault_dir.join("cf").join("time_index");
    let Ok(entries) = std::fs::read_dir(&cf_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("read time_index sst");
        if bytes.windows(needle.len()).any(|w| w == needle.as_slice()) {
            return true;
        }
    }
    false
}

fn calyx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
}

/// Runs `calyx <args...>`, asserting success, and returns parsed stdout JSON.
fn run_json(args: &[&str]) -> Value {
    let out = calyx().args(args).output().expect("spawn calyx");
    assert!(
        out.status.success(),
        "calyx {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("calyx {args:?} stdout not JSON ({e}): {:?}", out.stdout))
}

#[test]
fn timetravel_readback_cli_fsv() {
    let root = std::env::temp_dir().join(format!("calyx-tt-readback-fsv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let vault_dir = root.join("vault");

    // ---- Trigger X: three ingests at three distinct wall-clock instants. ---
    // System clock + a short sleep between commits guarantees strictly
    // ascending, distinct `millis` so the prefix property is observable.
    let inputs: [(&[u8], f32); 3] = [(b"alpha", 1.0), (b"bravo", 2.0), (b"charlie", 3.0)];
    let expected_ids: Vec<CxId> = {
        let vault = AsterVault::new_durable(
            &vault_dir,
            vault_id(),
            b"calyx-timetravel-readback".to_vec(),
            VaultOptions::default(),
        )
        .expect("open durable vault");
        let mut ids = Vec::new();
        for (input, tag) in inputs {
            let cx = constellation(&vault, input, tag);
            ids.push(cx.cx_id);
            vault.put(cx).expect("ingest");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        vault.flush().expect("flush to disk");
        ids
        // vault dropped here, releasing the directory for the subprocess.
    };

    // ==================== readback time-index ====================
    let ti = run_json(&[
        "readback",
        "time-index",
        "--vault",
        vault_dir.to_str().unwrap(),
    ]);
    let entries = ti["entries"].as_array().expect("entries array");
    println!("[SoT] time-index entries: {entries:?}");

    // (1) exactly one entry per commit.
    assert_eq!(ti["entry_count"].as_u64(), Some(3), "one entry per ingest");
    assert_eq!(entries.len(), 3);

    // (2) strictly ascending millis + (3) every reported key physically on disk.
    let mut prev_millis = 0u64;
    let mut pairs = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let millis = e["millis"].as_u64().expect("millis");
        let seqno = e["seqno"].as_u64().expect("seqno");
        if i > 0 {
            assert!(millis > prev_millis, "millis strictly ascending");
        }
        prev_millis = millis;
        assert!(
            key_on_disk(&vault_dir, millis, seqno),
            "reported key (millis={millis}, seqno={seqno}) must be byte-present in the time_index CF"
        );
        pairs.push((millis, seqno));
    }
    // seqnos are the three committed sequences in order.
    assert_eq!(
        pairs.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "seqnos are the committed sequence numbers"
    );

    let (m1, m2, m3) = (pairs[0].0, pairs[1].0, pairs[2].0);

    // ==================== readback as-of (prefix property) ====================
    let vault_arg = vault_dir.to_str().unwrap();
    let count_at = |t: u64| -> (usize, Vec<String>) {
        let v = run_json(&[
            "readback",
            "as-of",
            "--vault",
            vault_arg,
            "--t-millis",
            &t.to_string(),
        ]);
        let ids: Vec<String> = v["constellations"]
            .as_array()
            .expect("constellations")
            .iter()
            .map(|c| c["cx_id"].as_str().expect("cx_id").to_string())
            .collect();
        (v["constellation_count"].as_u64().unwrap() as usize, ids)
    };

    // happy path: as-of the last commit sees all three (and the exact cx_ids).
    let (n3, ids3) = count_at(m3);
    println!("[SoT] as-of({m3}) -> {n3} constellations: {ids3:?}");
    assert_eq!(n3, 3, "as-of(m3) sees all three");
    for id in &expected_ids {
        assert!(ids3.contains(&id.to_string()), "cx {id} present at m3");
    }

    // boundary: as-of m1 sees exactly the first; as-of m2 sees exactly two.
    let (n1, ids1) = count_at(m1);
    println!("[SoT] as-of({m1}) -> {n1} constellations: {ids1:?}");
    assert_eq!(n1, 1, "as-of(m1) sees exactly the first constellation");
    assert_eq!(ids1, vec![expected_ids[0].to_string()], "first cx is alpha");

    let (n2, _) = count_at(m2);
    println!("[SoT] as-of({m2}) -> {n2} constellations");
    assert_eq!(n2, 2, "as-of(m2) sees exactly the first two");

    // edge: far-future timestamp still sees all three (no over-read).
    let (nmax, _) = count_at(u64::MAX);
    assert_eq!(nmax, 3, "as-of(u64::MAX) sees all committed constellations");

    // ==================== edge: before the first write (fail closed) =========
    let out = calyx()
        .args([
            "readback",
            "as-of",
            "--vault",
            vault_arg,
            "--t-millis",
            &(m1 - 1).to_string(),
        ])
        .output()
        .expect("spawn calyx");
    assert!(!out.status.success(), "as-of before first write must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("[edge] as-of({}) -> {}", m1 - 1, stderr.trim());
    assert!(
        stderr.contains("CALYX_TIMETRAVEL_NO_DATA"),
        "must fail closed with CALYX_TIMETRAVEL_NO_DATA, got: {stderr}"
    );

    // ==================== edge: corrupt time_index fails closed =============
    {
        let vault = AsterVault::open(
            &vault_dir,
            vault_id(),
            b"calyx-timetravel-readback".to_vec(),
            VaultOptions::default(),
        )
        .expect("reopen vault to inject corrupt time_index row");
        vault
            .write_cf(ColumnFamily::TimeIndex, vec![0_u8; 17], vec![0])
            .expect("inject corrupt time_index key");
        vault.flush().expect("flush corrupt key");
    }
    let corrupt_index = calyx()
        .args(["readback", "time-index", "--vault", vault_arg])
        .output()
        .expect("spawn calyx");
    assert!(
        !corrupt_index.status.success(),
        "corrupt time-index readback must fail"
    );
    let corrupt_index_stderr = String::from_utf8_lossy(&corrupt_index.stderr);
    assert!(
        corrupt_index_stderr.contains("CALYX_ASTER_CORRUPT_SHARD"),
        "time-index readback must expose corrupt shard, got: {corrupt_index_stderr}"
    );

    let corrupt_asof = calyx()
        .args([
            "readback",
            "as-of",
            "--vault",
            vault_arg,
            "--t-millis",
            &m3.to_string(),
        ])
        .output()
        .expect("spawn calyx");
    assert!(
        !corrupt_asof.status.success(),
        "corrupt as-of readback must fail"
    );
    let corrupt_asof_stderr = String::from_utf8_lossy(&corrupt_asof.stderr);
    assert!(
        corrupt_asof_stderr.contains("CALYX_ASTER_CORRUPT_SHARD"),
        "as-of readback must expose corrupt shard, got: {corrupt_asof_stderr}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
