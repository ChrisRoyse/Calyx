use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, CalyxWarning, CxId, FixedClock, Result as CalyxResult};
use calyx_ledger::{
    DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore,
    QuarantineSet, decode, get_answer_trace,
};
use calyx_lodestar::{
    Kernel, KernelGraphParams, KernelParams, build_kernel_index, build_kernel_pipeline_with_ledger,
    kernel_answer_with_ledger, kernel_members_hash,
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
        anchor_kind: Some("ph33-ledger-anchor".to_string()),
        corpus_shard_hash: [33; 32],
        built_at_millis: 1_785_500_000,
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
fn kernel_build_and_answer_append_real_ledger_refs() {
    let graph = ring_graph();
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10_000))
        .expect("open ledger");

    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel with ledger");
    let anchor = receipt.kernel.members[0];
    let query = third_query(anchor);
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");
    let answer = kernel_answer_with_ledger(
        &index,
        &graph,
        query,
        &[1.0, 0.0],
        &[anchor],
        3,
        &mut appender,
    )
    .expect("answer with ledger");

    let entries = appender.scan_entries().expect("scan ledger");
    let kinds: Vec<_> = entries.iter().map(|entry| entry.kind).collect();
    let seqs: Vec<_> = answer.provenance.iter().map(|ledger| ledger.seq).collect();
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        query.as_bytes(),
    )
    .expect("answer trace");

    println!(
        "PH33_LEDGER_PROVENANCE build_seq={} answer_seqs={seqs:?}",
        receipt.ledger_ref.seq
    );

    assert_eq!(receipt.ledger_ref.seq, 0);
    assert_eq!(kinds[0], EntryKind::Kernel);
    assert_eq!(kinds[1..], vec![EntryKind::Answer; answer.hops.len() + 1]);
    assert_eq!(entries.len(), 2 + answer.hops.len());
    assert_eq!(seqs, vec![1, 2, 3]);
    assert_eq!(
        answer.provenance,
        answer
            .hops
            .iter()
            .map(|hop| hop.ledger_ref.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(entries[0].entry_hash, receipt.ledger_ref.hash);
    for (entry, ledger_ref) in entries[1..=answer.hops.len()]
        .iter()
        .zip(&answer.provenance)
    {
        assert_eq!(entry.entry_hash, ledger_ref.hash);
        assert_eq!(entry.seq, ledger_ref.seq);
    }
    assert!(trace.is_trusted());
    assert_eq!(trace.path.len(), answer.hops.len());
    assert_eq!(
        trace.kernel_entry.as_ref().unwrap().seq,
        receipt.ledger_ref.seq
    );
    assert!(kernel_payload_has_members_hash(
        &entries[0].payload,
        &receipt.kernel
    ));
}

#[test]
fn ledger_append_failures_are_reported_as_calyx_codes() {
    let graph = ring_graph();
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(1))
        .expect("open ledger");
    let _ = build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 1, &mut appender)
        .expect("first append");
    appender
        .store_mut()
        .insert_raw(1, b"not-a-ledger-entry".to_vec());

    let error = build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 2, &mut appender)
        .unwrap_err();

    assert_eq!(error.code(), "CALYX_LEDGER_CORRUPT");
}

#[test]
fn mid_hop_append_failure_leaves_untrusted_partial_trace() {
    let graph = ring_graph();
    let store = FailOnSeqStore::new(MemoryLedgerStore::default(), 2);
    let mut appender = LedgerAppender::open(store, FixedClock::new(3_000)).expect("open ledger");
    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel");
    let anchor = receipt.kernel.members[0];
    let query = third_query(anchor);
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");

    let error = kernel_answer_with_ledger(
        &index,
        &graph,
        query,
        &[1.0, 0.0],
        &[anchor],
        3,
        &mut appender,
    )
    .unwrap_err();
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        query.as_bytes(),
    )
    .expect("trace partial answer");
    let answer_rows = appender
        .store()
        .scan()
        .unwrap()
        .into_iter()
        .filter(|row| decode(&row.bytes).unwrap().kind == EntryKind::Answer)
        .count();

    assert_eq!(error.code(), "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(answer_rows, 1);
    assert_eq!(trace.path.len(), 1);
    assert!(!trace.complete);
    assert!(!trace.is_trusted());
    assert_eq!(
        trace.warnings,
        vec![CalyxWarning::unprovenanced(
            "answer_trace.partial_or_unmarked"
        )]
    );
}

