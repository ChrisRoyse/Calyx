use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, QuarantineSet, decode, get_answer_trace,
};
use calyx_lodestar::{
    AnswerPath, Kernel, KernelGraphParams, KernelParams, build_kernel_index,
    build_kernel_pipeline_with_ledger, kernel_answer_with_ledger,
};
use calyx_paths::AssocGraph;
use serde_json::{Value, json};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn ring_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for seed in [10, 11, 12, 13] {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    builder
        .add_edge(cx(10), cx(11), 1.0)
        .unwrap()
        .add_edge(cx(11), cx(12), 0.9)
        .unwrap()
        .add_edge(cx(12), cx(13), 0.8)
        .unwrap()
        .add_edge(cx(13), cx(10), 0.7)
        .unwrap();
    builder.build()
}

fn params() -> KernelParams {
    KernelParams {
        panel_version: 33,
        anchor_kind: Some("issue920-direct-hit-ledger-anchor".to_string()),
        corpus_shard_hash: [92; 32],
        built_at_millis: 1_785_700_000,
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 4,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn embeddings(kernel: &Kernel, anchor: CxId) -> BTreeMap<CxId, Vec<f32>> {
    kernel
        .members
        .iter()
        .map(|member| {
            let vector = if *member == anchor {
                vec![1.0, 0.0]
            } else {
                vec![0.0, 1.0]
            };
            (*member, vector)
        })
        .collect()
}

fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        PathBuf::from("target/fsv/issue920-direct-hit-provenance")
    })
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decoded_rows(ledger_dir: &Path) -> Vec<Value> {
    DirectoryLedgerStore::open(ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .into_iter()
        .map(|row| {
            let entry = decode(&row.bytes).unwrap();
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn issue920_direct_hit_answer_returns_physical_complete_answer_ref() {
    let root = fsv_root();
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    let returned_answer_path = root.join("issue920-returned-answer.json");
    let decoded_rows_path = root.join("issue920-ledger-decoded-rows.json");
    let readback_path = root.join("issue920-direct-hit-provenance-readback.json");

    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(1_785_700_000),
    )
    .unwrap();
    let graph = ring_graph();
    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender).unwrap();
    let anchor = receipt.kernel.members[0];
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor)).unwrap();

    let answer = kernel_answer_with_ledger(
        &index,
        &graph,
        anchor,
        &[1.0, 0.0],
        &[anchor],
        0,
        &mut appender,
    )
    .unwrap();
    write_json(
        &returned_answer_path,
        &serde_json::to_value(&answer).unwrap(),
    );
    let returned_answer: AnswerPath =
        serde_json::from_slice(&fs::read(&returned_answer_path).unwrap()).unwrap();
    let after_happy_entries = appender.scan_entries().unwrap();
    let physical_rows = decoded_rows(&ledger_dir);
    write_json(&decoded_rows_path, &json!(physical_rows));
    let physical_rows_readback: Vec<Value> =
        serde_json::from_slice(&fs::read(&decoded_rows_path).unwrap()).unwrap();
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        anchor.as_bytes(),
    )
    .unwrap();
    let answer_entry = &after_happy_entries[1];
    let answer_payload: Value = serde_json::from_slice(&answer_entry.payload).unwrap();

    assert!(returned_answer.hops.is_empty());
    assert_eq!(returned_answer.provenance.len(), 1);
    assert_eq!(returned_answer.provenance[0].seq, answer_entry.seq);
    assert_eq!(returned_answer.provenance[0].hash, answer_entry.entry_hash);
    assert_eq!(
        physical_rows_readback[1]["entry_hash"],
        hex(&answer_entry.entry_hash)
    );
    assert_eq!(answer_payload["complete"], true);
    assert_eq!(answer_payload["expected_hops"], 0);
    assert_eq!(answer_payload["path"].as_array().unwrap().len(), 0);
    assert!(trace.complete);
    assert!(trace.is_trusted());
    assert_eq!(trace.answer_entry.as_ref().unwrap().seq, answer_entry.seq);

    let rows_before_edges = appender.scan_entries().unwrap().len();
    let missing_anchor_error =
        kernel_answer_with_ledger(&index, &graph, anchor, &[1.0, 0.0], &[], 0, &mut appender)
            .unwrap_err();
    let rows_after_missing_anchor = appender.scan_entries().unwrap().len();
    let dim_error =
        kernel_answer_with_ledger(&index, &graph, anchor, &[1.0], &[anchor], 0, &mut appender)
            .unwrap_err();
    let rows_after_dim = appender.scan_entries().unwrap().len();
    let max_hops_error = kernel_answer_with_ledger(
        &index,
        &graph,
        cx(12),
        &[1.0, 0.0],
        &[anchor],
        0,
        &mut appender,
    )
    .unwrap_err();
    let rows_after_max_hops = appender.scan_entries().unwrap().len();

    assert_eq!(missing_anchor_error.code(), "CALYX_KERNEL_NO_ANCHORED_NODE");
    assert_eq!(dim_error.code(), "CALYX_KERNEL_DIM_MISMATCH");
    assert_eq!(max_hops_error.code(), "CALYX_PATHS_MAX_HOPS");
    assert_eq!(rows_before_edges, rows_after_missing_anchor);
    assert_eq!(rows_before_edges, rows_after_dim);
    assert_eq!(rows_before_edges, rows_after_max_hops);

    let readback = json!({
        "source_of_truth": "DirectoryLedgerStore rows plus returned AnswerPath JSON read from disk",
        "ledger_dir": ledger_dir.display().to_string(),
        "before_rows": before_rows.len(),
        "after_happy_rows": after_happy_entries.len(),
        "returned_answer_path": returned_answer_path.display().to_string(),
        "decoded_rows_path": decoded_rows_path.display().to_string(),
        "direct_hit": {
            "query_id": anchor,
            "hops": returned_answer.hops,
            "provenance": returned_answer.provenance,
            "complete_answer_seq": answer_entry.seq,
            "complete_answer_hash": hex(&answer_entry.entry_hash),
            "payload_complete": answer_payload["complete"],
            "payload_expected_hops": answer_payload["expected_hops"],
            "trace_complete": trace.complete,
            "trace_trusted": trace.is_trusted(),
            "trace_answer_entry_seq": trace.answer_entry.as_ref().map(|entry| entry.seq),
        },
        "edge_cases": [
            {
                "case": "missing_anchor_nodes",
                "before_rows": rows_before_edges,
                "after_rows": rows_after_missing_anchor,
                "code": missing_anchor_error.code(),
                "message": missing_anchor_error.to_string(),
            },
            {
                "case": "query_vector_dim_mismatch",
                "before_rows": rows_before_edges,
                "after_rows": rows_after_dim,
                "code": dim_error.code(),
                "message": dim_error.to_string(),
            },
            {
                "case": "direct_hit_not_possible_with_max_hops_zero",
                "before_rows": rows_before_edges,
                "after_rows": rows_after_max_hops,
                "code": max_hops_error.code(),
                "message": max_hops_error.to_string(),
            },
        ],
        "physical_rows": physical_rows_readback,
    });
    write_json(&readback_path, &readback);
    let stored: Value = serde_json::from_slice(&fs::read(&readback_path).unwrap()).unwrap();
    assert_eq!(stored, readback);
    println!("ISSUE920_READBACK={}", readback_path.display());
}
