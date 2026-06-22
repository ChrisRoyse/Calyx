use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{
    DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, MemoryLedgerStore,
    QuarantineSet, decode, get_answer_trace,
};
use calyx_lodestar::{
    Kernel, KernelGraphParams, KernelParams, build_kernel_index, build_kernel_pipeline_with_ledger,
    kernel_answer_with_ledger,
};
use calyx_paths::AssocGraph;
use serde_json::json;

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
        anchor_kind: Some("ph33-direct-hit-ledger-anchor".to_string()),
        corpus_shard_hash: [47; 32],
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

#[test]
fn direct_hit_answer_appends_complete_answer_ledger_row() {
    let graph = ring_graph();
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(20_000))
        .expect("open ledger");
    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel");
    let anchor = receipt.kernel.members[0];
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");

    let answer = kernel_answer_with_ledger(
        &index,
        &graph,
        anchor,
        &[1.0, 0.0],
        &[anchor],
        0,
        &mut appender,
    )
    .expect("direct-hit answer with ledger");
    let entries = appender.scan_entries().expect("scan entries");
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        anchor.as_bytes(),
    )
    .expect("answer trace");
    let answer_payload: serde_json::Value =
        serde_json::from_slice(&entries[1].payload).expect("answer payload json");

    assert!(answer.hops.is_empty());
    assert!(answer.provenance.is_empty());
    assert_eq!(answer.total_score, 1.0);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, EntryKind::Kernel);
    assert_eq!(entries[1].kind, EntryKind::Answer);
    assert_eq!(answer_payload["complete"], true);
    assert_eq!(answer_payload["expected_hops"], 0);
    assert_eq!(answer_payload["path"].as_array().unwrap().len(), 0);
    assert!(trace.complete);
    assert!(trace.is_trusted());
    assert_eq!(trace.path.len(), 0);
    assert_eq!(trace.answer_entry.as_ref().unwrap().seq, 1);
}

#[test]
#[ignore = "manual FSV for #647 direct-hit answer ledger provenance"]
fn ph33_direct_hit_ledger_provenance_manual_fsv() {
    let root = fsv_root().join("ph33-direct-hit-ledger-provenance");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .expect("open before ledger")
        .scan()
        .expect("scan before");
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open ledger"),
        FixedClock::new(1_785_700_000),
    )
    .expect("open appender");
    let graph = ring_graph();
    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel");
    let anchor = receipt.kernel.members[0];
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");

    let answer = kernel_answer_with_ledger(
        &index,
        &graph,
        anchor,
        &[1.0, 0.0],
        &[anchor],
        0,
        &mut appender,
    )
    .expect("direct-hit answer with ledger");
    let entries = appender.scan_entries().expect("scan entries");
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        anchor.as_bytes(),
    )
    .expect("answer trace");
    let readback = json!({
        "before_rows": before_rows.len(),
        "after_entries": entries.len(),
        "kernel_entry_count": entries.iter().filter(|entry| entry.kind == EntryKind::Kernel).count(),
        "answer_entry_count": entries.iter().filter(|entry| entry.kind == EntryKind::Answer).count(),
        "direct_hit_query_id": anchor,
        "answer_hops": answer.hops,
        "answer_provenance": answer.provenance,
        "answer_total_score": answer.total_score,
        "trace_complete": trace.complete,
        "trace_trusted": trace.is_trusted(),
        "trace_path_len": trace.path.len(),
        "trace_answer_entry_seq": trace.answer_entry.as_ref().map(|entry| entry.seq),
        "trace_warnings": trace.warnings,
        "ledger_dir": ledger_dir,
    });
    let readback_path = root.join("ph33-direct-hit-ledger-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    write_decoded_rows(&root, &entries);

    println!("PH33_DIRECT_HIT_LEDGER_FSV_ROOT={}", root.display());
    println!(
        "PH33_DIRECT_HIT_LEDGER_READBACK={}",
        readback_path.display()
    );
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows.len(), 0);
    assert_eq!(entries.len(), 2);
    assert_eq!(readback["answer_entry_count"], 1);
    assert!(trace.is_trusted());
    assert!(trace.complete);
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-direct-hit-ledger-fsv"))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn write_decoded_rows(root: &Path, entries: &[calyx_ledger::LedgerEntry]) {
    let rows = entries
        .iter()
        .map(|entry| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<serde_json::Value>(&entry.payload).unwrap(),
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        root.join("ph33-direct-hit-ledger-decoded-rows.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn directory_rows_decode_to_real_entries() {
    let root = fsv_root().join("ph33-direct-hit-directory-unit");
    reset_dir(&root);
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(root.join("ledger-cf")).expect("open ledger"),
        FixedClock::new(77),
    )
    .expect("open appender");
    let receipt =
        build_kernel_pipeline_with_ledger(&ring_graph(), &[cx(10)], &params(), 3, &mut appender)
            .expect("append build");
    let rows = appender.store().scan().expect("scan rows");
    let decoded = decode(&rows[0].bytes).expect("decode row");

    assert_eq!(rows[0].seq, receipt.ledger_ref.seq);
    assert_eq!(decoded.entry_hash, receipt.ledger_ref.hash);
    assert_eq!(decoded.kind, EntryKind::Kernel);
}
