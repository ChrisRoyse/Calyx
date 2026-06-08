use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, content_address};
use calyx_lodestar::RecallQuery;
use calyx_paths::AssocGraph;

use super::{CorpusCase, corpus_case, cx_for, read_lines, similarity_graph, token_vector};

const TEXT_LIMIT: usize = 180;
const CODE_LIMIT: usize = 180;
const SCIFACT_CORPUS_HASH: &str = "28f4c3e5cdc276b03d4605ea63d3ac19";
const SCIFACT_QRELS_HASH: &str = "193519c60f28c755ee2252d544f5885e";
const CORA_CONTENT_HASH: &str = "b6edd21ddd66164b96ad307884b900cc";
const CORA_CITES_HASH: &str = "ecb54faa6654b1c52773be5acfc2ce71";

pub fn scifact_text(home: &Path) -> CorpusCase {
    let root = home.join("data/datasets/beir-scifact/scifact");
    let corpus_path = root.join("corpus.jsonl");
    let qrels_path = root.join("qrels/test.tsv");
    assert_file_hash(&corpus_path, SCIFACT_CORPUS_HASH);
    assert_file_hash(&qrels_path, SCIFACT_QRELS_HASH);
    let qrel_ids = read_scifact_qrels(&qrels_path);
    let mut rows = Vec::new();
    let mut anchors = Vec::new();
    for line in read_lines(&corpus_path).into_iter().take(TEXT_LIMIT) {
        let value: serde_json::Value = serde_json::from_str(&line).expect("scifact json");
        let id = value["_id"].as_str().expect("_id");
        let title = value["title"].as_str().unwrap_or_default();
        let text = value["text"].as_str().unwrap_or_default();
        let body = format!("{title} {text}");
        let cx_id = cx_for("scifact", id, &body);
        if qrel_ids.contains(id) {
            anchors.push(cx_id);
        }
        rows.push(RecallQuery {
            cx_id,
            vector: token_vector(&body),
        });
    }
    if anchors.len() < 3 {
        anchors.extend(rows.iter().take(3).map(|row| row.cx_id));
    }
    let graph = similarity_graph(&rows, 4);
    corpus_case(
        "scifact_text",
        "text",
        vec![corpus_path, qrels_path],
        rows,
        graph,
        anchors,
    )
}

pub fn calyx_code(home: &Path) -> CorpusCase {
    let root = home.join("repo/crates");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    files.sort();
    let mut rows = Vec::new();
    let mut anchors = Vec::new();
    for path in files.into_iter().take(CODE_LIMIT) {
        let body = fs::read_to_string(&path).expect("read code");
        let rel = path.strip_prefix(home.join("repo")).unwrap_or(&path);
        let cx_id = cx_for("calyx-code", &rel.display().to_string(), &body);
        if body.contains("#[test]") || body.contains("proptest!") {
            anchors.push(cx_id);
        }
        rows.push(RecallQuery {
            cx_id,
            vector: token_vector(&body),
        });
    }
    if anchors.len() < 3 {
        anchors.extend(rows.iter().take(3).map(|row| row.cx_id));
    }
    let graph = similarity_graph(&rows, 4);
    corpus_case("calyx_code", "code", vec![root], rows, graph, anchors)
}

pub fn cora_graph(home: &Path) -> CorpusCase {
    let root = home.join("data/datasets/cora/cora");
    let content = root.join("cora.content");
    let cites = root.join("cora.cites");
    assert_file_hash(&content, CORA_CONTENT_HASH);
    assert_file_hash(&cites, CORA_CITES_HASH);
    let mut rows = Vec::new();
    let mut labels = BTreeMap::<String, Vec<CxId>>::new();
    let mut id_map = BTreeMap::<String, CxId>::new();
    for line in read_lines(&content) {
        let parts: Vec<_> = line.split_whitespace().collect();
        let paper = parts[0];
        let label = parts.last().expect("label").to_string();
        let vector = parts[1..parts.len() - 1]
            .iter()
            .map(|bit| bit.parse::<f32>().expect("feature bit"))
            .collect::<Vec<_>>();
        let cx_id = cx_for("cora", paper, &line);
        id_map.insert(paper.to_string(), cx_id);
        labels.entry(label).or_default().push(cx_id);
        rows.push(RecallQuery { cx_id, vector });
    }
    let mut builder = AssocGraph::builder();
    for row in &rows {
        builder.add_node(row.cx_id, 1.0).expect("node");
    }
    for line in read_lines(&cites) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if let (Some(src), Some(dst)) = (id_map.get(parts[0]), id_map.get(parts[1])) {
            builder.add_edge(*src, *dst, 1.0).expect("cite edge");
            builder.add_edge(*dst, *src, 1.0).expect("assoc edge");
        }
    }
    for ids in labels.values() {
        for pair in ids.windows(2) {
            builder.add_edge(pair[0], pair[1], 0.8).expect("label edge");
            builder.add_edge(pair[1], pair[0], 0.8).expect("label edge");
        }
    }
    let anchors = labels
        .values()
        .filter_map(|ids| ids.first().copied())
        .collect();
    corpus_case(
        "cora_graph",
        "graph",
        vec![content, cites],
        rows,
        builder.build(),
        anchors,
    )
}

fn read_scifact_qrels(path: &Path) -> BTreeSet<String> {
    read_lines(path)
        .into_iter()
        .skip(1)
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

fn collect_rs(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn assert_file_hash(path: &Path, expected: &str) {
    let body = fs::read(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let actual = hex(&content_address([body]));
    assert_eq!(
        actual,
        expected,
        "CALYX_DATASET_CHECKSUM_MISMATCH: {}",
        path.display()
    );
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
