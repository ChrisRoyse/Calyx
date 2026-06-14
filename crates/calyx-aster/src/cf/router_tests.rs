use super::{CfRouter, ColumnFamily};
use calyx_core::SlotId;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn put_get_and_flush_dispatch_per_cf() {
    let dir = test_dir("put-get");
    let mut router = CfRouter::open(&dir, 12).unwrap();

    router.put(ColumnFamily::Base, b"k1", b"v1").unwrap();
    router
        .put(ColumnFamily::slot(SlotId::new(0)), b"k1", b"s1")
        .unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    router.flush_cf(ColumnFamily::slot(SlotId::new(0))).unwrap();

    assert_eq!(
        router.get(ColumnFamily::Base, b"k1").unwrap(),
        Some(b"v1".to_vec())
    );
    assert_eq!(
        router
            .get(ColumnFamily::slot(SlotId::new(0)), b"k1")
            .unwrap(),
        Some(b"s1".to_vec())
    );
    assert_eq!(router.level_file_count(ColumnFamily::Base), 1);
    assert_eq!(
        router.level_file_count(ColumnFamily::slot(SlotId::new(0))),
        1
    );
    cleanup(dir);
}

#[test]
fn range_merges_memtable_and_sst_with_memtable_winning() {
    let dir = test_dir("range");
    let mut router = CfRouter::open(&dir, 1024).unwrap();
    router.put(ColumnFamily::Base, b"k1", b"old").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    router.put(ColumnFamily::Base, b"k1", b"new").unwrap();
    router.put(ColumnFamily::Base, b"k2", b"two").unwrap();

    let rows = router.range(ColumnFamily::Base, b"", b"\xff").unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value, b"new");
    assert_eq!(rows[1].value, b"two");
    cleanup(dir);
}

#[test]
fn reopen_loads_existing_sst_files() {
    let dir = test_dir("reopen");
    let mut router = CfRouter::open(&dir, 64).unwrap();
    router.put(ColumnFamily::Base, b"k", b"value").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    drop(router);

    let reopened = CfRouter::open(&dir, 64).unwrap();

    assert_eq!(
        reopened.get(ColumnFamily::Base, b"k").unwrap(),
        Some(b"value".to_vec())
    );
    cleanup(dir);
}

#[test]
fn reopen_fails_closed_on_unrecognized_sst_name() {
    let dir = test_dir("unrecognized-sst");
    let mut router = CfRouter::open(&dir, 64).unwrap();
    router.put(ColumnFamily::Base, b"k", b"value").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    drop(router);
    std::fs::write(dir.join("cf/base/junk.sst"), b"foreign bytes").unwrap();

    let error = CfRouter::open(&dir, 64).expect_err("unrecognized SST name");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("junk.sst"), "{}", error.message);
    cleanup(dir);
}

#[test]
fn reopen_fails_closed_on_unknown_cf_directory() {
    let dir = test_dir("unknown-cf-dir");
    let mut router = CfRouter::open(&dir, 64).unwrap();
    router.put(ColumnFamily::Base, b"k", b"value").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    drop(router);
    std::fs::create_dir_all(dir.join("cf/not_a_family")).unwrap();

    let error = CfRouter::open(&dir, 64).expect_err("unknown CF directory");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("not_a_family"), "{}", error.message);
    cleanup(dir);
}

#[test]
fn next_file_counter_resumes_past_existing_router_ssts() {
    let dir = test_dir("next-file-resume");
    let mut router = CfRouter::open(&dir, 64).unwrap();
    router.put(ColumnFamily::Base, b"k1", b"v1").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    router.put(ColumnFamily::Base, b"k2", b"v2").unwrap();
    router.flush_cf(ColumnFamily::Base).unwrap();
    drop(router);

    let mut reopened = CfRouter::open(&dir, 64).unwrap();
    reopened.put(ColumnFamily::Base, b"k3", b"v3").unwrap();
    let summary = reopened.flush_cf(ColumnFamily::Base).unwrap();

    assert_eq!(
        summary.path.file_name().unwrap().to_str().unwrap(),
        "00000000000000000003.sst"
    );
    cleanup(dir);
}

#[test]
fn assay_cf_persists_and_reopens() {
    let dir = test_dir("assay");
    let mut router = CfRouter::open(&dir, 64).unwrap();
    router
        .put(ColumnFamily::Assay, b"panel-a", b"bits")
        .unwrap();
    router.flush_cf(ColumnFamily::Assay).unwrap();
    drop(router);

    let reopened = CfRouter::open(&dir, 64).unwrap();

    assert_eq!(
        reopened.get(ColumnFamily::Assay, b"panel-a").unwrap(),
        Some(b"bits".to_vec())
    );
    assert_eq!(reopened.iter_cf(ColumnFamily::Assay).unwrap().len(), 1);
    cleanup(dir);
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-router-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
