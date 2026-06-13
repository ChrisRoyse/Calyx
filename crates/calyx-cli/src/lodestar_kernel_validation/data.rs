use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_core::{CxId, content_address};
use calyx_lodestar::RecallQuery;
use calyx_paths::AssocGraph;
use serde::Deserialize;

const TOKEN_DIM: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct CorpusSet {
    pub(crate) corpora: Vec<GraphCorpus>,
}

#[derive(Clone, Debug)]
pub(crate) struct GraphCorpus {
    pub(crate) name: String,
    pub(crate) source_path: String,
    pub(crate) source_sha256: String,
    pub(crate) rows: Vec<RecallQuery>,
    pub(crate) graph: AssocGraph,
    pub(crate) anchors: Vec<CxId>,
    pub(crate) edge_count: usize,
    pub(crate) corpus_hash: [u8; 16],
}

impl CorpusSet {
    pub(crate) fn load(dir: &Path) -> Result<Self, String> {
        if !dir.is_dir() {
            return Err(format!("CALYX_DATASET_NOT_FOUND: {}", dir.display()));
        }
        let mut paths = fs::read_dir(dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        paths.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
        paths.sort();
        if paths.is_empty() {
            return Err(format!(
                "CALYX_DATASET_NOT_FOUND: no corpus json files in {}",
                dir.display()
            ));
        }
        let corpora = paths
            .iter()
            .map(|path| GraphCorpus::load(path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { corpora })
    }
}

impl GraphCorpus {
    fn load(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let json: CorpusJson =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        json.validate(path)?;
        let mut id_map = BTreeMap::<String, CxId>::new();
        let mut rows = Vec::with_capacity(json.nodes.len());
        let mut anchors = Vec::new();
        let mut dim = None;
        for node in &json.nodes {
            let vector = node
                .features
                .clone()
                .unwrap_or_else(|| token_vector(&format!("{} {}", node.id, node.text)));
            validate_vector(&json.name, &node.id, &vector, &mut dim)?;
            let cx_id = cx_for(&json.name, &node.id, &node.text);
            id_map.insert(node.id.clone(), cx_id);
            if node.anchor {
                anchors.push(cx_id);
            }
            rows.push(RecallQuery { cx_id, vector });
        }
        let mut builder = AssocGraph::builder();
        for row in &rows {
            builder
                .add_node(row.cx_id, 1.0)
                .map_err(|error| error.to_string())?;
        }
        let mut edge_count = 0;
        for edge in &json.edges {
            let src = *id_map.get(&edge[0]).ok_or_else(|| {
                format!(
                    "CALYX_KERNEL_GRAPH_INVALID: corpus={} unknown edge source {}",
                    json.name, edge[0]
                )
            })?;
            let dst = *id_map.get(&edge[1]).ok_or_else(|| {
                format!(
                    "CALYX_KERNEL_GRAPH_INVALID: corpus={} unknown edge target {}",
                    json.name, edge[1]
                )
            })?;
            builder
                .add_edge(src, dst, 1.0)
                .map_err(|error| error.to_string())?;
            edge_count += 1;
        }
        if anchors.is_empty() {
            anchors.extend(rows.iter().take(3).map(|row| row.cx_id));
        }
        let corpus_hash = content_address(rows.iter().map(|row| row.cx_id.as_bytes().to_vec()));
        Ok(Self {
            name: json.name,
            source_path: json.source_path.unwrap_or_default(),
            source_sha256: json.source_sha256.unwrap_or_default(),
            rows,
            graph: builder.build(),
            anchors,
            edge_count,
            corpus_hash,
        })
    }
}

#[derive(Deserialize)]
struct CorpusJson {
    name: String,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default)]
    source_sha256: Option<String>,
    nodes: Vec<NodeJson>,
    edges: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct NodeJson {
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    anchor: bool,
    #[serde(default)]
    features: Option<Vec<f32>>,
}

impl CorpusJson {
    fn validate(&self, path: &Path) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self
                .name
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        {
            return Err(format!(
                "CALYX_KERNEL_GRAPH_INVALID: {} invalid corpus name {:?}",
                path.display(),
                self.name
            ));
        }
        let mut seen = BTreeSet::new();
        for node in &self.nodes {
            if node.id.trim().is_empty() {
                return Err(format!(
                    "CALYX_KERNEL_GRAPH_INVALID: {} has empty node id",
                    path.display()
                ));
            }
            if !seen.insert(node.id.as_str()) {
                return Err(format!(
                    "CALYX_KERNEL_GRAPH_INVALID: {} duplicate node {}",
                    path.display(),
                    node.id
                ));
            }
        }
        Ok(())
    }
}

fn validate_vector(
    corpus: &str,
    node: &str,
    vector: &[f32],
    expected_dim: &mut Option<usize>,
) -> Result<(), String> {
    if vector.is_empty() {
        return Err(format!(
            "CALYX_KERNEL_GRAPH_INVALID: corpus={corpus} node={node} empty vector"
        ));
    }
    if let Some(dim) = *expected_dim {
        if vector.len() != dim {
            return Err(format!(
                "CALYX_KERNEL_DIM_MISMATCH: corpus={corpus} node={node} expected {dim}, got {}",
                vector.len()
            ));
        }
    } else {
        *expected_dim = Some(vector.len());
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "CALYX_KERNEL_GRAPH_INVALID: corpus={corpus} node={node} non-finite vector"
        ));
    }
    Ok(())
}

fn cx_for(corpus: &str, id: &str, text: &str) -> CxId {
    CxId::from_bytes(content_address([
        corpus.as_bytes().to_vec(),
        id.as_bytes().to_vec(),
        text.as_bytes().to_vec(),
    ]))
}

fn token_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; TOKEN_DIM];
    for token in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.len() < 2 {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        let digest = blake3::hash(lower.as_bytes());
        let idx =
            u16::from_be_bytes([digest.as_bytes()[0], digest.as_bytes()[1]]) as usize % TOKEN_DIM;
        vector[idx] += 1.0;
    }
    normalize(&mut vector);
    vector
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    } else if let Some(first) = vector.first_mut() {
        *first = 1.0;
    }
}
