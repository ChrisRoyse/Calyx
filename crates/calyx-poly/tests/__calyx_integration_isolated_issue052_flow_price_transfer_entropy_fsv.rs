//! Generated integration-test harness. Regenerate with
//! `python scripts/consolidate_integration_tests.py`.

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "fsv_support.rs"]
mod __calyx_shared_fsv_support_rs;

#[path = "issue052_flow_price_transfer_entropy_fsv.rs"]
mod issue052_flow_price_transfer_entropy_fsv;
