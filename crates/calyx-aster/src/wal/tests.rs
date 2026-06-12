use super::record;
use super::*;
use proptest::prelude::*;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn append_and_replay_roundtrips_payload_bytes() {
    let dir = test_dir("roundtrip");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");

    let first = wal.append(b"acked-one").expect("append first");
    let second = wal.append(b"acked-two").expect("append second");
    drop(wal);

    let replay = replay_dir(&dir).expect("replay wal");
    assert_eq!(replay.torn_tail, None);
    assert_eq!(replay.records.len(), 2);
    assert_eq!(replay.records[0].seq, first.seq);
    assert_eq!(replay.records[0].payload, b"acked-one");
    assert_eq!(replay.records[1].seq, second.seq);
    assert_eq!(replay.records[1].payload, b"acked-two");

    let bytes = fs::read(&first.segment_path).expect("read segment bytes");
    assert_eq!(&bytes[0..4], &record::MAGIC.to_le_bytes());
    cleanup(dir);
}

#[test]
fn append_batch_assigns_ordered_sequences_and_one_segment() {
    let dir = test_dir("batch");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");

    let acks = wal
        .append_batch(&[
            b"first".as_slice(),
            b"second".as_slice(),
            b"third".as_slice(),
        ])
        .expect("append batch");
    drop(wal);

    assert_eq!(
        acks.iter().map(|ack| ack.seq).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(
        acks.windows(2)
            .all(|pair| pair[0].end_offset == pair[1].start_offset)
    );
    let replay = replay_dir(&dir).expect("replay wal");
    assert_eq!(replay.records.len(), 3);
    assert!(
        replay
            .records
            .iter()
            .all(|record| record.segment_path == acks[0].segment_path)
    );
    cleanup(dir);
}

#[test]
fn segment_rotates_before_crossing_limit() {
    let dir = test_dir("rotate");
    let options = WalOptions {
        max_segment_bytes: 56,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");

    let first = wal.append(b"record-one").expect("append first");
    let second = wal.append(b"record-two").expect("append second");
    drop(wal);

    assert_ne!(first.segment_path, second.segment_path);
    let replay = replay_dir(&dir).expect("replay wal");
    assert_eq!(replay.records.len(), 2);
    cleanup(dir);
}

#[test]
fn torn_tail_is_truncated_and_reported_with_catalog_code() {
    let dir = test_dir("torn");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    let acked = wal.append(b"acked").expect("append acked");
    drop(wal);

    let torn = record::encode(acked.seq + 1, b"unacked").expect("encode torn record");
    let mut file = OpenOptions::new()
        .append(true)
        .open(&acked.segment_path)
        .expect("open segment for torn write");
    file.write_all(&torn[..record::HEADER_LEN + 2])
        .expect("write partial record");
    file.sync_data().expect("fsync partial");
    drop(file);

    let replay = replay_dir(&dir).expect("replay torn wal");
    let tail = replay.torn_tail.expect("torn tail reported");
    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert!(tail.error().to_string().contains("CALYX_ASTER_TORN_WAL"));
    assert_eq!(replay.records.len(), 1);
    assert_eq!(replay.records[0].payload, b"acked");
    assert_eq!(
        fs::metadata(&acked.segment_path)
            .expect("segment metadata")
            .len(),
        acked.end_offset
    );
    cleanup(dir);
}

#[test]
fn replay_waits_for_append_lock_before_truncating_torn_tail() {
    let fsv_root = std::env::var_os("CALYX_WAL_RECOVERY_LOCK_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("replay-append-lock"),
        |root| {
            let _ = fs::remove_dir_all(root);
            fs::create_dir_all(root).expect("create fsv root");
            let dir = root.join("wal-replay-lock").join("wal");
            fs::create_dir_all(&dir).expect("create fsv wal dir");
            dir
        },
    );
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    let first = wal.append(b"acked").expect("append acked");
    drop(wal);

    let next = record::encode(first.seq + 1, b"completed").expect("encode next record");
    let append_guard = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))
        .expect("hold append lock");
    let mut file = OpenOptions::new()
        .append(true)
        .open(&first.segment_path)
        .expect("open segment for in-flight append");
    file.write_all(&next[..record::HEADER_LEN + 2])
        .expect("write partial next record");
    file.sync_data().expect("fsync partial next record");
    drop(file);
    let partial_len = fs::metadata(&first.segment_path)
        .expect("partial metadata")
        .len();

    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let replay_dir_path = dir.clone();
    let handle = std::thread::spawn(move || {
        attempt_tx.send(()).expect("send attempt");
        done_tx
            .send(replay_dir(&replay_dir_path).expect("replay after append completes"))
            .expect("send replay");
    });
    attempt_rx.recv().expect("replay thread attempted");
    assert!(
        done_rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "replay completed while append lock was held"
    );
    let locked_len = fs::metadata(&first.segment_path)
        .expect("locked metadata")
        .len();
    assert_eq!(locked_len, partial_len);

    let mut file = OpenOptions::new()
        .append(true)
        .open(&first.segment_path)
        .expect("reopen segment to finish append");
    file.write_all(&next[record::HEADER_LEN + 2..])
        .expect("finish next record");
    file.sync_data().expect("fsync completed next record");
    drop(file);
    drop(append_guard);

    let replay = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("replay finished after lock release");
    handle.join().expect("join replay thread");

    assert_eq!(replay.torn_tail, None);
    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| record.payload.as_slice())
            .collect::<Vec<_>>(),
        vec![b"acked".as_slice(), b"completed".as_slice()]
    );
    assert_eq!(
        fs::metadata(&first.segment_path)
            .expect("completed metadata")
            .len(),
        first.end_offset + next.len() as u64
    );
    if let Some(root) = fsv_root {
        let segment_bytes = fs::read(&first.segment_path).expect("read final wal bytes");
        let readback = serde_json::json!({
            "append_lock_path": dir.join(".append.lock").display().to_string(),
            "segment_path": first.segment_path.display().to_string(),
            "partial_len_before_replay": partial_len,
            "locked_len_after_replay_attempt": locked_len,
            "final_len": segment_bytes.len(),
            "expected_final_len": first.end_offset + next.len() as u64,
            "torn_tail_present": replay.torn_tail.is_some(),
            "records": replay.records.iter().map(|record| serde_json::json!({
                "seq": record.seq,
                "payload": String::from_utf8_lossy(&record.payload),
                "start_offset": record.start_offset,
                "end_offset": record.end_offset,
            })).collect::<Vec<_>>(),
            "contains_completed_payload": segment_bytes.windows(b"completed".len()).any(|window| window == b"completed"),
        });
        fs::write(
            root.join("wal-recovery-lock-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .expect("write fsv readback");
    } else {
        cleanup(dir);
    }
}

#[test]
fn reopen_resumes_after_last_replayed_sequence() {
    let dir = test_dir("reopen-next-seq");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    wal.append_batch(&[b"one".as_slice(), b"two".as_slice()])
        .expect("append two");
    drop(wal);

    let mut reopened = Wal::open(&dir, WalOptions::default()).expect("reopen wal");
    let ack = reopened.append(b"three").expect("append after replay");
    drop(reopened);

    let replay = replay_dir(&dir).expect("replay reopened wal");
    assert_eq!(ack.seq, 3);
    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    cleanup(dir);
}

#[test]
fn torn_tail_in_early_segment_removes_later_segments() {
    let dir = test_dir("torn-removes-later");
    let first = record::encode(1, b"acked").expect("encode acked");
    let torn = record::encode(2, b"torn").expect("encode torn");
    let segment0 = dir.join("00000000000000000000.wal");
    let segment1 = dir.join("00000000000000000001.wal");
    fs::write(
        &segment0,
        [&first[..], &torn[..record::HEADER_LEN + 1]].concat(),
    )
    .expect("write segment 0");
    fs::write(&segment1, record::encode(3, b"discard").unwrap()).expect("write segment 1");

    let replay = replay_dir(&dir).expect("replay torn early segment");
    let tail = replay.torn_tail.expect("torn tail");

    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(replay.records.len(), 1);
    assert_eq!(fs::metadata(&segment0).unwrap().len(), first.len() as u64);
    assert!(!segment1.exists());
    cleanup(dir);
}

#[test]
fn open_fails_closed_on_noncanonical_wal_file_name() {
    let dir = test_dir("noncanonical-name");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    wal.append(b"committed").expect("append");
    drop(wal);
    fs::write(dir.join("0001.wal"), b"").expect("write noncanonical wal name");

    let error = Wal::open(&dir, WalOptions::default()).expect_err("noncanonical wal name");
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("0001.wal"), "{}", error.message);

    let replay_error = replay_dir(&dir).expect_err("replay refuses noncanonical wal name");
    assert_eq!(replay_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    cleanup(dir);
}

#[test]
fn open_fails_closed_on_wal_segment_index_gap() {
    let dir = test_dir("segment-gap");
    fs::write(
        dir.join("00000000000000000000.wal"),
        record::encode(1, b"first").expect("encode first"),
    )
    .expect("write segment 0");
    fs::write(
        dir.join("00000000000000000002.wal"),
        record::encode(2, b"third").expect("encode third"),
    )
    .expect("write segment 2");

    let error = Wal::open(&dir, WalOptions::default()).expect_err("segment gap");
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(
        error.message.contains("not contiguous"),
        "{}",
        error.message
    );
    cleanup(dir);
}

#[test]
fn open_ignores_files_without_wal_extension() {
    let dir = test_dir("foreign-files");
    fs::write(dir.join("notes.txt"), b"operator scratch").expect("write foreign file");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open with foreign file");
    wal.append(b"committed").expect("append");
    drop(wal);

    let replay = replay_dir(&dir).expect("replay with foreign file");
    assert_eq!(replay.records.len(), 1);
    assert_eq!(replay.records[0].payload, b"committed");
    cleanup(dir);
}

#[test]
fn record_golden_and_edge_cases_are_byte_exact() {
    let encoded = record::encode(42, b"hello").expect("encode golden");
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&42_u64.to_le_bytes());
    hasher.update(&5_u32.to_le_bytes());
    hasher.update(b"hello");

    assert_eq!(&encoded[0..4], b"CXW1");
    assert_eq!(&encoded[4..12], &42_u64.to_le_bytes());
    assert_eq!(&encoded[12..16], &5_u32.to_le_bytes());
    assert_eq!(&encoded[16..20], &hasher.finalize().to_le_bytes());
    assert_eq!(&encoded[20..], b"hello");

    let zero = record::encode(7, b"").expect("encode zero payload");
    assert_eq!(zero.len(), record::HEADER_LEN);

    let max = vec![0x5a; record::MAX_RECORD_BYTES as usize];
    let max_encoded = record::encode(8, &max).expect("encode max payload");
    assert_eq!(
        max_encoded.len(),
        record::HEADER_LEN + record::MAX_RECORD_BYTES as usize
    );
    let too_large = record::encode(9, &[0_u8; record::MAX_RECORD_BYTES as usize + 1])
        .expect_err("max+1 rejected");
    assert_eq!(too_large.kind(), ErrorKind::InvalidInput);
}

