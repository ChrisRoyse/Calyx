use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterMistakeStorage, CALYX_ASTER_CF_UNAVAILABLE, MistakeLog, MistakeStorage,
    decode_mistake_entry, mistake_seq_from_key,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, FixedClock, Result};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::vault_id;

#[test]
#[ignore = "manual FSV fixture for issue #406 anneal_mistakes bytes"]
fn issue406_manual_fsv_fixture_writes_anneal_mistakes_bytes() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE406_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let salt = b"issue406-mistake-log-fsv".to_vec();
    let vid = vault_id();

    let report = {
        let vault = AsterVault::new_durable(&vault_dir, vid, salt.clone(), VaultOptions::default())
            .expect("open durable vault");
        let storage = AsterMistakeStorage::new(&vault);
        let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(4_060_001));
        let log = MistakeLog::open(storage, 3, clock).expect("open mistake log");

        let before_rows = raw_rows(&vault);
        let empty_rate = log.mistake_rate(3).expect("empty rate");
        let zero_window = log.mistake_rate(0).expect_err("zero window rejects");
        let invalid_append = log
            .append(cx(0xEE), f64::NAN, 0.1, AnchorKind::Reward)
            .expect_err("nan rejects before write");
        let invalid_after_rows = raw_rows(&vault);

        let first = log
            .append(
                cx(0xA1),
                0.9,
                0.1,
                AnchorKind::Label("issue406-gold".to_string()),
            )
            .expect("append first");
        let second = log
            .append(cx(0xA2), 0.8, 0.7, AnchorKind::Reward)
            .expect("append second");
        let third = log
            .append(cx(0xA3), 0.5, 0.5, AnchorKind::Reward)
            .expect("append third");
        let rate_3 = log.mistake_rate(3).expect("rate after appends");
        let recent = log.recent(3).expect("recent after appends");
        let typed_readback = log.readback_recent(3).expect("typed readback");

        vault.flush().expect("flush WAL and CF memtables");
        let after_flush_rows = raw_rows(&vault);
        json!({
            "issue": 406,
            "vault": vault_dir.display().to_string(),
            "before_rows": before_rows,
            "empty_rate_window_3": empty_rate,
            "zero_window_error": zero_window.code,
            "invalid_append_error": invalid_append.code,
            "rows_after_invalid_append": invalid_after_rows,
            "append_refs": [first, second, third],
            "rate_window_3": rate_3,
            "recent_entries": recent,
            "typed_readback": typed_readback,
            "after_flush_rows": after_flush_rows,
            "sst_files": list_files(&vault_dir.join("cf").join("anneal_mistakes")),
            "wal_files": list_files(&vault_dir.join("wal")),
        })
    };

    let reopened =
        AsterVault::open(&vault_dir, vid, salt, VaultOptions::default()).expect("reopen vault");
    let storage_failure = storage_failure_sample();
    let report = json!({
        "durable_report": report,
        "reopened_rows": raw_rows(&reopened),
        "storage_failure": storage_failure,
    });
    let artifact = root.join("issue406-fsv-artifact.json");
    fs::write(
        &artifact,
        serde_json::to_vec_pretty(&report).expect("serialize FSV report"),
    )
    .expect("write FSV artifact");
    println!("ISSUE406_FSV_ROOT={}", root.display());
    println!("ISSUE406_FSV_ARTIFACT={}", artifact.display());
}

struct FailingStorage;

impl MistakeStorage for FailingStorage {
    fn put_new(&self, _seq: u64, _value: &[u8]) -> Result<()> {
        Err(CalyxError {
            code: CALYX_ASTER_CF_UNAVAILABLE,
            message: "injected issue406 FSV CF outage".to_string(),
            remediation: "restore anneal_mistakes CF availability",
        })
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(Vec::new())
    }
}

fn storage_failure_sample() -> Value {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(4_060_002));
    let log = MistakeLog::open(FailingStorage, 3, clock).expect("open failing storage log");
    let error = log
        .append(cx(0xF1), 1.0, 0.0, AnchorKind::Reward)
        .expect_err("failing storage returns CF error");
    json!({
        "error_code": error.code,
        "session_recent_after_failed_append": log.recent(1).expect("session recent"),
    })
}

fn raw_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealMistakes)
        .expect("scan anneal_mistakes")
        .into_iter()
        .map(|(key, value)| {
            let seq = mistake_seq_from_key(&key).expect("decode mistake seq");
            let entry = decode_mistake_entry(&value).expect("decode mistake entry");
            json!({
                "seq": seq,
                "key_hex": hex_bytes(&key),
                "value_hex": hex_bytes(&value),
                "value_len": value.len(),
                "entry": entry,
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
    std::env::var_os("CALYX_ISSUE406_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE406_FSV_ROOT to a fresh data/fsv-issue406-* path")
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}
