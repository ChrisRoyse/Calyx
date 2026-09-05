use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use calyx_aster::ledger_view::LedgerQuerySnapshot;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, AnchorKind, CalyxError, CxId, SlotId, SlotVector, VaultStore};
use calyx_ledger::{
    EntryKind, LedgerCfStore, LedgerEntry, REPRODUCE_PAYLOAD_TAG, SubjectId, VerifyResult,
    answer_trace_from_entries, verify_snapshot,
};
use calyx_registry::load_vault_panel_state;
use serde::Serialize;
use serde_json::Value;

use crate::server::{ToolError, ToolResult};
use crate::tools::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

use super::answer_directory::resolve_answer_vaults;
use super::answer_entries::indexed_answer_entries;
pub(super) use super::ids::hex;
use super::ids::parse_answer_id;
use super::quarantine::NoQuarantine;

#[derive(Debug, Serialize)]
pub(super) struct LineageOut {
    cx_id: String,
    ingest_seq: u64,
    ledger_chain_hash: String,
    lens_measures: Vec<LensMeasureOut>,
    anchors: Vec<AnchorOut>,
}
#[derive(Debug, Serialize)]
struct LensMeasureOut {
    slot: u16,
    lens_id: String,
    measured_at: u64,
}

#[derive(Debug, Serialize)]
struct AnchorOut {
    kind: String,
    ledger_seq: u64,
}
#[derive(Debug, Serialize)]
pub(super) struct VerifyChainOut {
    status: &'static str,
    checked: u64,
    break_at: Option<u64>,
}
#[derive(Debug, Serialize)]
pub(super) struct ReproduceOut {
    pub(super) bit_parity: bool,
    pub(super) original_hash: String,
    pub(super) reproduced_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct AnswerTraceOut {
    answer_id: String,
    complete: bool,
    trusted: bool,
    answer_seq: Option<u64>,
    kernel_seq: Option<u64>,
    guard_seq: Option<u64>,
    retrieval_steps: Vec<TraceStepOut>,
    kernel_cx_ids: Vec<String>,
    ledger_refs: Vec<LedgerRefOut>,
    fusion_weights: Option<Value>,
    guard_result: Option<Value>,
    freshness_ts: Option<u64>,
    warnings: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct TraceStepOut {
    hop: u32,
    cx_id: String,
    from_cx_id: Option<String>,
    score: f32,
    lens_id: Option<String>,
    ledger_seq: u64,
}

#[derive(Debug, Serialize)]
struct LedgerRefOut {
    role: &'static str,
    seq: u64,
    chain_hash: String,
}

pub(super) fn lineage(vault: &str, cx_id: &str) -> ToolResult<LineageOut> {
    let resolved = resolve_requested_vault(vault)?;
    let cx_id = parse_cx_id(cx_id)?;
    lineage_for_resolved(&resolved, cx_id)
}

pub(super) fn verify_chain_report(
    vault: &str,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
) -> ToolResult<VerifyChainOut> {
    let resolved = resolve_requested_vault(vault)?;
    let store = super::open_ledger_view(&resolved.path)?;
    verify_chain_for_store(&store, from_seq, to_seq)
}

pub(super) fn answer_trace(answer_id: &str) -> ToolResult<AnswerTraceOut> {
    let answer_id = parse_answer_id(answer_id)?;
    let vault_root = home_dir()?.join("vaults");
    let (paths, _) = resolve_answer_vaults(&vault_root, &answer_id)?;
    let path = match paths.as_slice() {
        [] => {
            return Err(CalyxError::vault_access_denied(format!(
                "answer_id {} not found",
                hex(&answer_id)
            ))
            .into());
        }
        [path] => path,
        paths => {
            let candidates = paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CalyxError::ledger_corrupt(format!(
                "answer_id {} is ambiguous across {} authoritative vaults: {candidates}",
                hex(&answer_id),
                paths.len()
            ))
            .into());
        }
    };
    let query = LedgerQuerySnapshot::open(path)?;
    if !query.contains_answer(&answer_id) {
        return Err(CalyxError::ledger_corrupt(format!(
            "answer directory points to vault {} without answer_id {}",
            path.display(),
            hex(&answer_id)
        ))
        .into());
    }
    let entries = indexed_answer_entries(&query, &answer_id)?;
    let trace = answer_trace_from_entries(&entries, &NoQuarantine, &answer_id)?;
    if trace.answer_entry.is_some() {
        return answer_trace_out(&answer_id, trace);
    }
    Err(CalyxError::ledger_corrupt(format!(
        "answer index in vault {} did not resolve answer_id {}",
        path.display(),
        hex(&answer_id)
    ))
    .into())
}

pub(super) fn reproduce(vault: &str, answer_id: &str) -> ToolResult<ReproduceOut> {
    let resolved = resolve_requested_vault(vault)?;
    let answer_id = parse_answer_id(answer_id)?;
    let query = LedgerQuerySnapshot::open(&resolved.path)?;
    let entries = query.entries_for_subject(&SubjectId::Query(answer_id.clone()))?;
    reproduce_report(&entries, &answer_id)
}

pub(super) fn resolve_requested_vault(vault: &str) -> ToolResult<ResolvedVault> {
    resolve_vault_info(&home_dir()?, vault)
}

pub(super) fn open_vault(resolved: &ResolvedVault) -> ToolResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

pub(super) fn lineage_for_resolved(
    resolved: &ResolvedVault,
    cx_id: CxId,
) -> ToolResult<LineageOut> {
    let vault = open_vault(resolved)?;
    let stored = vault.get(cx_id, vault.snapshot()).map_err(|error| {
        if error.code == "CALYX_STALE_DERIVED" {
            CalyxError::vault_access_denied(format!("cx_id {cx_id} does not exist in vault"))
        } else {
            error
        }
    })?;
    let query = LedgerQuerySnapshot::open(&resolved.path)?;
    let entries = query.entries_for_cx(cx_id)?;
    let projection = project_lineage_entries(
        cx_id,
        stored.provenance.seq,
        &stored.provenance.hash,
        &stored.anchors,
        &entries,
    )?;
    let state = load_vault_panel_state(&resolved.path)?;
    let lens_measures = state
        .panel
        .slots
        .iter()
        .filter_map(|slot| {
            measured_slot(&stored.slots, slot.slot_id).map(|_| LensMeasureOut {
                slot: slot.slot_id.get(),
                lens_id: slot.lens_id.to_string(),
                measured_at: stored.created_at,
            })
        })
        .collect();
    Ok(LineageOut {
        cx_id: cx_id.to_string(),
        ingest_seq: projection.ingest_seq,
        ledger_chain_hash: hex(&projection.ingest_hash),
        lens_measures,
        anchors: projection.anchors,
    })
}

pub(super) fn verify_chain_for_store(
    store: &dyn LedgerCfStore,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
) -> ToolResult<VerifyChainOut> {
    let snapshot = store.snapshot()?;
    let from = from_seq.unwrap_or(0);
    let to = to_seq.unwrap_or_else(|| {
        snapshot
            .head_anchor()
            .map_or(snapshot.len() as u64, |anchor| anchor.height)
    });
    if from > to {
        return Err(ToolError::invalid_params(format!(
            "verify_chain from_seq {from} must be <= to_seq {to}"
        )));
    }
    match verify_snapshot(&snapshot, from..to)? {
        VerifyResult::Intact { count } => Ok(VerifyChainOut {
            status: "ok",
            checked: count,
            break_at: None,
        }),
        VerifyResult::Broken { at_seq, .. } => Err(CalyxError::ledger_chain_broken(format!(
            "ledger chain broken at seq={at_seq}"
        ))
        .into()),
        VerifyResult::Corrupt { at_seq, reason } => Err(CalyxError::ledger_corrupt(format!(
            "ledger corrupt at seq={at_seq}: {reason}"
        ))
        .into()),
    }
}

pub(super) fn reproduce_report(
    entries: &[LedgerEntry],
    answer_id: &[u8],
) -> ToolResult<ReproduceOut> {
    if let Some(payload) = latest_reproduce_payload(entries, answer_id)? {
        return reproduce_from_payload(&payload);
    }
    // Fail closed: an answer subject with no recorded reproduce evidence has no
    // reproducibility to replay. This is an evidence-sufficiency failure, not an
    // access-control decision (CALYX_VAULT_ACCESS_DENIED) — never synthesize an
    // answer. Distinguish absent (zero rows) from an incomplete/aborted lifecycle.
    Err(reproduce_insufficient_error(entries, answer_id).into())
}

/// Classifies why an answer subject has no reproducible evidence and builds the
/// stable `CALYX_REPRODUCE_INSUFFICIENT` error with a message that distinguishes
/// absent / completed-but-unrecorded / incomplete-lifecycle subjects. The message
/// text must stay byte-identical to the calyx-cli reproduce_report envelope.
fn reproduce_insufficient_error(entries: &[LedgerEntry], answer_id: &[u8]) -> CalyxError {
    let subject_rows: Vec<&LedgerEntry> = entries
        .iter()
        .filter(|entry| matches!(&entry.subject, SubjectId::Query(id) if id == answer_id))
        .collect();
    let hex = hex(answer_id);
    let message = if subject_rows.is_empty() {
        format!("answer_id {hex} is absent (no ledger rows)")
    } else if subject_rows
        .iter()
        .any(|entry| is_completed_answer_row(entry))
    {
        format!(
            "answer_id {hex} has a completed answer but no recorded reproduce evidence; run reproduce --record"
        )
    } else {
        format!(
            "answer_id {hex} lifecycle is incomplete or aborted ({} rows, no completion)",
            subject_rows.len()
        )
    };
    CalyxError::reproduce_insufficient(message)
}

/// True when the entry is a completed kernel answer (not a hop/frontier row).
fn is_completed_answer_row(entry: &LedgerEntry) -> bool {
    entry.kind == EntryKind::Answer
        && serde_json::from_slice::<Value>(&entry.payload)
            .ok()
            .as_ref()
            .and_then(|payload| payload.get("type").and_then(Value::as_str))
            .is_some_and(|ty| {
                matches!(
                    ty,
                    "kernel_answer_v2"
                        | "kernel_answer_v3"
                        | "kernel_answer_v4"
                        | "kernel_citation_answer_v1"
                )
            })
}

fn answer_trace_out(
    answer_id: &[u8],
    trace: calyx_ledger::AnswerTrace,
) -> ToolResult<AnswerTraceOut> {
    let kernel_cx_ids = trace
        .path
        .iter()
        .map(|hop| hop.cx_id.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let ledger_refs = [
        ("answer", trace.answer_entry.as_ref()),
        ("kernel", trace.kernel_entry.as_ref()),
        ("guard", trace.guard_entry.as_ref()),
    ]
    .into_iter()
    .filter_map(|(role, entry)| {
        entry.map(|entry| LedgerRefOut {
            role,
            seq: entry.seq,
            chain_hash: hex(&entry.entry_hash),
        })
    })
    .collect();
    let warnings = trace
        .warnings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| CalyxError::ledger_corrupt(format!("encode warnings: {err}")))?;
    Ok(AnswerTraceOut {
        answer_id: hex(answer_id),
        complete: trace.complete,
        trusted: trace.is_trusted(),
        answer_seq: trace.answer_entry.as_ref().map(|entry| entry.seq),
        kernel_seq: trace.kernel_entry.as_ref().map(|entry| entry.seq),
        guard_seq: trace.guard_entry.as_ref().map(|entry| entry.seq),
        retrieval_steps: trace
            .path
            .into_iter()
            .map(|hop| TraceStepOut {
                hop: hop.hop,
                cx_id: hop.cx_id.to_string(),
                from_cx_id: hop.from_cx_id.map(|id| id.to_string()),
                score: hop.score,
                lens_id: hop.lens_id.map(|id| id.to_string()),
                ledger_seq: hop.ledger_seq,
            })
            .collect(),
        kernel_cx_ids,
        ledger_refs,
        fusion_weights: trace
            .fusion_weights
            .map(serde_json::to_value)
            .transpose()
            .map_err(|err| CalyxError::ledger_corrupt(format!("encode fusion weights: {err}")))?,
        guard_result: trace.guard_result,
        freshness_ts: trace.freshness_ts,
        warnings,
    })
}

struct LineageLedgerProjection {
    ingest_seq: u64,
    ingest_hash: [u8; 32],
    anchors: Vec<AnchorOut>,
}

fn project_lineage_entries(
    cx_id: CxId,
    base_seq: u64,
    base_hash: &[u8; 32],
    anchors: &[Anchor],
    entries: &[LedgerEntry],
) -> ToolResult<LineageLedgerProjection> {
    let mut base_found = false;
    let mut ingest = None::<(u64, [u8; 32])>;
    let mut anchor_seqs = BTreeMap::<String, Vec<u64>>::new();
    for entry in entries {
        base_found |= entry.seq == base_seq && entry.entry_hash == *base_hash;
        if entry.kind != EntryKind::Ingest {
            continue;
        }
        let payload = json_payload(entry)?;
        let Some(anchor_kind) = payload.get("anchor_kind").and_then(Value::as_str) else {
            if ingest.is_none_or(|current| entry.seq < current.0) {
                ingest = Some((entry.seq, entry.entry_hash));
            }
            continue;
        };
        if matches!(
            payload.get("mode").and_then(Value::as_str),
            Some("mcp-anchor" | "cli-anchor")
        ) {
            anchor_seqs
                .entry(anchor_kind.to_string())
                .or_default()
                .push(entry.seq);
        }
    }
    if !base_found {
        return Err(CalyxError::ledger_chain_broken(format!(
            "base provenance hash for {cx_id} does not match ledger seq {base_seq}"
        ))
        .into());
    }
    let (ingest_seq, ingest_hash) = ingest.ok_or_else(|| {
        CalyxError::ledger_corrupt(format!("missing ingest ledger row for {cx_id}"))
    })?;
    let mut used_per_kind = BTreeMap::<String, usize>::new();
    let mut out = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        let kind = anchor_kind_key(&anchor.kind);
        let used = used_per_kind.entry(kind.clone()).or_default();
        let seq = anchor_seqs
            .get(&kind)
            .and_then(|seqs| {
                seqs.iter()
                    .copied()
                    .filter(|seq| *seq > ingest_seq)
                    .nth(*used)
            })
            .ok_or_else(|| {
                CalyxError::ledger_corrupt(format!(
                    "anchor {kind} for {cx_id} has no exact mcp/cli anchor ledger row"
                ))
            })?;
        *used += 1;
        out.push(AnchorOut {
            ledger_seq: seq,
            kind,
        });
    }
    Ok(LineageLedgerProjection {
        ingest_seq,
        ingest_hash,
        anchors: out,
    })
}

