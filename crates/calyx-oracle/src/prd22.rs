//! PRD-22 Oracle formula primitives.

use calyx_core::{CalyxError, CxId, Result};
use calyx_paths::{AssocGraph, AssocGraphBuilder, reach_scored};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleCeiling {
    pub tau_corr: f32,
    pub flakiness: f32,
    pub validity: f32,
    pub oracle_self_consistency: f32,
    pub capped_tau: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OraclePrediction {
    pub panel_bits: f32,
    pub anchor_entropy_bits: f32,
    pub confidence: f32,
    pub deficit_bits: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsequenceExpansion {
    pub cx_id: CxId,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuperIntelligenceEvidence {
    pub clean: bool,
    pub sufficient: bool,
    pub kernel_recall_ratio: f32,
    pub min_kernel_recall_ratio: f32,
    pub calibrated: bool,
    pub goodhart_defended: bool,
    pub mistake_closed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuperIntelligenceVerdict {
    pub pass: bool,
    pub failing_tiers: Vec<String>,
}

pub fn oracle_ceiling(tau_corr: f32, flakiness: f32, validity: f32) -> Result<OracleCeiling> {
    validate_unit_interval(tau_corr, "tau_corr")?;
    validate_unit_interval(flakiness, "flakiness")?;
    validate_unit_interval(validity, "validity")?;
    let oracle_self_consistency = (validity * (1.0 - flakiness)).clamp(0.0, 1.0);
    Ok(OracleCeiling {
        tau_corr,
        flakiness,
        validity,
        oracle_self_consistency,
        capped_tau: tau_corr.min(oracle_self_consistency),
    })
}

pub fn oracle_predict(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    requested_confidence: f32,
) -> Result<OraclePrediction> {
    validate_non_negative(panel_bits, "panel_bits")?;
    validate_non_negative(anchor_entropy_bits, "anchor_entropy_bits")?;
    validate_unit_interval(requested_confidence, "requested_confidence")?;
    let deficit_bits = (anchor_entropy_bits - panel_bits).max(0.0);
    if deficit_bits > f32::EPSILON {
        return Err(CalyxError::oracle_insufficient(format!(
            "sufficiency deficit {deficit_bits:.6} bits; refusing confident prediction"
        )));
    }
    Ok(OraclePrediction {
        panel_bits,
        anchor_entropy_bits,
        confidence: requested_confidence,
        deficit_bits,
    })
}

pub fn butterfly_expand(
    graph: &AssocGraph,
    source: CxId,
    max_hops: usize,
) -> Result<Vec<ConsequenceExpansion>> {
    reach_scored(graph, source, max_hops)
        .map(|rows| {
            let mut expansions = rows
                .into_iter()
                .map(|(cx_id, score)| ConsequenceExpansion { cx_id, score })
                .collect::<Vec<_>>();
            expansions.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.cx_id.to_string().cmp(&right.cx_id.to_string()))
            });
            expansions
        })
        .map_err(path_error)
}

pub fn reverse_query(
    graph: &AssocGraph,
    answer: CxId,
    max_hops: usize,
) -> Result<Vec<ConsequenceExpansion>> {
    let reversed = reverse_graph(graph)?;
    butterfly_expand(&reversed, answer, max_hops)
}

pub fn super_intelligence(evidence: SuperIntelligenceEvidence) -> Result<SuperIntelligenceVerdict> {
    validate_unit_interval(evidence.kernel_recall_ratio, "kernel_recall_ratio")?;
    validate_unit_interval(evidence.min_kernel_recall_ratio, "min_kernel_recall_ratio")?;
    let mut failing_tiers = Vec::new();
    if !evidence.clean {
        failing_tiers.push("clean".to_string());
    }
    if !evidence.sufficient {
        failing_tiers.push("sufficient".to_string());
    }
    if evidence.kernel_recall_ratio < evidence.min_kernel_recall_ratio {
        failing_tiers.push("kernel".to_string());
    }
    if !evidence.calibrated {
        failing_tiers.push("calibrated".to_string());
    }
    if !evidence.goodhart_defended {
        failing_tiers.push("goodhart".to_string());
    }
    if !evidence.mistake_closed {
        failing_tiers.push("mistake_closed".to_string());
    }
    Ok(SuperIntelligenceVerdict {
        pass: failing_tiers.is_empty(),
        failing_tiers,
    })
}

fn reverse_graph(graph: &AssocGraph) -> Result<AssocGraph> {
    let mut builder = AssocGraphBuilder::default();
    for node in graph.nodes() {
        builder
            .add_node(node.id, node.frequency_weight)
            .map_err(path_error)?;
    }
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        builder
            .add_edge(dst, src, edge.weight)
            .map_err(path_error)?;
    }
    Ok(builder.build())
}

fn validate_unit_interval(value: f32, field: &str) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(CalyxError::oracle_insufficient(format!(
            "{field} must be finite and in 0.0..=1.0"
        )))
    }
}

fn validate_non_negative(value: f32, field: &str) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(CalyxError::oracle_insufficient(format!(
            "{field} must be finite non-negative"
        )))
    }
}

fn path_error(error: impl std::fmt::Display) -> CalyxError {
    CalyxError::oracle_insufficient(format!("oracle path traversal failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_formula_primitives_match_known_values() {
        let ceiling = oracle_ceiling(0.9, 0.2, 0.75).unwrap();
        assert_eq!(ceiling.oracle_self_consistency, 0.6);
        assert_eq!(ceiling.capped_tau, 0.6);

        let prediction = oracle_predict(1.0, 0.75, 0.8).unwrap();
        assert_eq!(prediction.confidence, 0.8);
        assert_eq!(
            oracle_predict(0.25, 0.75, 0.8).unwrap_err().code,
            "CALYX_ORACLE_INSUFFICIENT"
        );

        let graph = graph();
        let forward = butterfly_expand(&graph, cx(1), 2).unwrap();
        assert_eq!(forward[0].cx_id, cx(2));
        assert!((forward[0].score - 0.72).abs() < 1.0e-6);
        let reverse = reverse_query(&graph, cx(3), 2).unwrap();
        assert_eq!(reverse[0].cx_id, cx(2));

        let verdict = super_intelligence(SuperIntelligenceEvidence {
            clean: true,
            sufficient: true,
            kernel_recall_ratio: 0.96,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        })
        .unwrap();
        assert!(verdict.pass);
    }

    fn graph() -> AssocGraph {
        let mut builder = AssocGraph::builder();
        builder.add_node(cx(1), 1.0).unwrap();
        builder.add_node(cx(2), 1.0).unwrap();
        builder.add_node(cx(3), 1.0).unwrap();
        builder.add_edge(cx(1), cx(2), 0.8).unwrap();
        builder.add_edge(cx(2), cx(3), 0.5).unwrap();
        builder.build()
    }

    fn cx(seed: u8) -> CxId {
        CxId::from_bytes([seed; 16])
    }
}
