//! PH55 ASK execution: retrieval grounding.

use std::collections::BTreeSet;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Result, Seq, VaultStore};
use serde::{Deserialize, Serialize};

use crate::error::{CALYX_ANSWER_UNGROUNDED, CALYX_INVALID_ARGUMENT, sextant_error};

use super::{AskSpec, DEFAULT_ASK_TOP_K, ProvenancedRow};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AskResult {
    pub answer: String,
    pub grounding: Vec<ProvenancedRow>,
    pub gaps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_conf: Option<f32>,
}

pub fn ask<C>(vault: &AsterVault<C>, spec: &AskSpec, snapshot_seq: Seq) -> Result<AskResult>
where
    C: Clock,
{
    if spec.question.trim().is_empty() {
        return Err(sextant_error(
            CALYX_INVALID_ARGUMENT,
            "ASK question must not be empty",
        ));
    }

    let top_k = effective_top_k(spec.top_k);
    let candidates = candidate_set(vault, spec, snapshot_seq)?;
    if candidates.is_empty() {
        return Err(sextant_error(
            CALYX_ANSWER_UNGROUNDED,
            "ASK produced no visible grounding candidates",
        ));
    }

    let _ = top_k;
    Err(sextant_error(
        CALYX_ANSWER_UNGROUNDED,
        format!(
            "ASK has {} visible candidate(s), but no real query lens or lexical retriever is wired; refusing hash-scored grounding",
            candidates.len()
        ),
    ))
}

fn effective_top_k(top_k: usize) -> usize {
    if top_k == 0 { DEFAULT_ASK_TOP_K } else { top_k }
}

fn candidate_set<C>(
    vault: &AsterVault<C>,
    spec: &AskSpec,
    snapshot_seq: Seq,
) -> Result<BTreeSet<CxId>>
where
    C: Clock,
{
    if !spec.context_cx_ids.is_empty() {
        return Ok(spec
            .context_cx_ids
            .iter()
            .copied()
            .filter(|cx_id| vault.get(*cx_id, snapshot_seq).is_ok())
            .collect());
    }
    Ok(vault
        .scan_cf_at(snapshot_seq, ColumnFamily::Base)?
        .into_iter()
        .filter_map(|(key, _)| cx_id_from_base_key(&key))
        .collect::<BTreeSet<_>>())
}

fn cx_id_from_base_key(key: &[u8]) -> Option<CxId> {
    let bytes: [u8; 16] = key.try_into().ok()?;
    Some(CxId::from_bytes(bytes))
}

#[cfg(test)]
mod fsv_tests;
#[cfg(test)]
mod tests;
