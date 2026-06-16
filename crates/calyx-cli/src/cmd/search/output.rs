use calyx_core::{CalyxError, CxId};
use calyx_sextant::Hit;
use serde::Serialize;

use crate::error::{CliError, CliResult};

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
struct GuardOut {
    verdict: &'static str,
    tau: f32,
}

#[derive(Serialize)]
struct ProvenanceOut {
    ledger_seq: u64,
    chain_hash: String,
}

#[derive(Serialize)]
struct GuardWarning {
    code: &'static str,
    message: String,
    remediation: &'static str,
    cx_id: String,
    cosine: Option<f32>,
    tau: f32,
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
            guard: guard_tau.map(|tau| GuardOut {
                verdict: "pass",
                tau,
            }),
            provenance: provenance.then(|| ProvenanceOut {
                ledger_seq: hit.provenance.seq,
                chain_hash: hex32(&hit.provenance.hash),
            }),
        })
        .collect()
}

pub(super) fn warn_guard_blocked(cx_id: CxId, cosine: Option<f32>, tau: f32) -> CliResult {
    let error = CalyxError::guard_ood(format!(
        "search guard blocked {cx_id}: cosine below in-region tau"
    ));
    let warning = GuardWarning {
        code: error.code,
        message: error.message,
        remediation: error.remediation,
        cx_id: cx_id.to_string(),
        cosine: cosine.filter(|value| value.is_finite()),
        tau,
    };
    let json = serde_json::to_string(&warning)
        .map_err(|err| CliError::usage(format!("serialize guard warning: {err}")))?;
    eprintln!("{json}");
    Ok(())
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
