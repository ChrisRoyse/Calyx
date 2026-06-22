#![cfg(unix)]

//! PH56 T05 FSV for mmap-backed cold column reads.

use calyx_aster::mmap_col::{CALYX_BOUNDS_EXCEEDED, CALYX_NOT_FOUND, MmapColumn};
mod fsv_support;
use fsv_support::{fsv_root_os, reset_dir};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

const COLD_FILE_LEN: u64 = 1_073_741_824;
const READ_LEN: usize = 1_048_576;
const RSS_DELTA_LIMIT: u64 = 2 * 1024 * 1024;

#[test]
#[ignore = "manual FSV maps a 1GiB cold column and records RSS readback"]
fn issue472_mmap_column_fsv() {
    let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-issue472-fsv");
    reset_dir(&root);
    let cold_path = root.join("cold-column-1g.bin");
    let f32_path = root.join("f32-column.bin");
    let empty_path = root.join("empty-column.bin");

    let expected = pattern(READ_LEN);
    write_sparse_cold_file(&cold_path, &expected);
    write_f32_file(&f32_path);
    fs::write(&empty_path, []).expect("write empty file");

    let rss_before = rss_bytes();
    let column = MmapColumn::open(&cold_path).expect("open cold mmap column");
    let rss_after_open = rss_bytes();
    assert_eq!(column.file_len() as u64, COLD_FILE_LEN);

    column.prefetch(0, READ_LEN);
    let slice = column.read_slice(0, READ_LEN).expect("read first MiB");
    assert_eq!(slice, expected);
    let slice_sha256 = sha256_hex(slice);
    let expected_sha256 = sha256_hex(&expected);
    let rss_after_read = rss_bytes();
    column.drop_pages(0, READ_LEN);

    let rss_delta_after_read = rss_after_read.saturating_sub(rss_before);
    assert!(
        rss_delta_after_read <= RSS_DELTA_LIMIT,
        "rss delta {rss_delta_after_read} exceeded {RSS_DELTA_LIMIT}"
    );

    let f32_column = MmapColumn::open(&f32_path).expect("open f32 column");
    let f32_values = f32_column
        .read_f32_slice(0, 4)
        .expect("read f32 values")
        .to_vec();
    assert_eq!(f32_values, [1.0_f32, 2.0, 3.0, 4.0]);

    let bounds_error = column
        .read_slice((COLD_FILE_LEN as usize) - 4, 8)
        .expect_err("bounds reject");
    let alignment_error = f32_column
        .read_f32_slice(3, 1)
        .expect_err("alignment reject");
    let missing_error =
        MmapColumn::open(&root.join("missing-column.bin")).expect_err("missing reject");
    let empty_error = MmapColumn::open(&empty_path).expect_err("empty reject");

    assert_eq!(bounds_error.code, CALYX_BOUNDS_EXCEEDED);
    assert_eq!(alignment_error.code, CALYX_BOUNDS_EXCEEDED);
    assert_eq!(missing_error.code, CALYX_NOT_FOUND);
    assert_eq!(empty_error.code, CALYX_NOT_FOUND);

    let readback = json!({
        "cold_path": cold_path,
        "cold_file_len": COLD_FILE_LEN,
        "read_offset": 0,
        "read_len": READ_LEN,
        "slice_sha256": slice_sha256,
        "expected_sha256": expected_sha256,
        "rss_before": rss_before,
        "rss_after_open": rss_after_open,
        "rss_after_read": rss_after_read,
        "rss_delta_after_read": rss_delta_after_read,
        "rss_delta_limit": RSS_DELTA_LIMIT,
        "f32_values": f32_values,
        "bounds_error_code": bounds_error.code,
        "alignment_error_code": alignment_error.code,
        "missing_error_code": missing_error.code,
        "empty_error_code": empty_error.code,
        "prefetch_drop_pages_called": true,
    });
    fs::write(
        root.join("issue472-mmap-fsv-readback.json"),
        serde_json::to_vec_pretty(&readback).expect("encode readback"),
    )
    .expect("write readback");
    println!(
        "issue472 FSV: file_len={COLD_FILE_LEN} read_len={READ_LEN} rss_delta={rss_delta_after_read}"
    );
}

fn write_sparse_cold_file(path: &Path, expected: &[u8]) {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .expect("create cold file");
    file.set_len(COLD_FILE_LEN).expect("set sparse length");
    file.seek(SeekFrom::Start(0)).expect("seek start");
    file.write_all(expected).expect("write expected prefix");
    file.sync_all().expect("sync cold file");
}

fn write_f32_file(path: &Path) {
    let bytes = [1.0_f32, 2.0, 3.0, 4.0]
        .into_iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect::<Vec<_>>();
    fs::write(path, bytes).expect("write f32 file");
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| ((index.wrapping_mul(31) + 7) % 251) as u8)
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn rss_bytes() -> u64 {
    let status = fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .expect("VmRSS value")
                .parse::<u64>()
                .expect("parse VmRSS");
            return kb * 1024;
        }
    }
    panic!("VmRSS not found");
}
