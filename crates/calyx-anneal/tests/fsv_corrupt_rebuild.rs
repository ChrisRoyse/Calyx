#[path = "fsv_support/mod.rs"]
mod fsv_support;
#[path = "support/fsv_corrupt_rebuild.rs"]
mod support;

#[ignore = "manual FSV for #405 corrupt ANN rebuild phase gate"]
#[test]
fn fsv_corrupt_ann_rebuild_and_failing_lens_route_manual() {
    support::run_issue405_fsv();
}
