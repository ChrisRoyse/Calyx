#![no_main]

use calyx_aster::sst::SstReader;
use libfuzzer_sys::fuzz_target;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_INPUT: usize = 1 << 20;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|data: &[u8]| {
    let data = bounded(data);
    let dir = temp_dir("aster-sst");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("input.sst");
    if fs::write(&path, data).is_ok()
        && let Ok(reader) = SstReader::open(&path)
    {
        let _ = reader.iter();
        let _ = reader.get(b"fuzz-key");
        let _ = reader.range(b"", b"\xff");
    }
    let _ = fs::remove_dir_all(dir);
});

fn bounded(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT)]
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("calyx-fuzz-{name}-{}-{id}", std::process::id()))
}