#[test]
#[ignore = "manual FSV for PH33 kernel/answer ledger provenance"]
fn ph33_kernel_ledger_provenance_manual_fsv() {
    let root = fsv_root().join("ph33-ledger-provenance");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    let before = DirectoryLedgerStore::open(&ledger_dir)
        .expect("open before ledger")
        .scan()
        .expect("scan before");
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open ledger"),
        FixedClock::new(1_785_500_000),
    )
    .expect("open appender");
    let graph = ring_graph();

    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel with ledger");
    let anchor = receipt.kernel.members[0];
    let query = third_query(anchor);
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");
    let answer = kernel_answer_with_ledger(
        &index,
        &graph,
        query,
        &[1.0, 0.0],
        &[anchor],
        3,
        &mut appender,
    )
    .expect("answer with ledger");
    let entries = appender.scan_entries().expect("scan entries");
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        query.as_bytes(),
    )
    .expect("answer trace");
    let readback = json!({
        "before_count": before.len(),
        "after_count": entries.len(),
        "kernel_entry_count": entries.iter().filter(|entry| entry.kind == EntryKind::Kernel).count(),
        "answer_entry_count": entries.iter().filter(|entry| entry.kind == EntryKind::Answer).count(),
        "trace_complete": trace.complete,
        "trace_trusted": trace.is_trusted(),
        "trace_path_len": trace.path.len(),
        "trace_answer_entry_seq": trace.answer_entry.as_ref().map(|entry| entry.seq),
        "trace_kernel_entry_seq": trace.kernel_entry.as_ref().map(|entry| entry.seq),
        "kernel_ledger_ref": receipt.ledger_ref,
        "answer_provenance": answer.provenance,
        "chain_ok": chain_ok(&entries),
        "payloads_secret_free": payloads_secret_free(&entries),
        "members_hash": hex(&kernel_members_hash(&receipt.kernel)),
        "ledger_dir": ledger_dir,
    });
    let readback_path = root.join("ph33-ledger-provenance-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    write_decoded_rows(&root, &entries);

    println!("PH33_LEDGER_FSV_ROOT={}", root.display());
    println!("PH33_LEDGER_READBACK={}", readback_path.display());
    println!("kernel ledger OK: 1 build entry, 3 answer hops, 1 complete answer row");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before.len(), 0);
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0].kind, EntryKind::Kernel);
    assert!(
        entries[1..]
            .iter()
            .all(|entry| entry.kind == EntryKind::Answer)
    );
    assert!(trace.is_trusted());
    assert!(chain_ok(&entries));
    assert!(payloads_secret_free(&entries));
}

