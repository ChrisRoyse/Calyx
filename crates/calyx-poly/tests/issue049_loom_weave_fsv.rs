//! Issue #49 - Loom weave integration.
//!
//! Source of truth: durable AsterVault Base/XTerm column families plus the
//! persisted Loom agreement-graph report read back from disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SparseEntry, VaultId,
    VaultStore,
};
use calyx_loom::agreement_graph::XtermRow;
use calyx_loom::{CALYX_LOOM_DIM_MISMATCH, CALYX_LOOM_ZERO_NORM_VECTOR, CrossTermValue};
use calyx_poly::loom_weave::{
    ERR_LOOM_WEAVE_NON_DENSE_SLOT, ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH,
    LOOM_WEAVE_SCHEMA_VERSION, LoomWeaveReport, read_loom_weave_report, run_loom_weave_for_cx_ids,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const PANEL_VERSION: u32 = 1;
const VAULT_SALT: &[u8] = b"issue049-loom-weave-salt";

#[test]
fn issue049_loom_weave_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE049_FSV_ROOT", "poly-issue049-loom-weave");
    reset_dir(&root);

    let happy = happy_path(&root);
    let panel_mismatch = edge_error(
        &root,
        "edge-panel-version-mismatch",
        vec![constellation(
            10,
            PANEL_VERSION + 1,
            vec![(0, dense(&[1.0, 0.0])), (1, dense(&[0.0, 1.0]))],
        )],
        PANEL_VERSION,
        ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH,
    );
    let non_dense = edge_error(
        &root,
        "edge-non-dense-slot",
        vec![constellation(
            11,
            PANEL_VERSION,
            vec![
                (0, dense(&[1.0, 0.0])),
                (
                    1,
                    SlotVector::Sparse {
                        dim: 8,
                        entries: vec![SparseEntry { idx: 3, val: 1.0 }],
                    },
                ),
            ],
        )],
        PANEL_VERSION,
        ERR_LOOM_WEAVE_NON_DENSE_SLOT,
    );
    let dim_mismatch = edge_error(
        &root,
        "edge-dimension-mismatch",
        vec![constellation(
            12,
            PANEL_VERSION,
            vec![(0, dense(&[1.0, 0.0])), (1, dense(&[1.0, 0.0, 0.0]))],
        )],
        PANEL_VERSION,
        CALYX_LOOM_DIM_MISMATCH,
    );
    let zero_norm = edge_error(
        &root,
        "edge-zero-norm",
        vec![constellation(
            13,
            PANEL_VERSION,
            vec![(0, dense(&[0.0, 0.0])), (1, dense(&[1.0, 0.0]))],
        )],
        PANEL_VERSION,
        CALYX_LOOM_ZERO_NORM_VECTOR,
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 49,
        "proof_claim": "Poly can read stored constellations from the durable vault, run the real calyx-loom weave over their dense slots, persist the resulting agreement XTerm rows into Aster's XTerm CF, read those rows back, and expose the deterministic agreement graph/report.",
        "minimum_sufficient_corpus": {
            "happy_path_stored_constellations": 2,
            "dense_slots_per_constellation": 3,
            "happy_path_xterm_rows": 6,
            "edge_cases": 4,
            "why_this_is_sufficient": "two constellations are the smallest corpus that proves agreement graph aggregation with n=2; three dense slots are the smallest slot set that proves C(3,2) multi-edge pair enumeration and deterministic graph ordering; four small edge fixtures prove fail-closed behavior before any XTerm CF write.",
            "why_smaller_is_insufficient": "one constellation would only prove row creation, not graph mean aggregation; two slots would produce only one edge and would not prove pair-order coverage across a graph; omitting edge fixtures would not prove the no-fallback failure contract.",
            "why_larger_is_wasteful": "additional constellations or slots would repeat the same Base readback, Loom weave, XTerm CF write/readback, and graph exposure paths without adding a distinct #49 invariant."
        },
        "source_of_truth": "AsterVault Base and XTerm column families plus persisted Loom weave report JSON",
        "happy_path": happy,
        "edge_cases": {
            "panel_version_mismatch": panel_mismatch,
            "non_dense_slot": non_dense,
            "dimension_mismatch": dim_mismatch,
            "zero_norm": zero_norm
        },
        "physical_files": files
    });
    let summary_path = root.join("issue049_loom_weave_fsv_report.json");
    write_json(&summary_path, &summary);
    write_blake3sums(&root);
    println!("ISSUE049_LOOM_WEAVE_FSV={}", summary_path.display());
}

