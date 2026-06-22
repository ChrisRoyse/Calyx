#![cfg(feature = "fsv")]

#[allow(dead_code)]
#[path = "support/real_corpora.rs"]
mod real_corpora;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{
    AnswerTrace, DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry,
    LedgerRow, QuarantineSet, decode, get_answer_trace,
};
use calyx_lodestar::{
    AnswerPath, Kernel, RecallQuery, append_kernel_build_entry, build_kernel_index, kernel_answer,
    kernel_answer_with_ledger,
};
use serde::Serialize;

use real_corpora::{
    CorpusCase, STAMP, TunedKernelFixtureReport, calyx_home, embeddings_for_case, scifact_text,
    source_readback_json, tuned_kernel_fixture, write_json,
};

const MAX_ANSWER_HOPS: usize = 64;
const GRAPH_SEQ: u64 = 631;

#[derive(Clone, Debug)]
struct LedgerAnswerFixture {
    query: RecallQuery,
    anchors: Vec<CxId>,
    expected_answer: AnswerPath,
}

#[derive(Serialize)]
struct RealLedgerAnswerReport {
    stamp: &'static str,
    corpus_name: &'static str,
    modality: &'static str,
    source_readback: serde_json::Value,
    row_count: usize,
    graph_nodes: usize,
    graph_edges: usize,
    anchor_count: usize,
    kernel_id: CxId,
    kernel_member_count: usize,
    tuned_kernel: TunedKernelFixtureReport,
    query_cx: CxId,
    selected_anchor: CxId,
    hop_count: usize,
    before_ledger_rows: usize,
    after_ledger_rows: usize,
    kernel_ledger_seq: u64,
    answer_hop_ledger_seqs: Vec<u64>,
    complete_answer_seq: Option<u64>,
    trace_complete: bool,
    trace_trusted: bool,
    trace_path_len: usize,
    chain_ok: bool,
    answer_elapsed_micros: u64,
    ledger_dir: String,
    answer_artifact: String,
    decoded_rows_artifact: String,
    trace_artifact: String,
    manifest_artifact: String,
}

#[derive(Serialize)]
struct DecodedLedgerRow {
    seq: u64,
    row_file: String,
    row_bytes_len: usize,
    row_bytes_b3: String,
    row_bytes_hex: String,
    kind: &'static str,
    prev_hash: String,
    entry_hash: String,
    subject: calyx_ledger::SubjectId,
    actor: calyx_ledger::ActorId,
    payload: serde_json::Value,
}

#[test]
#[ignore = "manual FSV: real SciFact kernel_answer_with_ledger readback"]
fn ph33_real_kernel_answer_with_ledger_manual_fsv() {
    let home = calyx_home();
    let root = fsv_root(&home);
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");

    let case = scifact_text(&home);
    let (kernel, tuning) = tuned_kernel_fixture(&case);
    let embeddings = embeddings_for_case(&case);
    let index = build_kernel_index(&kernel, &embeddings).expect("kernel index");
    let fixture = select_fixture(&case, &kernel).expect("real non-direct answer fixture");

    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .expect("open before ledger")
        .scan()
        .expect("scan before");
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open ledger"),
        FixedClock::new(1_785_631_000),
    )
    .expect("open appender");
    let kernel_ref =
        append_kernel_build_entry(&mut appender, &kernel, GRAPH_SEQ).expect("append kernel row");

    let started = Instant::now();
    let answer = kernel_answer_with_ledger(
        &index,
        case.graph(),
        fixture.query.cx_id,
        &fixture.query.vector,
        &fixture.anchors,
        MAX_ANSWER_HOPS,
        &mut appender,
    )
    .expect("real answer with ledger");
    let answer_elapsed = started.elapsed();

    let after_rows = appender.store().scan().expect("scan after");
    let entries = after_rows
        .iter()
        .map(|row| decode(&row.bytes).expect("decode row"))
        .collect::<Vec<_>>();
    let trace = get_answer_trace(
        appender.store(),
        &QuarantineSet::default(),
        fixture.query.cx_id.as_bytes(),
    )
    .expect("answer trace");

    assert_eq!(before_rows.len(), 0);
    assert_eq!(
        answer.anchor_kernel_node,
        fixture.expected_answer.anchor_kernel_node
    );
    assert_eq!(answer.hops.len(), fixture.expected_answer.hops.len());
    assert_eq!(after_rows.len(), answer.hops.len() + 2);
    assert_eq!(entries[0].kind, EntryKind::Kernel);
    assert!(
        entries[1..]
            .iter()
            .all(|entry| entry.kind == EntryKind::Answer)
    );
    assert!(trace.is_trusted());
    assert_eq!(trace.path.len(), answer.hops.len());
    assert_eq!(trace.kernel_entry.as_ref().unwrap().seq, kernel_ref.seq);
    assert!(chain_ok(&entries));

    let answer_path = root.join("ph33-real-ledger-answer.json");
    let rows_path = root.join("ph33-real-ledger-decoded-rows.json");
    let trace_path = root.join("ph33-real-ledger-answer-trace.json");
    write_json(&answer_path, &answer);
    write_json(
        &rows_path,
        &decoded_rows(&ledger_dir, &after_rows, &entries),
    );
    write_json(&trace_path, &trace);

    let answer_readback: AnswerPath =
        serde_json::from_slice(&fs::read(&answer_path).expect("read answer")).unwrap();
    let trace_readback: AnswerTrace =
        serde_json::from_slice(&fs::read(&trace_path).expect("read trace")).unwrap();
    let rows_readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&rows_path).expect("read rows")).unwrap();
    assert_eq!(answer_readback, answer);
    assert_eq!(trace_readback, trace);
    assert_eq!(rows_readback.as_array().unwrap().len(), after_rows.len());

    let report = RealLedgerAnswerReport {
        stamp: STAMP,
        corpus_name: case.name,
        modality: case.modality(),
        source_readback: source_readback_json(&case),
        row_count: case.rows.len(),
        graph_nodes: case.graph().node_count(),
        graph_edges: case.graph().edge_count(),
        anchor_count: case.anchors().len(),
        kernel_id: kernel.kernel_id,
        kernel_member_count: index.rows().len(),
        tuned_kernel: tuning,
        query_cx: answer.query_cx,
        selected_anchor: answer.anchor_kernel_node,
        hop_count: answer.hops.len(),
        before_ledger_rows: before_rows.len(),
        after_ledger_rows: after_rows.len(),
        kernel_ledger_seq: kernel_ref.seq,
        answer_hop_ledger_seqs: answer.provenance.iter().map(|ledger| ledger.seq).collect(),
        complete_answer_seq: trace.answer_entry.as_ref().map(|entry| entry.seq),
        trace_complete: trace.complete,
        trace_trusted: trace.is_trusted(),
        trace_path_len: trace.path.len(),
        chain_ok: chain_ok(&entries),
        answer_elapsed_micros: micros(answer_elapsed),
        ledger_dir: ledger_dir.display().to_string(),
        answer_artifact: answer_path.display().to_string(),
        decoded_rows_artifact: rows_path.display().to_string(),
        trace_artifact: trace_path.display().to_string(),
        manifest_artifact: root.join("BLAKE3SUMS.txt").display().to_string(),
    };
    let report_path = root.join("ph33-real-ledger-answer-report.json");
    write_json(&report_path, &report);
    let manifest_path = write_manifest(
        &root,
        &[answer_path, rows_path, trace_path, report_path.clone()],
    );
    let manifest = fs::read_to_string(&manifest_path).expect("read manifest");
    assert!(manifest.contains("ph33-real-ledger-answer-report.json"));
    assert!(manifest.contains("ledger-cf/0000000000000000.ledger"));

    println!("PH33_REAL_LEDGER_ANSWER_FSV_ROOT={}", root.display());
    println!("PH33_REAL_LEDGER_ANSWER_REPORT={}", report_path.display());
    println!(
        "PH33_REAL_LEDGER_ANSWER_MANIFEST={}",
        manifest_path.display()
    );
}

