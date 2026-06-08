use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_paths::{AssocGraph, PathsError, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgreementEdge {
    pub src: CxId,
    pub dst: CxId,
    pub agreement: f32,
    pub directional_confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrequencyEntry {
    pub cx_id: CxId,
    pub frequency: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationEdge {
    pub src: CxId,
    pub dst: CxId,
}

pub fn build_assoc_graph(
    agreements: &[AgreementEdge],
    frequencies: &[FrequencyEntry],
    citations: &[CitationEdge],
) -> Result<AssocGraph> {
    let mut node_weights = BTreeMap::<CxId, f32>::new();
    for edge in agreements {
        validate_unit(edge.agreement, "agreement")?;
        validate_unit(edge.directional_confidence, "directional_confidence")?;
        node_weights.entry(edge.src).or_insert(1.0);
        node_weights.entry(edge.dst).or_insert(1.0);
    }
    for entry in frequencies {
        if entry.frequency.is_finite() && entry.frequency >= 1.0 {
            node_weights
                .entry(entry.cx_id)
                .and_modify(|weight| *weight = weight.max(entry.frequency))
                .or_insert(entry.frequency);
        } else {
            return Err(PathsError::GraphInvalidWeight {
                field: "frequency",
                value: entry.frequency,
            });
        }
    }
    for citation in citations {
        if !node_weights.contains_key(&citation.src) {
            return Err(PathsError::GraphUnknownNode { id: citation.src });
        }
        if !node_weights.contains_key(&citation.dst) {
            return Err(PathsError::GraphUnknownNode { id: citation.dst });
        }
    }

    let mut builder = AssocGraph::builder();
    for (id, weight) in node_weights {
        builder.add_node(id, weight)?;
    }
    for edge in agreements {
        builder.add_edge(
            edge.src,
            edge.dst,
            edge.agreement * edge.directional_confidence,
        )?;
    }
    for citation in citations {
        builder.add_edge(citation.src, citation.dst, 1.0)?;
    }
    Ok(builder.build())
}

fn validate_unit(value: f32, field: &'static str) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(PathsError::GraphInvalidWeight { field, value })
    }
}