#[test]
fn corrupt_record_bytes_fail_closed_as_torn() {
    let complete = record::encode(11, b"payload").expect("encode");
    let mut crc_flip = complete.clone();
    crc_flip[16] ^= 0xff;
    assert_torn_contains(&crc_flip, "crc mismatch");
    assert_torn_contains(&complete[..record::HEADER_LEN - 1], "partial WAL header");
    assert_torn_contains(&complete[..record::HEADER_LEN], "partial WAL payload");
    let mut bad_magic = complete;
    bad_magic[0..4].copy_from_slice(&0_u32.to_le_bytes());
    assert_torn_contains(&bad_magic, "bad WAL magic");
}

proptest! {
    #[test]
    fn encoded_records_roundtrip(seq in any::<u64>(), payload in proptest::collection::vec(any::<u8>(), 0..=1024)) {
        let encoded = record::encode(seq, &payload).expect("encode proptest payload");
        let dir = test_dir("record-proptest");
        let path = dir.join("record.wal");
        fs::write(&path, &encoded).expect("write encoded record");
        let mut file = fs::File::open(&path).expect("open encoded record");

        match record::decode_at(&mut file, 0).expect("decode") {
            record::DecodeStatus::Complete(decoded) => {
                prop_assert_eq!(decoded.seq, seq);
                prop_assert_eq!(decoded.payload, payload);
                prop_assert_eq!(decoded.start_offset, 0);
                prop_assert_eq!(decoded.end_offset, encoded.len() as u64);
            }
            other => prop_assert!(false, "unexpected decode status: {other:?}"),
        }
        cleanup(dir);
    }
}

fn assert_torn_contains(bytes: &[u8], expected: &str) {
    let dir = test_dir("record-torn");
    let path = dir.join("record.wal");
    fs::write(&path, bytes).expect("write torn bytes");
    let mut file = fs::File::open(&path).expect("open torn bytes");
    match record::decode_at(&mut file, 0).expect("decode torn") {
        record::DecodeStatus::Torn { offset, message } => {
            assert_eq!(offset, 0);
            assert!(message.contains(expected), "{message}");
        }
        other => panic!("expected torn status, got {other:?}"),
    }
    cleanup(dir);
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
