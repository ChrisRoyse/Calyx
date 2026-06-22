use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterMistakeStorage, AsterReplayStorage, CALYX_ANNEAL_REPLAY_INVALID_ROW,
    CALYX_ASTER_CF_UNAVAILABLE, DEFAULT_REPLAY_CAPACITY, MistakeLog, MistakeRef, ReplayBuffer,
    ReplayEntry, ReplayStorage, decode_replay_snapshot,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, FixedClock, Result};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::vault_id;

#[test]
#[ignore = "manual FSV fixture for issue #407 anneal_replay bytes"]
fn issue407_manual_fsv_fixture_writes_anneal_replay_bytes() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE407_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let salt = b"issue407-replay-buffer-fsv".to_vec();
    let vid = vault_id();

    let report = {
        let vault = AsterVault::new_durable(&vault_dir, vid, salt.clone(), VaultOptions::default())
            .expect("open durable vault");
        let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(4_070_001));
        let log = MistakeLog::open(AsterMistakeStorage::new(&vault), 10, clock.clone())
            .expect("open mistake log");
        let mut buffer =
            ReplayBuffer::open(AsterReplayStorage::new(&vault), 10, clock).expect("open replay");

        let before_rows = raw_rows(&vault);
        let empty_sample = buffer.sample_batch(3, 42);
        let invalid_capacity = ReplayBuffer::open(
            AsterReplayStorage::new(&vault),
            0,
            Arc::new(FixedClock::new(4_070_002)),
        )
        .err()
        .expect("zero capacity rejects");
        let invalid_entry = buffer
            .push(ReplayEntry {
                cx_id: cx(0xEE),
                surprise: f64::NAN,
                mistake_ref: MistakeRef {
                    seq: 1,
                    surprise: f64::NAN,
                },
                added_ts: 4_070_003,
            })
            .expect_err("nan replay entry rejects");
        let after_invalid_rows = raw_rows(&vault);

        log.append(cx(0xA1), 0.9, 0.1, AnchorKind::Reward)
            .expect("append high mistake");
        log.append(cx(0xA2), 0.7, 0.3, AnchorKind::Reward)
            .expect("append medium mistake");
        log.append(cx(0xA3), 0.5, 0.5, AnchorKind::Reward)
            .expect("append zero mistake");
        let log_rows_before_seed = log.readback_recent(10).expect("log rows before seed").len();
        let accepted_from_log = buffer.seed_from_log(&log, 3).expect("seed from log");
        let log_rows_after_seed = log.readback_recent(10).expect("log rows after seed").len();

        let stream_accepts = push_deterministic_stream(&mut buffer);
        let state_after_stream = buffer.snapshot();
        let sample_a = buffer.sample_batch(10, 42);
        let sample_b = buffer.sample_batch(10, 42);
        let low = buffer
            .push(entry(0xF0, 0.0, 300))
            .expect("low surprise discard");
        let min_surprise = buffer
            .entries_by_priority()
            .last()
            .map(|entry| entry.surprise)
            .expect("buffer has rows");
        let equal = buffer
            .push(entry(0xF1, min_surprise, 301))
            .expect("equal surprise discard");
        let high = buffer
            .push(entry(0xF2, 1.0, 302))
            .expect("high surprise replaces min");
        let final_state = buffer.snapshot();

        vault.flush().expect("flush WAL and CF memtables");
        json!({
            "issue": 407,
            "vault": vault_dir.display().to_string(),
            "source_of_truth": "Aster anneal_replay CF snapshot plus WAL/SST under vault/",
            "default_capacity": DEFAULT_REPLAY_CAPACITY,
            "before_rows": before_rows,
            "empty_sample": empty_sample,
            "invalid_capacity_error": invalid_capacity.code,
            "invalid_entry_error": invalid_entry.code,
            "expected_invalid_entry_error": CALYX_ANNEAL_REPLAY_INVALID_ROW,
            "rows_after_invalid_entry": after_invalid_rows,
            "accepted_from_log": accepted_from_log,
            "log_rows_before_seed": log_rows_before_seed,
            "log_rows_after_seed": log_rows_after_seed,
            "stream_accepts": stream_accepts,
            "state_after_stream": state_after_stream,
            "sample_seed42_first": sample_a,
            "sample_seed42_second": sample_b,
            "low_surprise_push_accepted": low,
            "equal_min_surprise_push_accepted": equal,
            "high_surprise_push_accepted": high,
            "final_state": final_state,
            "after_flush_rows": raw_rows(&vault),
            "sst_files": list_files(&vault_dir.join("cf").join("anneal_replay")),
            "wal_files": list_files(&vault_dir.join("wal")),
        })
    };

    let reopened =
        AsterVault::open(&vault_dir, vid, salt, VaultOptions::default()).expect("reopen vault");
    let reopened_buffer = ReplayBuffer::open(
        AsterReplayStorage::new(&reopened),
        10,
        Arc::new(FixedClock::new(4_070_004)),
    )
    .expect("reopen replay buffer");
    let report = json!({
        "durable_report": report,
        "reopened_rows": raw_rows(&reopened),
        "reopened_state": reopened_buffer.snapshot(),
        "storage_failure": storage_failure_sample(),
    });
    let artifact = root.join("issue407-fsv-artifact.json");
    fs::write(
        &artifact,
        serde_json::to_vec_pretty(&report).expect("serialize FSV report"),
    )
    .expect("write FSV artifact");
    println!("ISSUE407_FSV_ROOT={}", root.display());
    println!("ISSUE407_FSV_ARTIFACT={}", artifact.display());
}

