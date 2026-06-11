use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use calyx_core::CxId;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub enum CorpusSource {
    Synthetic,
    Scifact(PathBuf),
}

#[derive(Clone, Debug)]
pub struct CorpusDoc {
    pub doc_id: String,
    pub query_id: String,
    pub query: String,
    pub title: String,
    pub text: String,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct CorpusFixture {
    pub docs: Vec<CorpusDoc>,
    pub readback: Value,
}

pub fn load_corpus(source: &CorpusSource, limit: usize) -> CorpusFixture {
    match source {
        CorpusSource::Synthetic => synthetic_fixture(limit),
        CorpusSource::Scifact(dir) => scifact_fixture(dir, limit),
    }
}

pub fn readback_bundle(
    calyx_bin: &Path,
    vault_dir: &Path,
    recurrence_cx: CxId,
    ledger_end: u64,
) -> Value {
    let vault = display(vault_dir);
    let cx = recurrence_cx.to_string();
    let range = format!("0..{ledger_end}");
    json!({
        "vault_tree": command_stdout(calyx_bin, &["readback", "--vault-tree", &vault]),
        "base": command_stdout(calyx_bin, &["readback", "--cf", "base", "--vault", &vault]),
        "anchors": command_stdout(calyx_bin, &["readback", "--cf", "anchors", "--vault", &vault]),
        "slot_00": command_stdout(calyx_bin, &["readback", "--cf", "slot_00", "--vault", &vault]),
        "slot_01": command_stdout(calyx_bin, &["readback", "--cf", "slot_01", "--vault", &vault]),
        "slot_02": command_stdout(calyx_bin, &["readback", "--cf", "slot_02", "--vault", &vault]),
        "slot_03": command_stdout(calyx_bin, &["readback", "--cf", "slot_03", "--vault", &vault]),
        "assay": command_stdout(calyx_bin, &["readback", "--cf", "assay", "--vault", &vault]),
        "online": command_stdout(calyx_bin, &["readback", "--cf", "online", "--vault", &vault]),
        "recurrence": command_stdout(calyx_bin, &["readback", "--cf", "recurrence", "--vault", &vault]),
        "ledger": command_stdout(calyx_bin, &["readback", "--cf", "ledger", "--vault", &vault]),
        "wal": command_stdout(calyx_bin, &["readback", "--wal", "--vault", &vault]),
        "scan_ledger": command_stdout(calyx_bin, &["scan", "--cf", "ledger", "--vault", &vault]),
        "verify_chain": command_stdout(calyx_bin, &["verify-chain", "--vault", &vault, "--range", &range]),
        "merkle_root": command_stdout(calyx_bin, &["merkle-root", "--vault", &vault, "--range", &range]),
        "recurrence_series": command_json(calyx_bin, &[
            "readback", "recurrence-series", "--vault", &vault, "--cx-id", &cx
        ]),
        "time_prediction": command_json(calyx_bin, &[
            "readback", "time-prediction", "--vault", &vault, "--cx-id", &cx,
            "--confidence-ceiling", "0.91"
        ]),
    })
}

pub fn reset_dir(root: &Path) {
    if root.exists() {
        fs::remove_dir_all(root).expect("reset fsv root");
    }
    fs::create_dir_all(root).expect("create fsv root");
}

pub fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).expect("write json");
}

pub fn write_text(path: &Path, value: &str) {
    fs::write(path, value).expect("write text");
}

pub fn write_readback_files(root: &Path, readbacks: &Value) {
    let object = readbacks.as_object().expect("readback object");
    for (name, value) in object {
        if let Some(text) = value.as_str() {
            write_text(&root.join(format!("readback-{name}.txt")), text);
        }
    }
}

pub fn write_blake3_manifest(root: &Path) {
    let manifest = root.join("fsv-b3sum-manifest.txt");
    let mut files = Vec::new();
    collect_files(root, &manifest, &mut files);
    files.sort();
    let mut lines = Vec::new();
    for path in files {
        let bytes = fs::read(&path).expect("hash evidence file");
        let rel = path.strip_prefix(root).unwrap_or(&path).display();
        lines.push(format!("{}  {}", blake3::hash(&bytes).to_hex(), rel));
    }
    write_text(&manifest, &format!("{}\n", lines.join("\n")));
}

pub fn list_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_all(root, &mut files);
    files.sort();
    files.into_iter().map(|path| display(&path)).collect()
}

pub fn display(path: &Path) -> String {
    path.display().to_string()
}

