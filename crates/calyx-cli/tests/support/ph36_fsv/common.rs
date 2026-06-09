use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, VaultId};
use calyx_ledger::HitRef;

pub fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

pub fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph36-fsv-integration"))
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub fn hit(cx_id: CxId, score: f32) -> HitRef {
    HitRef { cx_id, score }
}

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
