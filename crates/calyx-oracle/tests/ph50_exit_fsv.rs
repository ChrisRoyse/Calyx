mod support;

#[test]
#[ignore = "manual FSV for issue #439 PH50 exit gate"]
fn issue439_ph50_exit_gate_fsv_writes_readbacks() {
    support::ph50_exit::run_issue439_fsv();
}
