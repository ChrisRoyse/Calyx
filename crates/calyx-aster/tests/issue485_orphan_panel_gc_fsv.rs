#[path = "issue485_orphan_panel_gc_fsv/support.rs"]
mod support;

#[test]
#[ignore = "manual FSV for issue #485 orphan and panel/codebook GC"]
fn issue485_orphan_panel_codebook_gc_fsv() {
    let root = support::fsv_root();
    let summary = support::run_fsv(&root);
    support::write_and_assert(&root, &summary);
}
