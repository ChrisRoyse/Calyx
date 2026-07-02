use calyx_sextant::Hit;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct KernelAnswerOut {
    pub answer: String,
    pub kernel_cx_ids: Vec<String>,
    pub recall: f32,
    pub gaps: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct SearchHitOut {
    rank: usize,
    cx_id: String,
    score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_lens: Option<Vec<PerLensOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guard: Option<GuardOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<ProvenanceOut>,
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
) -> Vec<SearchHitOut> {
    hits.iter()
        .map(|hit| SearchHitOut {
            rank: hit.rank,
            cx_id: hit.cx_id.to_string(),
            score: hit.score,
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
            freshness: FreshnessOut {
                built_at_seq: hit.freshness.built_at_seq,
                base_seq: hit.freshness.base_seq,
                stale_by: hit.freshness.stale_by,
                policy: hit.freshness.policy.clone(),
            },
        })
        .collect()
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
