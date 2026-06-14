//! PH54 T06 FSV: same-seq data/index writes and no half-indexed crash state.
//!
//! Trigger on aiwonder with:
//! `CALYX_ISSUE462_FSV_ROOT=/home/croyse/calyx/data/fsv-issue462-ph54-<stamp> \
//! cargo test -p calyx-aster --test ph54_fsv -- --ignored --nocapture`

use std::fs;
use std::path::PathBuf;

#[path = "ph54_fsv/support.rs"]
mod support;

#[test]
fn ph54_fsv_same_seq_crash_range_rebuild() {
    let root = std::env::temp_dir().join("calyx-ph54-fsv-test");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let evidence = support::run_fsv(&root);
    support::write_and_assert(&root, &evidence);
}

#[test]
#[ignore]
fn ph54_fsv_aiwonder() {
    let root = PathBuf::from(
        std::env::var_os("CALYX_ISSUE462_FSV_ROOT")
            .expect("CALYX_ISSUE462_FSV_ROOT must point at a fresh aiwonder evidence root"),
    );
    if root.exists() {
        panic!("CALYX_ISSUE462_FSV_ROOT must be fresh: {}", root.display());
    }
    fs::create_dir_all(&root).unwrap();
    let evidence = support::run_fsv(&root);
    support::write_and_assert(&root, &evidence);
}
