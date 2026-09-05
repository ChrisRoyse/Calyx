use std::collections::BTreeMap;

use calyx_search::SearchRerankReport;
use calyx_sextant::{Hit, HitRerankEvidence};
use serde::Serialize;

use super::roster::SlotRosterOut;
use crate::error::{CliError, CliResult};

/// `--explain` response envelope: the vault-level serving roster (which slots
/// contributed via the resident, which were measured locally, and which are
/// parked/excluded from serving) plus the ranked hits (#1490).
#[derive(Serialize)]
pub(super) struct SearchExplainOut {
    pub slots: SlotRosterOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank: Option<SearchRerankReport>,
    pub hits: Vec<SearchHitOut>,
}

#[cfg(test)]
#[derive(Debug, Serialize)]
pub(super) struct KernelAnswerOut {
    pub answer: String,
    pub kernel_cx_ids: Vec<String>,
    pub recall: f32,
    pub gaps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct GroundedKernelAnswerOut {
    pub status: &'static str,
    pub traversal_mode: &'static str,
    pub answer_id: String,
    pub query_cx_id: String,
    pub kernel_id: String,
    pub kernel_manifest_sha256: String,
    pub embedding_slots: Vec<u16>,
    pub fusion: &'static str,
    pub rrf_k: u32,
    pub nearest_score: f32,
    pub nearest_lanes: Vec<calyx_lodestar::TypedPanelFusionLane>,
    pub admission_threshold: f32,
    pub anchor_kernel_node_id: String,
    pub hops: Vec<GroundedKernelAnswerHopOut>,
    pub total_score: f32,
    pub ledger_refs: Vec<GroundedKernelLedgerRefOut>,
    pub retained_query_pointer: String,
    pub source_support: calyx_lodestar::KernelAnswerSourceSupport,
    pub physical_readback: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct GroundedKernelAnswerHopOut {
    pub edge_kind: &'static str,
    pub from_cx_id: String,
    pub to_cx_id: String,
    pub edge_weight: f32,
    pub hop_index: u32,
    pub hop_score: f32,
    pub ledger_ref: GroundedKernelLedgerRefOut,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct GroundedKernelLedgerRefOut {
    pub seq: u64,
    pub hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct GroundedKernelRefusalOut {
    pub status: &'static str,
    pub code: &'static str,
    pub reason: &'static str,
    pub nearest_cx_id: String,
    pub nearest_score: f32,
    pub nearest_lanes: Vec<calyx_lodestar::TypedPanelFusionLane>,
    pub admission_threshold: f32,
    pub kernel_id: String,
    pub kernel_manifest_sha256: String,
    pub embedding_slots: Vec<u16>,
    pub fusion: &'static str,
    pub rrf_k: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct GroundedKernelScopeRefusalOut {
    pub status: &'static str,
    pub code: &'static str,
    pub reason: &'static str,
    pub detected_kind: &'static str,
    pub detected_scope: String,
    pub allowed_court_system: String,
    pub allowed_state: String,
    pub allowed_counties: Vec<String>,
    pub allowed_appellate_districts: Vec<String>,
    pub includes_statewide_scope: bool,
    pub kernel_id: String,
    pub kernel_manifest_sha256: String,
}

#[derive(Debug, Serialize)]
pub(super) struct GroundedKernelSourceRefusalOut {
    pub status: &'static str,
    pub code: &'static str,
    pub reason: &'static str,
    pub nearest_cx_id: String,
    pub nearest_score: f32,
    pub nearest_lanes: Vec<calyx_lodestar::TypedPanelFusionLane>,
    pub admission_threshold: f32,
    pub kernel_id: String,
    pub kernel_manifest_sha256: String,
    pub embedding_slots: Vec<u16>,
    pub fusion: &'static str,
    pub rrf_k: u32,
    pub source_support: calyx_lodestar::KernelAnswerSourceSupport,
}

#[derive(Serialize)]
pub(super) struct SearchHitOut {
    rank: usize,
    cx_id: String,
    score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    rerank: Option<HitRerankEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_lens: Option<Vec<PerLensOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guard: Option<GuardOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<ProvenanceOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<BTreeMap<String, String>>,
    freshness: FreshnessOut,
}

#[derive(Serialize)]
struct PerLensOut {
    slot: u16,
    rank: usize,
    #[serde(rename = "raw")]
    raw_score: f32,
    weight: f32,
    contribution: f32,
}

#[derive(Serialize)]
#[serde(untagged)]
enum GuardOut {
    /// Flat operator-supplied tau gate (#1088): the tau is the whole story.
    OperatorTau { verdict: &'static str, tau: f32 },
    /// Calibrated Ward profile gate (#1094): per-slot conformal taus with
    /// the full verdict decomposition from the hit's guard evidence.
    Profile {
        mode: &'static str,
        overall_pass: bool,
        provisional: bool,
        per_slot: Vec<SlotVerdictOut>,
    },
}

#[derive(Serialize)]
struct SlotVerdictOut {
    slot: u16,
    cos: f32,
    tau: f32,
    pass: bool,
}

#[derive(Serialize)]
struct ProvenanceOut {
    ledger_seq: u64,
    chain_hash: String,
}

#[derive(Serialize)]
struct FreshnessOut {
    built_at_seq: u64,
    base_seq: u64,
    stale_by: u64,
    policy: String,
}

pub(super) fn render_hits(
    hits: &[Hit],
    explain: bool,
    provenance: bool,
    guard_tau: Option<f32>,
    metadata: Option<&[BTreeMap<String, String>]>,
) -> CliResult<Vec<SearchHitOut>> {
    if let Some(rows) = metadata
        && rows.len() != hits.len()
    {
        return Err(CliError::runtime(format!(
            "search hit metadata cardinality {} differs from hit cardinality {}",
            rows.len(),
            hits.len()
        )));
    }
    let metadata = metadata
        .map(|rows| rows.iter().map(Some).collect::<Vec<_>>())
        .unwrap_or_else(|| std::iter::repeat_n(None, hits.len()).collect());
    Ok(hits
        .iter()
        .zip(metadata)
        .map(|(hit, metadata)| SearchHitOut {
            rank: hit.rank,
            cx_id: hit.cx_id.to_string(),
            score: hit.score,
            rerank: hit
                .explain
                .as_ref()
                .and_then(|explain| explain.rerank.clone()),
            per_lens: explain.then(|| {
                hit.per_lens
                    .iter()
                    .map(|item| PerLensOut {
                        slot: item.slot.get(),
                        rank: item.rank,
                        raw_score: item.raw_score,
                        weight: item.weight,
                        contribution: item.contribution,
                    })
                    .collect()
            }),
            guard: hit
                .guard
                .as_ref()
                .map(|evidence| GuardOut::Profile {
                    mode: "in_region_only",
                    overall_pass: evidence.verdict.overall_pass,
                    provisional: evidence.verdict.provisional,
                    per_slot: evidence
                        .verdict
                        .per_slot
                        .iter()
                        .map(|slot| SlotVerdictOut {
                            slot: slot.slot.get(),
                            cos: slot.cos,
                            tau: slot.tau,
                            pass: slot.pass,
                        })
                        .collect(),
                })
                .or(guard_tau.map(|tau| GuardOut::OperatorTau {
                    verdict: "pass",
                    tau,
                })),
            provenance: provenance.then(|| ProvenanceOut {
                ledger_seq: hit.provenance.seq,
                chain_hash: hex32(&hit.provenance.hash),
            }),
            metadata: metadata.cloned(),
            freshness: FreshnessOut {
                built_at_seq: hit.freshness.built_at_seq,
                base_seq: hit.freshness.base_seq,
                stale_by: hit.freshness.stale_by,
                policy: hit.freshness.policy.clone(),
            },
        })
        .collect())
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
