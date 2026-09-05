//! Issue #231 — shape-aware Loom weave for heterogeneous/sparse slots, FSV.
//!
//! Source of truth: durable AsterVault XTerm CF readback plus the persisted
//! shape-aware report. The corpus is intentionally minimal: two constellations
//! prove graph aggregation, two same-dim dense slots prove real LoomStore use,
//! two same-dim sparse slots prove sparse-safe agreement without densifying, and
//! one incompatible dense slot proves persisted unsupported-pair reasons.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    SparseEntry, VaultId, VaultStore,
};
use calyx_poly::loom_shape_weave::{
    SHAPE_AWARE_LOOM_WEAVE_SCHEMA_VERSION, read_shape_aware_loom_weave_report,
    run_shape_aware_loom_weave_for_cx_ids,
};
use calyx_poly::loom_weave::{ERR_LOOM_WEAVE_DUPLICATE_CX, ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const PANEL_VERSION: u32 = 1;
const VAULT_SALT: &[u8] = b"issue231-shape-loom-salt";

#[test]
fn issue231_shape_aware_loom_weave_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE231_FSV_ROOT", "poly-issue231-shape-loom");
    reset_dir(&root);

    let happy = happy_shape_aware_weave(&root);
    let zero_sparse = edge_zero_norm_sparse(&root);
    let multi_absent = edge_multi_and_absent(&root);
    let panel_mismatch = edge_panel_mismatch_fails_closed(&root);
    let duplicate = edge_duplicate_request_fails_closed(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 231,
        "proof_claim": "Poly can weave heterogeneous default-panel-like slots by using real LoomStore for same-dimension dense groups, sparse cosine for same-dimension sparse pairs without densifying, and persisted unsupported reasons for every non-materialized pair.",
        "minimum_sufficient_corpus": {
            "happy_path_constellations": 2,
            "slots_per_constellation": 5,
            "happy_path_xterm_rows": 4,
            "happy_path_unsupported_pairs": 16,
            "edge_cases": 4,
            "why_this_is_sufficient": "two constellations are the smallest corpus that proves agreement graph aggregation with n=2; two dense slots are the smallest real Loom group; two sparse slots are the smallest sparse agreement pair; one incompatible dense slot creates persisted unsupported pairs.",
            "why_smaller_is_insufficient": "one constellation would not prove graph mean aggregation; one dense or sparse slot cannot create a pair; omitting the incompatible slot would not prove no-silent-skip unsupported reporting.",
            "why_larger_is_wasteful": "additional constellations or slots repeat the same dense LoomStore, sparse cosine, XTerm CF write/readback, and unsupported-report paths without adding a distinct #231 invariant."
        },
        "source_of_truth": "AsterVault XTerm column family plus persisted shape-aware Loom weave report JSON",
        "happy_path": happy,
        "edge_cases": {
            "zero_norm_sparse": zero_sparse,
            "multi_and_absent": multi_absent,
            "panel_mismatch": panel_mismatch,
            "duplicate_request": duplicate
        },
        "physical_files": files
    });
    let summary_path = root.join("issue231_shape_aware_loom_weave_fsv_report.json");
    write_json(&summary_path, &summary);
    write_blake3sums(&root);
    println!(
        "ISSUE231_SHAPE_AWARE_LOOM_WEAVE_FSV={}",
        summary_path.display()
    );
}

fn happy_shape_aware_weave(root: &Path) -> Value {
    let case_dir = root.join("happy");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(
        &vault,
        vec![
            constellation(
                1,
                vec![
                    (0, dense(&[1.0, 0.0])),
                    (1, dense(&[1.0, 0.0])),
                    (2, sparse(8, &[(1, 1.0), (3, 1.0)])),
                    (3, sparse(8, &[(1, 1.0), (4, 1.0)])),
                    (4, dense(&[1.0, 0.0, 0.0])),
                ],
            ),
            constellation(
                2,
                vec![
                    (0, dense(&[1.0, 0.0])),
                    (1, dense(&[0.0, 1.0])),
                    (2, sparse(8, &[(1, 1.0), (3, 1.0)])),
                    (3, sparse(8, &[(4, 1.0), (5, 1.0)])),
                    (4, dense(&[0.0, 1.0, 0.0])),
                ],
            ),
        ],
    );

    let before = xterm_count(&vault);
    let run = run_shape_aware_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        PANEL_VERSION,
        &cx_ids,
        &case_dir.join("reports"),
        16,
    )
    .expect("shape-aware happy path");
    assert_eq!(
        run.report.schema_version,
        SHAPE_AWARE_LOOM_WEAVE_SCHEMA_VERSION
    );
    assert_eq!(run.report.xterm_count, 4);
    assert_eq!(run.report.unsupported_pair_count, 16);
    assert_eq!(xterm_count(&vault), 4);
    assert_eq!(
        read_shape_aware_loom_weave_report(&run.report_path).expect("read report"),
        run.report
    );
    assert_eq!(run.report.agreement_graph_order, vec!["0:1", "2:3"]);
    assert_eq!(run.report.agreement_graph[0].n, 2);
    assert_eq!(run.report.agreement_graph[1].n, 2);
    assert!(
        run.report
            .unsupported_pairs
            .iter()
            .any(|pair| pair.reason_code == "dense_dimension_mismatch")
    );
    assert!(
        run.report
            .unsupported_pairs
            .iter()
            .any(|pair| pair.reason_code == "heterogeneous_shape_pair")
    );

    let evidence = json!({
        "report_path": run.report_path.display().to_string(),
        "xterm_count_before": before,
        "xterm_count_after": xterm_count(&vault),
        "xterm_order": run.report.xterm_order,
        "agreement_graph_order": run.report.agreement_graph_order,
        "unsupported_pair_count": run.report.unsupported_pair_count,
        "persisted_seq": run.persisted_seq,
    });
    write_json(&case_dir.join("happy_summary.json"), &evidence);
    evidence
}