fn select_fixture(case: &CorpusCase, kernel: &Kernel) -> Option<LedgerAnswerFixture> {
    let embeddings = embeddings_for_case(case);
    let index = build_kernel_index(kernel, &embeddings).ok()?;
    let anchors = case
        .anchors()
        .iter()
        .copied()
        .filter(|anchor| kernel.members.contains(anchor))
        .collect::<Vec<_>>();
    for query in &case.rows {
        let Ok(answer) = kernel_answer(
            &index,
            case.graph(),
            query.cx_id,
            &query.vector,
            &anchors,
            MAX_ANSWER_HOPS,
        ) else {
            continue;
        };
        if query.cx_id != answer.anchor_kernel_node && !answer.hops.is_empty() {
            return Some(LedgerAnswerFixture {
                query: query.clone(),
                anchors,
                expected_answer: answer,
            });
        }
    }
    None
}

fn decoded_rows(
    ledger_dir: &Path,
    rows: &[LedgerRow],
    entries: &[LedgerEntry],
) -> Vec<DecodedLedgerRow> {
    rows.iter()
        .zip(entries)
        .map(|(row, entry)| DecodedLedgerRow {
            seq: row.seq,
            row_file: ledger_dir
                .join(format!("{:016x}.ledger", row.seq))
                .display()
                .to_string(),
            row_bytes_len: row.bytes.len(),
            row_bytes_b3: hex(blake3::hash(&row.bytes).as_bytes()),
            row_bytes_hex: hex(&row.bytes),
            kind: entry.kind.as_str(),
            prev_hash: hex(&entry.prev_hash),
            entry_hash: hex(&entry.entry_hash),
            subject: entry.subject.clone(),
            actor: entry.actor.clone(),
            payload: serde_json::from_slice(&entry.payload).expect("payload json"),
        })
        .collect()
}

fn write_manifest(root: &Path, artifact_paths: &[PathBuf]) -> PathBuf {
    let mut paths = artifact_paths.to_vec();
    let ledger_dir = root.join("ledger-cf");
    for entry in fs::read_dir(&ledger_dir).expect("read ledger dir") {
        let path = entry.expect("ledger dir entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("ledger") {
            paths.push(path);
        }
    }
    paths.sort();
    let lines = paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).expect("read manifest path");
            let rel = path.strip_prefix(root).unwrap_or(path);
            format!("{}  {}", hex(blake3::hash(&bytes).as_bytes()), slash(rel))
        })
        .collect::<Vec<_>>();
    let manifest = root.join("BLAKE3SUMS.txt");
    fs::write(&manifest, format!("{}\n", lines.join("\n"))).expect("write manifest");
    manifest
}

fn chain_ok(entries: &[LedgerEntry]) -> bool {
    entries
        .first()
        .is_some_and(|entry| entry.prev_hash == [0; 32])
        && entries
            .windows(2)
            .all(|pair| pair[1].prev_hash == pair[0].entry_hash)
        && entries.iter().all(LedgerEntry::verify)
}

fn fsv_root(home: &Path) -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join("fsv").join("ph33-real-ledger-answer"))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn slash(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
