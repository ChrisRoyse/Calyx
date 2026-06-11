#[path = "support/fsv_corrupt_rebuild.rs"]
mod support;

#[ignore = "manual aiwonder FSV for #405 corrupt ANN rebuild phase gate"]
#[test]
fn fsv_corrupt_ann_rebuild_and_failing_lens_route_aiwonder() {
    support::run_issue405_fsv();
}