fn edge_zero_norm_sparse(root: &Path) -> Value {
    let case_dir = root.join("edge-zero-sparse");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(
        &vault,
        vec![constellation(
            10,
            vec![(0, sparse(8, &[])), (1, sparse(8, &[(2, 1.0)]))],
        )],
    );
    let run = run_shape_aware_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        PANEL_VERSION,
        &cx_ids,
        &case_dir.join("reports"),
        4,
    )
    .expect("zero sparse reports unsupported");
    assert_eq!(run.report.xterm_count, 0);
    assert_eq!(run.report.unsupported_pairs[0].reason_code, "zero_norm");
    assert_eq!(xterm_count(&vault), 0);
    write_json(
        &case_dir.join("edge_zero_sparse.json"),
        &json!({"unsupported": run.report.unsupported_pairs, "xterm_count": run.report.xterm_count}),
    );
    json!({"xterm_count": 0, "reason": "zero_norm"})
}

fn edge_multi_and_absent(root: &Path) -> Value {
    let case_dir = root.join("edge-multi-absent");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(
        &vault,
        vec![constellation(
            11,
            vec![
                (
                    0,
                    SlotVector::Multi {
                        token_dim: 2,
                        tokens: vec![vec![1.0, 0.0]],
                    },
                ),
                (
                    1,
                    SlotVector::Absent {
                        reason: AbsentReason::Deferred,
                    },
                ),
            ],
        )],
    );
    let run = run_shape_aware_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        PANEL_VERSION,
        &cx_ids,
        &case_dir.join("reports"),
        4,
    )
    .expect("multi/absent reports unsupported");
    assert_eq!(run.report.xterm_count, 0);
    assert_eq!(
        run.report.unsupported_pairs[0].reason_code,
        "multi_vector_unsupported"
    );
    json!({"xterm_count": 0, "reason": run.report.unsupported_pairs[0].reason_code})
}

fn edge_panel_mismatch_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-panel-mismatch");
    let vault = open_vault(&case_dir.join("vault"));
    let mut mismatched = constellation(12, vec![(0, dense(&[1.0, 0.0])), (1, dense(&[0.0, 1.0]))]);
    mismatched.panel_version = PANEL_VERSION + 1;
    let cx_ids = put_all(&vault, vec![mismatched]);
    let err = run_shape_aware_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        PANEL_VERSION,
        &cx_ids,
        &case_dir.join("reports"),
        4,
    )
    .expect_err("panel mismatch must fail closed");
    assert_eq!(err.code(), ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH);
    assert_eq!(xterm_count(&vault), 0);
    write_json(
        &case_dir.join("edge_panel_mismatch.json"),
        &json!({"code": err.code(), "message": err.message(), "xterm_count": 0}),
    );
    json!({"code": err.code(), "xterm_count": 0})
}

fn edge_duplicate_request_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-duplicate");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(
        &vault,
        vec![constellation(13, vec![(0, dense(&[1.0, 0.0]))])],
    );
    let err = run_shape_aware_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        PANEL_VERSION,
        &[cx_ids[0], cx_ids[0]],
        &case_dir.join("reports"),
        4,
    )
    .expect_err("duplicate request must fail closed");
    assert_eq!(err.code(), ERR_LOOM_WEAVE_DUPLICATE_CX);
    assert_eq!(xterm_count(&vault), 0);
    json!({"code": err.code(), "xterm_count": 0})
}

fn put_all(vault: &AsterVault, constellations: Vec<Constellation>) -> Vec<CxId> {
    let mut ids = Vec::new();
    for constellation in constellations {
        let id = vault.put(constellation).expect("put constellation");
        ids.push(id);
    }
    vault.flush().expect("flush constellation source rows");
    ids
}

fn constellation(seed: u8, slots: Vec<(u16, SlotVector)>) -> Constellation {
    let mut slot_map = BTreeMap::new();
    for (slot, vector) in slots {
        slot_map.insert(SlotId::new(slot), vector);
    }
    let cx_id = CxId::from_bytes([seed; 16]);
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: PANEL_VERSION,
        created_at: 1_785_500_231_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: slot_map,
        scalars: BTreeMap::from([("fixture_seed".to_string(), f64::from(seed))]),
        metadata: BTreeMap::from([("fixture".to_string(), format!("issue231-{seed}"))]),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    }
}

fn dense(values: &[f32]) -> SlotVector {
    SlotVector::Dense {
        dim: values.len() as u32,
        data: values.to_vec(),
    }
}

fn sparse(dim: u32, entries: &[(u32, f32)]) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable test vault")
}

fn xterm_count(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::XTerm)
        .expect("scan xterm")
        .len()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
