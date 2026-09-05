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
fn replay_after_checkpoint_skips_obsolete_payloads_and_replays_tail() {
    let dir = test_dir("replay-after-checkpoint");
    let options = WalOptions {
        max_segment_bytes: 56,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    wal.append(b"checkpointed-one").expect("append first");
    wal.append(b"checkpointed-two").expect("append second");
    let tail = wal.append(b"tail-three").expect("append tail");
    drop(wal);

    let replay = replay_dir_after(&dir, 2).expect("replay after checkpoint");

    assert_eq!(replay.torn_tail, None);
    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| (record.seq, record.payload.as_slice()))
            .collect::<Vec<_>>(),
        vec![(tail.seq, b"tail-three".as_slice())]
    );
    cleanup(dir);
}

#[test]
fn open_after_checkpoint_resumes_after_floor_when_tail_is_empty() {
    let dir = test_dir("open-after-checkpoint");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    wal.append_batch(&[b"one".as_slice(), b"two".as_slice()])
        .expect("append checkpointed");
    drop(wal);

    let mut reopened = Wal::open_after(&dir, WalOptions::default(), 2).expect("open after floor");
    let ack = reopened
        .append(b"three")
        .expect("append after checkpoint floor");
    drop(reopened);

    assert_eq!(ack.seq, 3);
    let replay = replay_dir(&dir).expect("full replay");
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
fn torn_tail_in_early_segment_fails_closed_and_preserves_all_segments() {
    let fsv_root = std::env::var_os("CALYX_WAL_MID_LOG_CORRUPTION_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("torn-fails-closed"),
        |root| {
            let _ = fs::remove_dir_all(root);
            let dir = root.join("wal");
            fs::create_dir_all(&dir).expect("create fsv wal dir");
            dir
        },
    );
    let first = record::encode(1, b"acked").expect("encode acked");
    let torn = record::encode(2, b"torn").expect("encode torn");
    let segment0 = dir.join("00000000000000000000.wal");
    let segment1 = dir.join("00000000000000000001.wal");
    let segment0_before = [&first[..], &torn[..record::HEADER_LEN + 1]].concat();
    let segment1_before = record::encode(3, b"durable-later").expect("encode later record");
    fs::write(&segment0, &segment0_before).expect("write segment 0");
    fs::write(&segment1, &segment1_before).expect("write segment 1");

    let replay = replay_dir(&dir);
    let segment0_after = fs::read(&segment0).ok();
    let segment1_after = fs::read(&segment1).ok();
    let segment0_preserved = segment0_after.as_deref() == Some(segment0_before.as_slice());
    let segment1_preserved = segment1_after.as_deref() == Some(segment1_before.as_slice());

    assert!(
        segment0_preserved && segment1_preserved,
        "mid-log replay mutated durable bytes: segment0_preserved={segment0_preserved} \
         segment1_preserved={segment1_preserved}"
    );
    let error = replay.expect_err("mid-log corruption must fail closed");
    assert_eq!(error.code, "CALYX_ASTER_TORN_WAL");
    assert!(error.message.contains(&segment0.display().to_string()));
    if fsv_root.is_none() {
        cleanup(dir);
    }
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
    // `record::encode` is the single physical-record encoder: one physical
    // record still caps at MAX_RECORD_BYTES. Logical payloads larger than this
    // are no longer rejected by the WAL — the writer fragments them across a
    // CXW2 chain (see `oversized_payload_fragments_and_replays_byte_exact`).
    let too_large = record::encode(9, &[0_u8; record::MAX_RECORD_BYTES as usize + 1])
        .expect_err("max+1 rejected by the single-record encoder");
    assert_eq!(too_large.kind(), ErrorKind::InvalidInput);
}

#[test]
fn oversized_payload_fragments_and_replays_byte_exact() {
    let dir = test_dir("fragment-roundtrip");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");

    let prologue = wal.append(b"prologue").expect("append prologue");
    // Exceed the per-record cap so the writer must emit a CXW2 fragment chain.
    let big: Vec<u8> = (0..(record::MAX_RECORD_BYTES as usize + 4096))
        .map(|index| (index % 251) as u8)
        .collect();
    let big_ack = wal.append(&big).expect("append oversized payload");
    let epilogue = wal.append(b"epilogue").expect("append epilogue");
    drop(wal);

    // One logical seq per commit; fragmentation does not consume extra seqs.
    assert_eq!(prologue.seq + 1, big_ack.seq);
    assert_eq!(big_ack.seq + 1, epilogue.seq);
    // The whole chain lands in a single segment file (fragments never straddle).
    assert_ne!(big_ack.segment_path, prologue.segment_path);
    assert_ne!(big_ack.segment_path, epilogue.segment_path);

    let replay = replay_dir(&dir).expect("replay fragmented wal");
    assert_eq!(replay.torn_tail, None);
    assert_eq!(replay.records.len(), 3);

    assert!(!replay.records[0].fragmented);
    assert_eq!(replay.records[0].payload, b"prologue");

    let reassembled = &replay.records[1];
    assert_eq!(reassembled.seq, big_ack.seq);
    assert!(
        reassembled.fragmented,
        "oversized commit must reassemble a chain"
    );
    assert_eq!(
        reassembled.payload, big,
        "fragment chain must round-trip byte-exact"
    );
    assert_eq!(reassembled.segment_path, big_ack.segment_path);
    assert_eq!(reassembled.start_offset, big_ack.start_offset);
    assert_eq!(reassembled.end_offset, big_ack.end_offset);

    assert!(!replay.records[2].fragmented);
    assert_eq!(replay.records[2].payload, b"epilogue");
    cleanup(dir);
}

#[test]
fn torn_fragment_chain_tail_discards_whole_logical_record() {
    let dir = test_dir("fragment-torn");
    // Large segment so the durable record and the chain share one segment.
    let options = WalOptions {
        max_segment_bytes: 512 * 1024 * 1024,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    let durable = wal.append(b"durable-record").expect("append durable");
    let big: Vec<u8> = (0..(record::MAX_RECORD_BYTES as usize + 4096))
        .map(|index| (index % 251) as u8)
        .collect();
    let torn = wal.append(&big).expect("append oversized payload");
    drop(wal);
    assert_eq!(durable.segment_path, torn.segment_path);
    assert_eq!(torn.start_offset, durable.end_offset);

    // Truncate inside the first fragment so the chain is incomplete at the tail.
    let file = OpenOptions::new()
        .write(true)
        .open(&torn.segment_path)
        .expect("open segment for truncation");
    file.set_len(torn.start_offset + 64)
        .expect("truncate mid-chain");
    file.sync_data().expect("fsync truncation");
    drop(file);

    let replay = replay_dir(&dir).expect("replay torn chain");
    let tail = replay.torn_tail.expect("torn tail reported");
    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(tail.offset, torn.start_offset);
    assert_eq!(replay.records.len(), 1);
    assert_eq!(replay.records[0].seq, durable.seq);
    assert_eq!(replay.records[0].payload, b"durable-record");
    // The entire torn logical record was truncated away.
    assert_eq!(
        fs::metadata(&torn.segment_path)
            .expect("segment metadata")
            .len(),
        torn.start_offset
    );
    cleanup(dir);
}

#[test]
fn logical_payload_above_ceiling_fails_closed_without_disk_pressure() {
    let dir = test_dir("payload-ceiling");
    let mut wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
    let over = vec![0u8; (record::MAX_LOGICAL_PAYLOAD_BYTES + 1) as usize];

    let error = wal
        .append(&over)
        .expect_err("payload above ceiling rejected");
    assert_eq!(error.code, CALYX_WAL_PAYLOAD_TOO_LARGE);
    assert_ne!(error.code, "CALYX_DISK_PRESSURE");
    assert!(
        error.message.contains("single-commit ceiling"),
        "{}",
        error.message
    );
    // Fail-closed: nothing was written to the WAL.
    let replay = replay_dir(&dir).expect("replay after rejected append");
    assert!(replay.records.is_empty());
    cleanup(dir);
}

#[test]
fn legacy_cxw1_segments_still_replay() {
    let dir = test_dir("legacy-cxw1");
    // Bytes written by the original single-format encoder must replay unchanged.
    let first = record::encode(1, b"legacy-one").expect("encode legacy one");
    let second = record::encode(2, b"legacy-two").expect("encode legacy two");
    let bytes = [first.as_slice(), second.as_slice()].concat();
    fs::write(dir.join("00000000000000000000.wal"), &bytes).expect("write legacy segment");

    let replay = replay_dir(&dir).expect("replay legacy segment");
    assert_eq!(replay.torn_tail, None);
    assert_eq!(replay.records.len(), 2);
    assert_eq!(replay.records[0].seq, 1);
    assert_eq!(replay.records[0].payload, b"legacy-one");
    assert!(!replay.records[0].fragmented);
    assert_eq!(replay.records[1].seq, 2);
    assert_eq!(replay.records[1].payload, b"legacy-two");
    cleanup(dir);
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

#[test]
fn recover_tip_seq_reads_highest_seq_without_materializing_payloads() {
    // Issue #1992: attaching a fresh write handle to a large multi-segment WAL
    // must recover next_seq via header-seek scans, not by decoding every
    // payload into the heap. The final record is multi-MiB, so a payload-
    // materializing recovery would copy it; the header-only path reads none.
    let dir = test_dir("recover-tip-seq");
    let options = WalOptions {
        max_segment_bytes: 64,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    for index in 0..6u8 {
        wal.append(format!("tip-record-{index:03}").as_bytes())
            .expect("append");
    }
    let big = vec![0xAB_u8; 4 * 1024 * 1024];
    let last = wal.append(&big).expect("append big");
    drop(wal);

    // Segments actually rolled, so this exercises the backwards multi-segment
    // scan, and the tip is the highest committed seq.
    assert!(super::segment::list_segments(&dir).unwrap().len() >= 2);
    let tip = super::recover_tip_seq_locked(&dir).expect("recover tip");
    assert_eq!(tip, last.seq);
    cleanup(dir);
}

#[test]
fn recover_tip_seq_is_zero_for_empty_wal() {
    let dir = test_dir("recover-tip-empty");
    let _wal = Wal::open(&dir, WalOptions::default()).expect("open empty wal");
    assert_eq!(super::recover_tip_seq_locked(&dir).expect("recover"), 0);
    cleanup(dir);
}

#[test]
fn recover_tip_seq_walks_back_over_empty_newest_segment() {
    // A freshly rotated newest segment can be empty; recovery must walk back to
    // the previous non-empty segment for the highest seq.
    let dir = test_dir("recover-tip-walkback");
    let options = WalOptions {
        max_segment_bytes: 1,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    let last = wal.append(b"only-record").expect("append");
    drop(wal);
    let next_index = super::segment::list_segments(&dir)
        .unwrap()
        .last()
        .unwrap()
        .0
        + 1;
    fs::write(dir.join(format!("{next_index:020}.wal")), b"").expect("empty segment");

    assert_eq!(
        super::recover_tip_seq_locked(&dir).expect("recover"),
        last.seq
    );
    cleanup(dir);
}

#[test]
fn fresh_handle_refreshes_next_seq_after_external_multi_segment_appends() {
    // The wal-prune / ingest reproduction: a handle whose cached active segment
    // is stale must recover the correct next_seq through the header-seek
    // refresh path (issue #1992) and append at the contiguous next seq — never
    // via a payload-materializing full replay.
    let dir = test_dir("external-refresh");
    let options = WalOptions {
        max_segment_bytes: 1,
        ..WalOptions::default()
    };
    let mut first = Wal::open(&dir, options).expect("open first");
    assert_eq!(first.append(b"r1").expect("append r1").seq, 1);

    {
        let mut second = Wal::open(&dir, options).expect("open second");
        for index in 0..5u8 {
            second
                .append(format!("ext-{index}").as_bytes())
                .expect("external append");
        }
    }

    let next = first
        .append(b"after-refresh")
        .expect("append after refresh");
    assert_eq!(next.seq, 7, "next_seq must recover to the external tip + 1");

    drop(first);
    let replay = replay_dir(&dir).expect("replay");
    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        (1..=7).collect::<Vec<_>>(),
        "seqs must be contiguous with no gaps or duplicates"
    );
    cleanup(dir);
}

#[test]
fn for_each_record_payload_after_skips_below_floor_and_reuses_buffer() {
    let dir = test_dir("payload-after-floor");
    let options = WalOptions {
        max_segment_bytes: 1,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    for index in 0..6u8 {
        wal.append(format!("payload-{index}").as_bytes())
            .expect("append");
    }
    drop(wal);

    // Floor 3: seqs 1..=3 header-skipped (no payload read), seqs 4..=6 decoded.
    let mut seen = Vec::new();
    super::for_each_record_payload_after(&dir, 3, |seq, payload| {
        seen.push((seq, payload.to_vec()));
        Ok(true)
    })
    .expect("scan");
    assert_eq!(
        seen,
        vec![
            (4, b"payload-3".to_vec()),
            (5, b"payload-4".to_vec()),
            (6, b"payload-5".to_vec()),
        ]
    );
    cleanup(dir);
}

#[test]
fn for_each_record_payload_reverse_visits_newest_first_and_stops_early() {
    let dir = test_dir("payload-reverse");
    let options = WalOptions {
        max_segment_bytes: 1,
        ..WalOptions::default()
    };
    let mut wal = Wal::open(&dir, options).expect("open wal");
    for index in 0..5u8 {
        wal.append(format!("rev-{index}").as_bytes())
            .expect("append");
    }
    drop(wal);

    // Stop after the first (newest) record: proves newest-first + early stop, so
    // a bounded lookup never decodes the whole WAL.
    let mut visited = Vec::new();
    super::for_each_record_payload_reverse(&dir, |seq, payload| {
        visited.push((seq, payload.to_vec()));
        Ok(false)
    })
    .expect("reverse scan");
    assert_eq!(visited, vec![(5, b"rev-4".to_vec())]);
    cleanup(dir);
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
