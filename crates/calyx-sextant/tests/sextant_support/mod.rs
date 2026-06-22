#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, SlotVector, VaultId, content_address};

pub fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

pub fn cx_u8_fill(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

pub fn cx_u128_be(value: u128) -> CxId {
    CxId::from_bytes(value.to_be_bytes())
}

pub fn cx_usize_be(value: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&(value as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub fn cx_u32_be(value: u32) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&u64::from(value).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub fn digest_hex(bytes: &[u8]) -> String {
    hex(&content_address([bytes]))
}

pub fn raw_blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

pub fn write_root_file_blake3_sums(root: &Path) {
    let mut entries = fs::read_dir(root)
        .expect("read root")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    entries.sort();
    let mut lines = String::new();
    for path in entries {
        if path.file_name().and_then(|name| name.to_str()) == Some("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read artifact");
        let name = path.file_name().expect("file name").to_string_lossy();
        lines.push_str(&format!("{}  {}\n", raw_blake3_hex(&bytes), name));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksums");
}

pub fn fsv_root(env_key: &str, temp_prefix: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("{temp_prefix}-{}", std::process::id())),
        false,
    )
}

pub fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}

pub fn write_named_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = Path::new(root).join(name);
    let file = fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

pub fn guarded_test_guard_id() -> calyx_ward::GuardId {
    "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
        .parse()
        .expect("guard id")
}

pub fn default_vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
