//! Generated integration-test harness. Regenerate with
//! `python scripts/consolidate_integration_tests.py`.

#[allow(
    dead_code,
    reason = "shared integration support is used selectively by each harness"
)]
#[path = "fsv_support.rs"]
mod __calyx_shared_fsv_support_rs;

#[path = "issue051_temporal_graph_edges_fsv.rs"]
mod issue051_temporal_graph_edges_fsv;
