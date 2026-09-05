use super::*;
use calyx_aster::cf::CfRouter;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "calyx-issue1302-compaction-error-{}-{id}",
            std::process::id()
        )))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gc_tick_fails_closed_when_compaction_inventory_is_unreadable() {
    let root = TestRoot::new();
    let vault_dir = root.0.join("vault");
    let router = CfRouter::open(&vault_dir, 1024 * 1024).expect("open router");
    let store = VersionedCfStore::new_with_router(0, router);
    let corrupt_cf = vault_dir.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&corrupt_cf).expect("create corrupt CF directory");
    fs::write(
        corrupt_cf.join("flush-00000000000000000001-0000.sst"),
        b"not an SST",
    )
    .expect("write corrupt SST");
    let mut wal =
        Wal::open(root.0.join("wal"), calyx_aster::wal::WalOptions::default()).expect("open WAL");
    let recycler = WalRecycler::with_limits(64, 64, Duration::ZERO);
    let mut counts = SoakCounts {
        gc_ticks: GC_SWEEP_EVERY_GC_TICKS - 1,
        ..SoakCounts::default()
    };

    let result = gc_tick_op(1, &vault_dir, &store, &mut wal, &recycler, 0, &mut counts);

    assert!(
        result.is_err(),
        "an unreadable physical inventory must fail the soak instead of being discarded"
    );
}
