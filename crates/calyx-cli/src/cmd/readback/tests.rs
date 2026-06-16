use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn parse_claims_new_readback_forms_only() {
    assert!(try_run(&tokens(["readback", "--hex", "x"])).is_some());
    assert!(try_run(&tokens(["readback", "--vault-tree", "v"])).is_some());
    assert!(
        try_run(&tokens([
            "readback", "--cf-row", "v", "--cf", "base", "--key", "00"
        ]))
        .is_some()
    );
    assert!(try_run(&tokens(["readback", "--wal", "00000000000000000000.wal"])).is_some());
    assert!(try_run(&tokens(["readback", "--ledger", "v", "--seq", "1"])).is_some());
    assert!(try_run(&tokens(["readback", "--wal", "--vault", "v"])).is_none());
    assert!(try_run(&tokens(["readback", "time-index", "--vault", "v"])).is_none());
}

#[test]
fn hex_key_parser_accepts_uppercase_and_rejects_bad_width() {
    assert_eq!(parse_hex_bytes("00Af", "--key").unwrap(), vec![0x00, 0xaf]);

    let odd = parse_hex_bytes("abc", "--key").unwrap_err();
    assert_eq!(odd.code(), "CALYX_CLI_USAGE_ERROR");
    let bad = parse_hex_bytes("zz", "--key").unwrap_err();
    assert_eq!(bad.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn wal_events_reads_records_and_torn_payload_tail() {
    let dir = temp_dir("wal-torn-tail");
    let path = dir.join("00000000000000000000.wal");
    let mut bytes = wal_record(1, b"alpha");
    let mut torn = wal_record(2, b"bravo");
    torn.truncate(torn.len() - 2);
    bytes.extend_from_slice(&torn);
    fs::write(&path, bytes).unwrap();

    let events = wal_events(&path).unwrap();

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        WalEvent::Record(WalRecord { seq: 1, payload, .. }) if payload == b"alpha"
    ));
    assert_eq!(events[1], WalEvent::TornTail { seq: Some(2) });
    cleanup(dir);
}

#[test]
fn wal_events_zero_length_file_is_empty() {
    let dir = temp_dir("wal-empty");
    let path = dir.join("00000000000000000000.wal");
    fs::write(&path, []).unwrap();

    assert!(wal_events(&path).unwrap().is_empty());
    cleanup(dir);
}

#[test]
fn wal_events_bad_crc_fails_closed() {
    let dir = temp_dir("wal-bad-crc");
    let path = dir.join("00000000000000000000.wal");
    let mut bytes = wal_record(7, b"corrupt-me");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&path, bytes).unwrap();

    let error = wal_events(&path).unwrap_err();

    assert_eq!(error.code(), "CALYX_ASTER_TORN_WAL");
    cleanup(dir);
}

fn wal_record(seq: u64, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let crc = wal_payload_crc(seq, len, payload);
    let mut bytes = Vec::with_capacity(WAL_HEADER_LEN + payload.len());
    bytes.extend_from_slice(&WAL_MAGIC.to_le_bytes());
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-cli-readback-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