struct FailingStorage;

impl ReplayStorage for FailingStorage {
    fn load_snapshot(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn save_snapshot(&self, _value: &[u8]) -> Result<()> {
        Err(CalyxError {
            code: CALYX_ASTER_CF_UNAVAILABLE,
            message: "injected issue407 FSV CF outage".to_string(),
            remediation: "restore anneal_replay CF availability",
        })
    }
}

fn storage_failure_sample() -> Value {
    let mut buffer = ReplayBuffer::open(FailingStorage, 2, Arc::new(FixedClock::new(4_070_005)))
        .expect("open failing replay buffer");
    let error = buffer
        .push(entry(0xFA, 0.9, 1))
        .expect_err("failing storage returns CF error");
    json!({
        "error_code": error.code,
        "len_after_failed_push": buffer.len(),
        "entries_after_failed_push": buffer.entries_by_priority(),
    })
}

fn push_deterministic_stream<S: ReplayStorage>(buffer: &mut ReplayBuffer<S>) -> usize {
    let mut accepted = 0;
    for seq in 4..=100 {
        let surprise = ((seq * 37) % 100) as f64 / 100.0;
        if buffer
            .push(entry((seq % 255) as u8, surprise, seq as u64))
            .expect("push deterministic replay entry")
        {
            accepted += 1;
        }
    }
    accepted
}

fn raw_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReplay)
        .expect("scan anneal_replay")
        .into_iter()
        .map(|(key, value)| {
            let snapshot = decode_replay_snapshot(&value).expect("decode replay snapshot");
            json!({
                "key_hex": hex_bytes(&key),
                "value_hex": hex_bytes(&value),
                "value_len": value.len(),
                "snapshot": snapshot,
            })
        })
        .collect::<Vec<_>>()
}

fn list_files(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut files = fs::read_dir(dir)
        .expect("read artifact dir")
        .map(|entry| entry.expect("dir entry").path().display().to_string())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn hex_bytes(bytes: &[u8]) -> String {
    let digits = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(digits[(byte >> 4) as usize] as char);
        out.push(digits[(byte & 0x0f) as usize] as char);
    }
    out
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE407_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE407_FSV_ROOT to a fresh data/fsv-issue407-* path")
}

fn entry(byte: u8, surprise: f64, seq: u64) -> ReplayEntry {
    ReplayEntry::new(
        cx(byte),
        surprise,
        MistakeRef { seq, surprise },
        4_070_100 + seq,
    )
    .unwrap()
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}
