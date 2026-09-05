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

#[path = "dedup_anchor_conflict_property_readback.rs"]
mod dedup_anchor_conflict_property_readback;
#[path = "dedup_anchor_conflict_readback.rs"]
mod dedup_anchor_conflict_readback;
#[path = "dedup_audit_readback.rs"]
mod dedup_audit_readback;
#[path = "dedup_rolled_undo_readback.rs"]
mod dedup_rolled_undo_readback;
#[path = "issue757_summarize_cli.rs"]
mod issue757_summarize_cli;
#[path = "manifest_readback_show_manifest_fsv.rs"]
mod manifest_readback_show_manifest_fsv;
#[path = "ph42_readback_schema.rs"]
mod ph42_readback_schema;
#[path = "recurrence_concurrency_readback.rs"]
mod recurrence_concurrency_readback;
#[path = "temporal_log_recurrence_readback.rs"]
mod temporal_log_recurrence_readback;