#[test]
#[ignore = "manual FSV for PH36 T06 mid-hop append failure"]
fn ph36_audit_mid_hop_failure_manual_fsv() {
    let root = fsv_root().join("ph36-audit-mid-hop-failure");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .expect("open before ledger")
        .scan()
        .expect("scan before");
    let store = FailOnSeqStore::new(
        DirectoryLedgerStore::open(&ledger_dir).expect("open ledger"),
        2,
    );
    let mut appender =
        LedgerAppender::open(store, FixedClock::new(1_785_600_000)).expect("open appender");
    let graph = ring_graph();
    let receipt =
        build_kernel_pipeline_with_ledger(&graph, &[cx(10)], &params(), 44, &mut appender)
            .expect("build kernel");
    let anchor = receipt.kernel.members[0];
    let query = third_query(anchor);
    let index = build_kernel_index(&receipt.kernel, &embeddings(&receipt.kernel, anchor))
        .expect("build index");

    let answer_error = kernel_answer_with_ledger(
        &index,
        &graph,
        query,
        &[1.0, 0.0],
        &[anchor],
        3,
        &mut appender,
    )
    .unwrap_err();
    drop(appender);
    let disk_store = DirectoryLedgerStore::open(&ledger_dir).expect("reopen ledger");
    let after_rows = disk_store.scan().expect("scan after");
    let after_entries = after_rows
        .iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();
    let trace = get_answer_trace(&disk_store, &QuarantineSet::default(), query.as_bytes())
        .expect("trace partial");
    let readback = json!({
        "before_rows": before_rows.len(),
        "after_rows": after_rows.len(),
        "answer_error_code": answer_error.code(),
        "answer_entry_count": after_entries.iter().filter(|entry| entry.kind == EntryKind::Answer).count(),
        "trace_complete": trace.complete,
        "trace_trusted": trace.is_trusted(),
        "trace_warnings": trace.warnings,
        "trace_path_len": trace.path.len(),
        "ledger_dir": ledger_dir,
    });
    let readback_path = root.join("ph36-audit-mid-hop-failure-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    write_decoded_rows(&root, &after_entries);

    println!("PH36_AUDIT_MID_HOP_FSV_ROOT={}", root.display());
    println!("PH36_AUDIT_MID_HOP_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows.len(), 0);
    assert_eq!(after_entries.len(), 2);
    assert_eq!(answer_error.code(), "CALYX_LEDGER_CHAIN_BROKEN");
    assert!(!trace.complete);
    assert!(!trace.is_trusted());
}

fn third_query(anchor: CxId) -> CxId {
    match anchor {
        value if value == cx(10) => cx(13),
        value if value == cx(11) => cx(10),
        value if value == cx(12) => cx(11),
        _ => cx(12),
    }
}

fn kernel_payload_has_members_hash(payload: &[u8], kernel: &Kernel) -> bool {
    let value: serde_json::Value = serde_json::from_slice(payload).expect("payload json");
    value["members_hash"] == json!(hex(&kernel_members_hash(kernel)))
}

fn chain_ok(entries: &[calyx_ledger::LedgerEntry]) -> bool {
    entries
        .first()
        .is_some_and(|entry| entry.prev_hash == [0; 32])
        && entries
            .windows(2)
            .all(|pair| pair[1].prev_hash == pair[0].entry_hash)
}

fn payloads_secret_free(entries: &[calyx_ledger::LedgerEntry]) -> bool {
    entries.iter().all(|entry| {
        let lower = String::from_utf8_lossy(&entry.payload).to_ascii_lowercase();
        !["secret", "password", "token"]
            .iter()
            .any(|needle| lower.contains(needle))
    })
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
    let path = root.join("ph33-ledger-decoded-rows.json");
    fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}

#[derive(Debug)]
struct FailOnSeqStore<S> {
    inner: S,
    fail_seq: u64,
}

impl<S> FailOnSeqStore<S> {
    const fn new(inner: S, fail_seq: u64) -> Self {
        Self { inner, fail_seq }
    }
}

impl<S: LedgerCfStore> LedgerCfStore for FailOnSeqStore<S> {
    fn scan(&self) -> CalyxResult<Vec<LedgerRow>> {
        self.inner.scan()
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> CalyxResult<()> {
        if seq == self.fail_seq {
            return Err(CalyxError::ledger_chain_broken(
                "injected mid-hop append failure",
            ));
        }
        self.inner.put_new(seq, bytes)
    }
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-ledger-fsv"))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn directory_rows_decode_to_real_ledger_entries() {
    let root = fsv_root().join("ph33-ledger-directory-unit");
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
