use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EvidenceNode {
    pub stable_key: String,
    pub node_type: String,
    pub label: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EvidenceEdge {
    pub src_key: String,
    pub edge_type: String,
    pub dst_key: String,
    pub weight: usize,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct EvidenceGraphDraft {
    pub nodes: BTreeMap<String, EvidenceNode>,
    pub edges: BTreeMap<String, EvidenceEdge>,
    pub node_type_counts: BTreeMap<String, usize>,
    pub edge_type_counts: BTreeMap<String, usize>,
    pub source_row_counts: BTreeMap<String, usize>,
    pub representative_paths: BTreeMap<String, Vec<String>>,
}

impl EvidenceGraphDraft {
    pub(crate) fn add_node(
        &mut self,
        stable_key: impl Into<String>,
        node_type: impl Into<String>,
        label: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) -> String {
        let stable_key = stable_key.into();
        let node_type = node_type.into();
        match self.nodes.entry(stable_key.clone()) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let node = entry.get_mut();
                for (key, value) in metadata {
                    node.metadata.entry(key).or_insert(value);
                }
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                *self.node_type_counts.entry(node_type.clone()).or_insert(0) += 1;
                entry.insert(EvidenceNode {
                    stable_key: stable_key.clone(),
                    node_type,
                    label: label.into(),
                    metadata,
                });
            }
        }
        stable_key
    }

    pub(crate) fn add_edge(
        &mut self,
        src_key: &str,
        edge_type: &str,
        dst_key: &str,
        metadata: BTreeMap<String, String>,
    ) {
        let key = edge_key(src_key, edge_type, dst_key);
        match self.edges.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let edge = entry.get_mut();
                edge.weight += 1;
                for (key, value) in metadata {
                    edge.metadata.entry(key).or_insert(value);
                }
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                *self
                    .edge_type_counts
                    .entry(edge_type.to_string())
                    .or_insert(0) += 1;
                entry.insert(EvidenceEdge {
                    src_key: src_key.to_string(),
                    edge_type: edge_type.to_string(),
                    dst_key: dst_key.to_string(),
                    weight: 1,
                    metadata,
                });
            }
        }
    }

    pub(crate) fn bump_source_row(&mut self, family: &str) {
        *self
            .source_row_counts
            .entry(family.to_string())
            .or_insert(0) += 1;
    }

    pub(crate) fn record_path(&mut self, name: &str, path: Vec<String>) {
        self.representative_paths
            .entry(name.to_string())
            .or_insert(path);
    }

    pub(crate) fn require_path(&self, name: &str) -> crate::error::CliResult {
        if self.representative_paths.contains_key(name) {
            Ok(())
        } else {
            Err(crate::error::CliError::runtime(format!(
                "missing representative evidence path {name}"
            )))
        }
    }

    pub(crate) fn association_summary(&self) -> Value {
        json!({
            "node_count": self.nodes.len(),
            "edge_count": self.edges.len(),
            "node_type_counts": self.node_type_counts,
            "edge_type_counts": self.edge_type_counts,
            "source_row_counts": self.source_row_counts,
            "representative_paths": self.representative_paths,
        })
    }
}

pub(crate) fn concept_node(
    draft: &mut EvidenceGraphDraft,
    concept_id: Option<&str>,
    label: &str,
    concept_type: &str,
) -> String {
    let stable_key = concept_id
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("concept:{}", value.trim()))
        .unwrap_or_else(|| {
            format!(
                "concept_text:{}:{}",
                normalize_key(concept_type),
                normalize_key(label)
            )
        });
    let mut metadata = BTreeMap::new();
    metadata.insert("concept_type".to_string(), concept_type.to_string());
    if let Some(concept_id) = concept_id.filter(|value| !value.trim().is_empty()) {
        metadata.insert("concept_id".to_string(), concept_id.to_string());
    }
    draft.add_node(stable_key, "concept", clean_label(label), metadata)
}

pub(crate) fn simple_node(
    draft: &mut EvidenceGraphDraft,
    node_type: &str,
    namespace: &str,
    value: &str,
    label: &str,
) -> String {
    draft.add_node(
        format!("{}:{}", namespace, normalize_key(value)),
        node_type,
        clean_label(label),
        BTreeMap::from([("source_value".to_string(), clean_label(value))]),
    )
}

pub(crate) fn edge_value(edge: &EvidenceEdge) -> Value {
    json!({
        "src_key": edge.src_key,
        "edge_type": edge.edge_type,
        "dst_key": edge.dst_key,
        "weight": edge.weight,
        "metadata": edge.metadata,
    })
}

pub(crate) fn normalize_key(value: &str) -> String {
    let mut out = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

pub(crate) fn clean_label(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 512 {
        format!("{}...", collapsed.chars().take(512).collect::<String>())
    } else {
        collapsed
    }
}

fn edge_key(src_key: &str, edge_type: &str, dst_key: &str) -> String {
    format!("{src_key}\u{1f}{edge_type}\u{1f}{dst_key}")
}
