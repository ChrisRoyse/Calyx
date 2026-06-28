use super::*;
use crate::memtable::Memtable;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn flushed_memtable_reads_back_byte_exact_and_ordered() {
    let dir = test_dir("sst-roundtrip");
    let path = dir.join("000001.sst");
    let mut table = Memtable::new(128);
    table.put(b"k03", b"three").expect("put k03");
    table.put(b"k01", b"one").expect("put k01");
    table.put(b"k02", b"two").expect("put k02");

    let summary = table.flush_to_sst(&path).expect("flush sst");
    let disk_bytes = fs::read(&path).expect("read sst bytes");
    assert_eq!(summary.bytes, disk_bytes.len() as u64);
    assert_eq!(&disk_bytes[0..4], b"CXS1");

    let reader = SstReader::open(&path).expect("open sst");
    assert_eq!(reader.get(b"k02").expect("get k02"), Some(b"two".to_vec()));
    let rows = reader.range(b"k01", b"k04").expect("range");
    let keys: Vec<_> = rows.into_iter().map(|row| row.key).collect();
    assert_eq!(keys, [b"k01".to_vec(), b"k02".to_vec(), b"k03".to_vec()]);
    assert!(reader.bloom_may_contain(b"k01"));
    assert!(reader.bloom_may_contain(b"k02"));
    assert!(reader.bloom_may_contain(b"k03"));
    cleanup(dir);
}

#[test]
fn corrupt_record_crc_fails_closed() {
    let dir = test_dir("sst-corrupt");
    let path = dir.join("000001.sst");
    let mut table = Memtable::new(128);
    table.put(b"k01", b"one").expect("put k01");
    table.flush_to_sst(&path).expect("flush sst");

    let mut bytes = fs::read(&path).expect("read sst");
    bytes[HEADER_LEN + RECORD_HEADER_LEN] ^= 0xff;
    fs::write(&path, bytes).expect("write corrupt sst");
    let error = SstReader::open(&path).expect_err("crc mismatch");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    cleanup(dir);
}

#[test]
fn invalid_header_offsets_fail_closed() {
    let dir = test_dir("sst-bad-header");
    let path = dir.join("000001.sst");
    let mut table = Memtable::new(128);
    table.put(b"k01", b"one").expect("put k01");
    table.flush_to_sst(&path).expect("flush sst");

    let mut bytes = fs::read(&path).expect("read sst");
    bytes[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
    fs::write(&path, bytes).expect("write bad header");
    let error = SstReader::open(&path).expect_err("bad header rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    cleanup(dir);
}

#[test]
fn corrupt_index_section_fails_closed_on_open() {
    let dir = test_dir("sst-corrupt-index");
    let path = dir.join("000001.sst");
    let summary = write_sst(
        &path,
        [
            (b"k01".as_slice(), b"one".as_slice()),
            (b"k02".as_slice(), b"two".as_slice()),
        ],
    )
    .expect("write sst");

    let mut bytes = fs::read(&path).expect("read sst");
    bytes[summary.index_offset as usize + INDEX_ENTRY_FIXED_LEN] ^= 0x01;
    fs::write(&path, bytes).expect("write corrupt index");
    let error = SstReader::open(&path).expect_err("index crc rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    cleanup(dir);
}

#[test]
fn corrupt_bloom_section_fails_closed_on_open() {
    let dir = test_dir("sst-corrupt-bloom");
    let path = dir.join("000001.sst");
    let summary = write_sst(&path, [(b"k01".as_slice(), b"one".as_slice())]).expect("write sst");

    let mut bytes = fs::read(&path).expect("read sst");
    bytes[summary.bloom_offset as usize] ^= 0x01;
    fs::write(&path, bytes).expect("write corrupt bloom");
    let error = SstReader::open(&path).expect_err("bloom crc rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
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
