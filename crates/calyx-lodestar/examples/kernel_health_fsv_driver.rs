//! FSV driver for issue #644 — writes hand-known synthetic kernel artifacts
//! so `calyx readback kernel-health` output can be judged against expected
//! values computed by hand. Usage:
//!
//! ```text
//! cargo run -p calyx-lodestar --example kernel_health_fsv_driver -- <store-root> [rebuild]
//! ```
//!
//! Writes two artifacts under `<store-root>/idx/kernel/<id>/kernel.json`:
//! - kernel `2a..2a`: 3 members, 2 kernel-graph nodes, 1 unanchored member
//!   (grounded fraction 2/3), recall 0.9 over 10 queries vs a 0.95 gate.
//!   With `rebuild`, the same kernel is re-persisted with recall 0.99 over
//!   25 queries and built_at 67890.
//! - kernel `2d..2d`: fully ungrounded (provisional) variant.

use calyx_core::CxId;
use calyx_lodestar::{
    FsKernelStore, GroundednessReport, Kernel, RecallReport, RecallTestParams,
    write_kernel_artifact,
};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn known_kernel() -> Kernel {
    Kernel {
        kernel_id: cx(0x2a),
        panel_version: 7,
        anchor_kind: Some("synthetic_outcome".to_string()),
        corpus_shard_hash: [7; 32],
        members: vec![cx(1), cx(2), cx(3)],
        kernel_graph: vec![cx(1), cx(2)],
        groundedness: GroundednessReport {
            reached_anchor: 2.0 / 3.0,
            unanchored_members: vec![cx(3)],
        },
        recall: RecallReport {
            kernel_only: 0.9,
            full: 1.0,
            ratio: 0.9,
            approx_factor: 2.0,
            tau_star_estimate: 4,
            tau_star_exact: false,
            recall_test_params: Some(RecallTestParams::default()),
            corpus_name: Some("synthetic".to_string()),
            n_queries_tested: 10,
            held_out: vec![cx(9)],
            warning: None,
        },
        built_at_millis: 12345,
        estimator_provenance: "ph32::Tournament2Approx; trust=anchored".to_string(),
        warnings: Vec::new(),
    }
}

fn ungrounded_kernel() -> Kernel {
    let mut kernel = known_kernel();
    kernel.kernel_id = cx(0x2d);
    kernel.groundedness = GroundednessReport {
        reached_anchor: 0.0,
        unanchored_members: kernel.members.clone(),
    };
    kernel.warnings =
        vec!["CALYX_KERNEL_UNGROUNDED: all kernel members are provisional".to_string()];
    kernel.estimator_provenance = "ph32::Tournament2Approx; trust=provisional".to_string();
    kernel
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args
        .next()
        .expect("usage: kernel_health_fsv_driver <store-root> [rebuild]");
    let rebuild = args.next().as_deref() == Some("rebuild");
    let store = FsKernelStore::new(&root);

    let mut kernel = known_kernel();
    if rebuild {
        kernel.recall.kernel_only = 0.99;
        kernel.recall.ratio = 0.99;
        kernel.recall.n_queries_tested = 25;
        kernel.built_at_millis = 67890;
    }
    write_kernel_artifact(&kernel, &store).expect("write known kernel");
    println!(
        "WROTE kernel_id={} path={}",
        kernel.kernel_id,
        store.kernel_file_path(kernel.kernel_id).display()
    );

    let provisional = ungrounded_kernel();
    write_kernel_artifact(&provisional, &store).expect("write ungrounded kernel");
    println!(
        "WROTE kernel_id={} path={}",
        provisional.kernel_id,
        store.kernel_file_path(provisional.kernel_id).display()
    );
}
