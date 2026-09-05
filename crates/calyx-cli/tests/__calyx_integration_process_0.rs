//! Generated integration-test harness. Regenerate with
//! `python scripts/consolidate_integration_tests.py`.

#[path = "support/dedup_fsv_io.rs"]
mod __calyx_shared_support_dedup_fsv_io_rs;

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "support/fsv_io.rs"]
mod __calyx_shared_support_fsv_io_rs;

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "support/mod.rs"]
mod __calyx_shared_support_mod_rs;

#[path = "audit_cli.rs"]
mod audit_cli;
#[path = "audit_cli_hardening.rs"]
mod audit_cli_hardening;
#[path = "checkpoint_scan.rs"]
mod checkpoint_scan;
#[path = "dedup_qqp_paws_fsv.rs"]
mod dedup_qqp_paws_fsv;
#[path = "navigate_fsv.rs"]
mod navigate_fsv;
#[path = "periodic_recall_readback.rs"]
mod periodic_recall_readback;
#[path = "ph42_readback_surfaces.rs"]
mod ph42_readback_surfaces;
#[path = "time_prediction_readback.rs"]
mod time_prediction_readback;
#[path = "timetravel_readback_fsv.rs"]
mod timetravel_readback_fsv;
