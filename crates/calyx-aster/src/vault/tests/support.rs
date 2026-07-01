use super::*;

pub(super) fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

pub(super) fn sst_count(dir: PathBuf) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().path().extension().unwrap() == "sst")
        .count()
}

pub(super) fn wal_bytes(dir: &Path) -> u64 {
    let wal = dir.join("wal");
    if !wal.is_dir() {
        return 0;
    }
    fs::read_dir(wal)
        .unwrap()
        .map(|entry| fs::metadata(entry.unwrap().path()).unwrap().len())
        .sum()
}

pub(super) fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}

pub(super) fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx-ph35-group-commit-fsv")
    })
}

pub(super) fn reset_dir(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

pub(super) fn row_index(rows: &[encode::WriteRow], cf: ColumnFamily) -> usize {
    rows.iter()
        .position(|row| row.cf == cf)
        .expect("row for CF")
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
