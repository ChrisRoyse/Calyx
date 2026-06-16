use calyx_sextant::Hit;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct KernelAnswerOut {
    pub(super) answer: String,
    pub(super) kernel_cx_ids: Vec<String>,
    pub(super) recall: f32,
    pub(super) gaps: Vec<String>,
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
    provenance: ProvenanceOut,
}

#[derive(Serialize)]
struct PerLensOut {
    slot: u16,
    rank: usize,
    raw: f32,
    weight: f32,
    contribution: f32,
}

#[derive(Serialize)]
struct GuardOut {
    verdict: &'static str,
    tau: f32,
}

#[derive(Serialize)]
struct ProvenanceOut {
    ledger_seq: u64,
    chain_hash: String,
}

pub(super) fn render_hits(
    hits: &[Hit],
    explain: bool,
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
                        raw: item.raw_score,
                        weight: item.weight,
                        contribution: item.contribution,
                    })
                    .collect()
            }),
            guard: guard_tau.map(|tau| GuardOut {
                verdict: "pass",
                tau,
            }),
            provenance: ProvenanceOut {
                ledger_seq: hit.provenance.seq,
                chain_hash: hex32(&hit.provenance.hash),
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
