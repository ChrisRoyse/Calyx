use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_core::CxId;
use serde::Deserialize;

use super::engine::cx_for_doc_id;
use super::request::RecallRequest;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CorpusDoc {
    pub(crate) doc_id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ValidationData {
    pub(crate) corpus: Vec<CorpusDoc>,
    pub(crate) queries: BTreeMap<String, String>,
    pub(crate) qrels: BTreeMap<String, BTreeSet<CxId>>,
    pub(crate) qrels_rows: usize,
}

impl ValidationData {
    pub(crate) fn load(request: &RecallRequest) -> Result<Self, String> {
        let corpus = load_corpus(&request.corpus_jsonl)?;
        let queries = load_queries(&request.queries_jsonl)?;
        let (qrels, qrels_rows) = load_qrels(&request.qrels_tsv)?;
        if qrels_rows == 0 || qrels.values().all(BTreeSet::is_empty) {
            return Err("CALYX_FSV_EMPTY_QRELS".to_string());
        }
        Ok(Self {
            corpus,
            queries,
            qrels,
            qrels_rows,
        })
    }
}

#[derive(Deserialize)]
struct CorpusJsonRow {
    #[serde(rename = "_id")]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct QueryJsonRow {
    #[serde(rename = "_id")]
    id: String,
    text: String,
}

fn load_corpus(path: &Path) -> Result<Vec<CorpusDoc>, String> {
    let lines = read_lines(path)?;
    let mut docs = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: CorpusJsonRow = serde_json::from_str(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), idx + 1))?;
        let title = row.title.unwrap_or_default();
        let body = row.text.unwrap_or_default();
        let text = format!("{title} {body}").trim().to_string();
        if text.is_empty() {
            continue;
        }
        docs.push(CorpusDoc {
            doc_id: row.id,
            text,
        });
    }
    if docs.is_empty() {
        return Err(format!(
            "CALYX_FSV_SEXTANT_EMPTY_CORPUS: {}",
            path.display()
        ));
    }
    Ok(docs)
}

fn load_queries(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let lines = read_lines(path)?;
    let mut queries = BTreeMap::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: QueryJsonRow = serde_json::from_str(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), idx + 1))?;
        queries.insert(row.id, row.text);
    }
    if queries.is_empty() {
        return Err(format!(
            "CALYX_FSV_SEXTANT_EMPTY_QUERIES: {}",
            path.display()
        ));
    }
    Ok(queries)
}

fn load_qrels(path: &Path) -> Result<(BTreeMap<String, BTreeSet<CxId>>, usize), String> {
    let lines = read_lines(path)?;
    let mut qrels = BTreeMap::<String, BTreeSet<CxId>>::new();
    let mut rows = 0;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols = columns(line);
        if cols.len() < 3 {
            continue;
        }
        let Some(relevance) = parse_relevance(&cols) else {
            continue;
        };
        rows += 1;
        if relevance > 0 {
            qrels
                .entry(cols[0].to_string())
                .or_default()
                .insert(cx_for_doc_id(cols[1]));
        }
    }
    Ok((qrels, rows))
}

fn parse_relevance(cols: &[&str]) -> Option<i32> {
    cols.get(2)
        .and_then(|value| value.parse::<i32>().ok())
        .or_else(|| cols.last().and_then(|value| value.parse::<i32>().ok()))
}

fn columns(line: &str) -> Vec<&str> {
    let tabbed = line.split('\t').collect::<Vec<_>>();
    if tabbed.len() >= 3 {
        tabbed
    } else {
        line.split_whitespace().collect()
    }
}

fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    Ok(fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .lines()
        .map(str::to_string)
        .collect())
}
