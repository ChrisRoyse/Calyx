use super::record;
use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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