fn synthetic_fixture(limit: usize) -> CorpusFixture {
    let docs = [
        (
            "q0",
            "d0",
            "Does 2+2 equal 4?",
            "Arithmetic",
            "2+2=4 in integer arithmetic.",
            1.0,
        ),
        (
            "q1",
            "d1",
            "Is water dry?",
            "Water",
            "Liquid water is wet under normal conditions.",
            0.0,
        ),
        (
            "q2",
            "d2",
            "Does fire need oxygen?",
            "Fire",
            "Combustion commonly needs oxygen.",
            1.0,
        ),
        (
            "q3",
            "d3",
            "Can ice be hot?",
            "Ice",
            "Ice is cold water in solid form.",
            0.0,
        ),
    ];
    let docs = docs
        .into_iter()
        .take(limit)
        .map(|(query_id, doc_id, query, title, text, score)| CorpusDoc {
            doc_id: doc_id.to_string(),
            query_id: query_id.to_string(),
            query: query.to_string(),
            title: title.to_string(),
            text: text.to_string(),
            score,
        })
        .collect::<Vec<_>>();
    CorpusFixture {
        readback: json!({
            "source": "synthetic-known-io",
            "expected_docs": docs.len(),
            "hand_expected": "2+2=4 doc is relevant; water-dry doc is non-relevant",
        }),
        docs,
    }
}

fn scifact_fixture(dir: &Path, limit: usize) -> CorpusFixture {
    let corpus = dir.join("corpus.jsonl");
    let queries = dir.join("queries.jsonl");
    let qrels = dir.join("qrels").join("test.tsv");
    let qrel_rows = read_qrels(&qrels, limit);
    let query_ids = qrel_rows
        .iter()
        .map(|(query_id, _, _)| query_id.clone())
        .collect::<BTreeSet<_>>();
    let doc_ids = qrel_rows
        .iter()
        .map(|(_, doc_id, _)| doc_id.clone())
        .collect::<BTreeSet<_>>();
    let query_map = read_jsonl_text_map(&queries, &query_ids, "text");
    let doc_map = read_corpus_map(&corpus, &doc_ids);
    let mut docs = Vec::new();
    for (query_id, doc_id, score) in qrel_rows {
        let query = query_map.get(&query_id).expect("query id present").clone();
        let (title, text) = doc_map.get(&doc_id).expect("corpus id present").clone();
        docs.push(CorpusDoc {
            doc_id,
            query_id,
            query,
            title,
            text,
            score,
        });
    }
    CorpusFixture {
        docs,
        readback: json!({
            "source": "beir-scifact",
            "dataset_dir": display(dir),
            "corpus": file_digest(&corpus),
            "queries": file_digest(&queries),
            "qrels": file_digest(&qrels),
        }),
    }
}

fn read_qrels(path: &Path, limit: usize) -> Vec<(String, String, f64)> {
    fs::read_to_string(path)
        .expect("read qrels")
        .lines()
        .skip(1)
        .filter_map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            (parts.len() == 3).then(|| {
                (
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].parse::<f64>().expect("qrel score"),
                )
            })
        })
        .take(limit)
        .collect()
}

fn read_jsonl_text_map(
    path: &Path,
    ids: &BTreeSet<String>,
    field: &str,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in fs::read_to_string(path).expect("read jsonl").lines() {
        let value: Value = serde_json::from_str(line).expect("parse jsonl");
        let id = value["_id"].as_str().expect("_id").to_string();
        if ids.contains(&id) {
            out.insert(id, value[field].as_str().expect(field).to_string());
        }
    }
    out
}

fn read_corpus_map(path: &Path, ids: &BTreeSet<String>) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for line in fs::read_to_string(path).expect("read corpus").lines() {
        let value: Value = serde_json::from_str(line).expect("parse corpus");
        let id = value["_id"].as_str().expect("_id").to_string();
        if ids.contains(&id) {
            let title = value["title"].as_str().unwrap_or_default().to_string();
            let text = value["text"].as_str().unwrap_or_default().to_string();
            out.insert(id, (title, text));
        }
    }
    out
}

fn command_json(calyx_bin: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&command_stdout(calyx_bin, args)).expect("parse command json")
}

fn command_stdout(calyx_bin: &Path, args: &[&str]) -> String {
    let output = Command::new(calyx_bin)
        .args(args)
        .output()
        .expect("run calyx");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn file_digest(path: &Path) -> Value {
    let bytes = fs::read(path).expect("read digest file");
    json!({
        "path": display(path),
        "bytes": bytes.len(),
        "blake3": blake3::hash(&bytes).to_hex().to_string(),
    })
}

fn collect_files(root: &Path, manifest: &Path, out: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }
    for entry in fs::read_dir(root).expect("read evidence dir") {
        let path = entry.expect("dir entry").path();
        if path == manifest {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, manifest, out);
        } else {
            out.push(path);
        }
    }
}

fn collect_all(root: &Path, out: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }
    for entry in fs::read_dir(root).expect("read files") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_all(&path, out);
        } else {
            out.push(path);
        }
    }
}