fn latest_reproduce_payload(
    entries: &[LedgerEntry],
    answer_id: &[u8],
) -> ToolResult<Option<Value>> {
    for entry in entries.iter().rev() {
        if entry.kind != EntryKind::Admin
            || !matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
        {
            continue;
        }
        let payload = json_payload(entry)?;
        if payload.get("type").and_then(Value::as_str) == Some(REPRODUCE_PAYLOAD_TAG) {
            return Ok(Some(payload));
        }
    }
    Ok(None)
}

fn reproduce_from_payload(payload: &Value) -> ToolResult<ReproduceOut> {
    let original = payload
        .get("original_hits")
        .ok_or_else(|| CalyxError::ledger_corrupt("reproduce payload missing original_hits"))?;
    let reproduced = payload
        .get("reproduced_hits")
        .ok_or_else(|| CalyxError::ledger_corrupt("reproduce payload missing reproduced_hits"))?;
    let original_hash = hash_json(original)?;
    let reproduced_hash = hash_json(reproduced)?;
    Ok(ReproduceOut {
        bit_parity: payload.get("reproduced").and_then(Value::as_bool) == Some(true)
            && original_hash == reproduced_hash,
        original_hash,
        reproduced_hash,
    })
}

fn measured_slot(slots: &BTreeMap<SlotId, SlotVector>, slot: SlotId) -> Option<()> {
    slots
        .get(&slot)
        .filter(|vector| !vector.is_absent())
        .map(|_| ())
}

fn json_payload(entry: &LedgerEntry) -> ToolResult<Value> {
    serde_json::from_slice(&entry.payload).map_err(|err| {
        CalyxError::ledger_corrupt(format!(
            "decode ledger payload seq={} kind={:?}: {err}",
            entry.seq, entry.kind
        ))
        .into()
    })
}

fn hash_json(value: &Value) -> ToolResult<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|err| CalyxError::ledger_corrupt(format!("encode json hash: {err}")))?;
    Ok(hex(blake3::hash(&bytes).as_bytes()))
}

fn parse_cx_id(raw: &str) -> ToolResult<CxId> {
    CxId::from_str(raw)
        .map_err(|err| ToolError::invalid_params(format!("parse cx_id {raw}: {err}")))
}

fn anchor_kind_key(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::TestPass => "test_pass".to_string(),
        AnchorKind::TieFormed => "tie_formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker_match".to_string(),
        AnchorKind::StyleHold => "style_hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}