fn happy_path(root: &Path) -> Value {
    let case_dir = root.join("happy");
    let report_dir = case_dir.join("reports");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(
        &vault,
        vec![
            constellation(
                1,
                PANEL_VERSION,
                vec![
                    (0, dense(&[1.0, 0.0])),
                    (1, dense(&[1.0, 0.0])),
                    (2, dense(&[0.0, 1.0])),
                ],
            ),
            constellation(
                2,
                PANEL_VERSION,
                vec![
                    (0, dense(&[1.0, 0.0])),
                    (1, dense(&[0.6, 0.8])),
                    (2, dense(&[0.0, 1.0])),
                ],
            ),
        ],
    );

    let before = source_counts(&vault);
    let run = run_loom_weave_for_cx_ids(&vault, "crypto", PANEL_VERSION, &cx_ids, &report_dir, 16)
        .expect("loom weave happy path");
    let after = source_counts(&vault);
    assert_eq!(run.report.schema_version, LOOM_WEAVE_SCHEMA_VERSION);
    assert_eq!(run.report.constellation_count, 2);
    assert_eq!(run.report.measured_slot_count, 6);
    assert_eq!(run.report.xterm_count, 6);
    assert_eq!(run.report.agreement_graph_order, vec!["0:1", "0:2", "1:2"]);
    assert_eq!(run.report.xterm_order.len(), 6);
    let mut sorted_order = run.report.xterm_order.clone();
    sorted_order.sort();
    assert_eq!(run.report.xterm_order, sorted_order);

    assert_edge(&run.report, 0, 1, 0.8, 0.8, 2);
    assert_edge(&run.report, 0, 2, 0.0, 0.0, 2);
    assert_edge(&run.report, 1, 2, 0.4, 0.4, 2);
    assert_xterm_value(&run.report.xterm_rows, cx_ids[0], 0, 1, 1.0);
    assert_xterm_value(&run.report.xterm_rows, cx_ids[0], 0, 2, 0.0);
    assert_xterm_value(&run.report.xterm_rows, cx_ids[1], 0, 1, 0.6);
    assert_xterm_value(&run.report.xterm_rows, cx_ids[1], 1, 2, 0.8);

    let report_readback = read_loom_weave_report(&run.report_path).expect("report readback");
    assert_eq!(report_readback, run.report);
    let cf_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::XTerm)
        .expect("scan xterm CF");
    assert_eq!(cf_rows.len(), 6);
    let mut decoded_rows = cf_rows
        .iter()
        .map(|(_, value)| serde_json::from_slice::<XtermRow>(value).expect("decode xterm row"))
        .collect::<Vec<_>>();
    decoded_rows.sort_by_key(|row| row.key);
    assert_eq!(decoded_rows, run.report.xterm_rows);

    let cf_key_order = cf_rows
        .iter()
        .map(|(key, _)| support::hex(key))
        .collect::<Vec<_>>();
    let report_bytes = fs::read(&run.report_path).expect("read report bytes");
    let evidence = json!({
        "trigger": "weave two stored constellations with three dense slots each",
        "before": before,
        "after": after,
        "cx_ids": cx_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "persisted_seq": run.persisted_seq,
        "report_path": run.report_path.display().to_string(),
        "report_blake3": blake3::hash(&report_bytes).to_hex().to_string(),
        "xterm_cf_key_order": cf_key_order,
        "xterm_cf_decoded_rows": decoded_rows,
        "report_readback": report_readback,
        "expected_graph_means": {
            "0:1": 0.8,
            "0:2": 0.0,
            "1:2": 0.4
        }
    });
    write_json(&case_dir.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_error(
    root: &Path,
    name: &str,
    constellations: Vec<calyx_core::Constellation>,
    requested_panel_version: u32,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(name);
    let report_dir = case_dir.join("reports");
    let vault = open_vault(&case_dir.join("vault"));
    let cx_ids = put_all(&vault, constellations);
    let before = source_counts(&vault);
    let err = run_loom_weave_for_cx_ids(
        &vault,
        "crypto",
        requested_panel_version,
        &cx_ids,
        &report_dir,
        4,
    )
    .expect_err("edge case must fail closed");
    assert_eq!(err.code(), expected_code);
    let after = source_counts(&vault);
    assert_eq!(before["xterm_count"], after["xterm_count"]);
    let report_path = report_dir.join(format!("loom_weave_crypto_v{requested_panel_version}.json"));
    assert!(!report_path.exists());
    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": err.code(),
        "message": err.message(),
        "cx_ids": cx_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "before": before,
        "after": after,
        "report_present": report_path.exists()
    });
    write_json(&case_dir.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn put_all(vault: &AsterVault, constellations: Vec<calyx_core::Constellation>) -> Vec<CxId> {
    let mut out = Vec::new();
    for constellation in constellations {
        out.push(vault.put(constellation).expect("put constellation"));
    }
    vault.flush().expect("flush constellation source rows");
    out
}

fn constellation(
    seed: u8,
    panel_version: u32,
    slots: Vec<(u16, SlotVector)>,
) -> calyx_core::Constellation {
    let cx_id = CxId::from_bytes([seed; 16]);
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version,
        created_at: 1_785_500_049_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: slots
            .into_iter()
            .map(|(slot, vector)| (SlotId::new(slot), vector))
            .collect::<BTreeMap<_, _>>(),
        scalars: BTreeMap::from([("fixture_seed".to_string(), f64::from(seed))]),
        metadata: BTreeMap::from([("fixture".to_string(), format!("issue049-{seed}"))]),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
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

fn assert_edge(
    report: &LoomWeaveReport,
    a: u16,
    b: u16,
    expected_mean: f32,
    expected_weight: f32,
    expected_n: usize,
) {
    let edge = report
        .agreement_graph
        .iter()
        .find(|edge| edge.a == SlotId::new(a) && edge.b == SlotId::new(b))
        .unwrap_or_else(|| panic!("missing agreement edge {a}:{b}"));
    assert_close(edge.raw_mean_agreement, expected_mean);
    assert_close(edge.mean_agreement, expected_mean);
    assert_close(edge.agreement_weight, expected_weight);
    assert_eq!(edge.n, expected_n);
}

fn assert_xterm_value(rows: &[XtermRow], cx_id: CxId, a: u16, b: u16, expected: f32) {
    let row = rows
        .iter()
        .find(|row| {
            row.key.cx_id == cx_id && row.key.a == SlotId::new(a) && row.key.b == SlotId::new(b)
        })
        .unwrap_or_else(|| panic!("missing xterm row {cx_id}:{a}:{b}"));
    match row.value {
        CrossTermValue::Scalar(actual) => assert_close(actual, expected),
        CrossTermValue::Vector(_) => panic!("expected scalar agreement xterm"),
    }
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "actual {actual} expected {expected}"
    );
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

fn source_counts(vault: &AsterVault) -> Value {
    json!({
        "base_count": vault.scan_cf_at(vault.snapshot(), ColumnFamily::Base).expect("scan base").len(),
        "xterm_count": vault.scan_cf_at(vault.snapshot(), ColumnFamily::XTerm).expect("scan xterm").len()
    })
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
