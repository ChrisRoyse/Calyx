#[path = "support/complete_fsv/mod.rs"]
mod complete_fsv;

#[test]
fn ph51_complete_fsv_writes_partial_completion_readbacks() {
    complete_fsv::run_ph51_complete_fsv();
}
