use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, AnchorKind, CalyxError, CxId, SlotId, SlotVector, VaultStore};
use calyx_ledger::{
    EntryKind, LedgerCfStore, LedgerEntry, REPRODUCE_PAYLOAD_TAG, SubjectId, VerifyResult, decode,
    get_answer_trace, get_provenance, verify_chain,
};
use calyx_registry::load_vault_panel_state;
use serde::Serialize;
use serde_json::{Value, json};

use crate::server::{ToolError, ToolResult};
use crate::tools::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

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
    for path in vault_paths()? {
        let Ok(store) = AsterLedgerCfStore::open(&path) else {
            continue;
        };
        let trace = get_answer_trace(&store, &NoQuarantine, &answer_id)?;
        if trace.answer_entry.is_some() {
            return answer_trace_out(&answer_id, trace);
        }
    }
    Err(CalyxError::vault_access_denied(format!("answer_id {} not found", hex(&answer_id))).into())
}

pub(super) fn reproduce(vault: &str, answer_id: &str) -> ToolResult<ReproduceOut> {
    let resolved = resolve_requested_vault(vault)?;
    let entries = ledger_entries(&resolved.path)?;
    reproduce_report(&entries, &parse_answer_id(answer_id)?)
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
    let store = AsterLedgerCfStore::open(&resolved.path)?;
    let entries = get_provenance(&store, &NoQuarantine, cx_id)?;
    verify_current_base_ref(
        cx_id,
        stored.provenance.seq,
        &stored.provenance.hash,
        &entries,
    )?;
    let ingest = ingest_entry(cx_id, &entries)?;
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
    let anchors = anchor_outputs(cx_id, ingest.seq, &stored.anchors, &entries)?;
    Ok(LineageOut {
        cx_id: cx_id.to_string(),
        ingest_seq: ingest.seq,
        ledger_chain_hash: hex(&ingest.entry_hash),
        lens_measures,
        anchors,
    })
}

pub(super) fn verify_chain_for_store(
    store: &dyn LedgerCfStore,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
) -> ToolResult<VerifyChainOut> {
    let from = from_seq.unwrap_or(0);
    let to = to_seq.unwrap_or_else(|| chain_end(store).unwrap_or(from));
    if from > to {
        return Err(ToolError::invalid_params(format!(
            "verify_chain from_seq {from} must be <= to_seq {to}"
        )));
    }
    match verify_chain(store, from..to)? {
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
    if entries.iter().any(|entry| {
        entry.kind == EntryKind::Answer
            && matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
    }) {
        return Err(CalyxError::reproduce_nondeterministic(format!(
            "answer_id {} has no reproduce_v1 ledger row",
            hex(answer_id)
        ))
        .into());
    }
    Err(CalyxError::vault_access_denied(format!("answer_id {} not found", hex(answer_id))).into())
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

fn verify_current_base_ref(
    cx_id: CxId,
    seq: u64,
    hash: &[u8; 32],
    entries: &[LedgerEntry],
) -> ToolResult<()> {
    if entries
        .iter()
        .any(|entry| entry.seq == seq && entry.entry_hash == *hash)
    {
        return Ok(());
    }
    Err(CalyxError::ledger_chain_broken(format!(
        "base provenance hash for {cx_id} does not match ledger seq {seq}"
    ))
    .into())
}

fn ingest_entry(cx_id: CxId, entries: &[LedgerEntry]) -> ToolResult<&LedgerEntry> {
    entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::Ingest
                && matches!(entry.subject, SubjectId::Cx(id) if id == cx_id)
                && json_payload(entry)
                    .get("anchor_kind")
                    .and_then(Value::as_str)
                    .is_none()
        })
        .min_by_key(|entry| entry.seq)
        .ok_or_else(|| {
            CalyxError::ledger_corrupt(format!("missing ingest ledger row for {cx_id}")).into()
        })
}

fn anchor_outputs(
    cx_id: CxId,
    ingest_seq: u64,
    anchors: &[Anchor],
    entries: &[LedgerEntry],
) -> ToolResult<Vec<AnchorOut>> {
    let mut used = BTreeSet::new();
    let mut out = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        let kind = anchor_kind_key(&anchor.kind);
        out.push(AnchorOut {
            ledger_seq: match_anchor_entry(cx_id, ingest_seq, &kind, entries, &mut used)?,
            kind,
        });
    }
    Ok(out)
}

fn match_anchor_entry(
    cx_id: CxId,
    ingest_seq: u64,
    kind: &str,
    entries: &[LedgerEntry],
    used: &mut BTreeSet<u64>,
) -> ToolResult<u64> {
    let mut fallback = None;
    for entry in entries {
        if used.contains(&entry.seq) || entry.seq <= ingest_seq {
            continue;
        }
        if entry.kind != EntryKind::Ingest
            || !matches!(entry.subject, SubjectId::Cx(id) if id == cx_id)
        {
            continue;
        }
        let payload = json_payload(entry);
        let mode = payload.get("mode").and_then(Value::as_str);
        let anchor_kind = payload.get("anchor_kind").and_then(Value::as_str);
        if matches!(mode, Some("mcp-anchor" | "cli-anchor")) && anchor_kind == Some(kind) {
            used.insert(entry.seq);
            return Ok(entry.seq);
        }
        if fallback.is_none()
            && (matches!(mode, Some("mcp-anchor" | "cli-anchor"))
                || (mode.is_none() && anchor_kind.is_none()))
        {
            fallback = Some(entry.seq);
        }
    }
    if let Some(seq) = fallback {
        used.insert(seq);
        Ok(seq)
    } else {
        Err(CalyxError::ledger_corrupt(format!(
            "anchor {kind} for {cx_id} has no matching ledger row"
        ))
        .into())
    }
}

fn latest_reproduce_payload(
    entries: &[LedgerEntry],
    answer_id: &[u8],
) -> ToolResult<Option<Value>> {
    for entry in entries.iter().rev() {
        if !matches!(&entry.subject, SubjectId::Query(id) if id == answer_id) {
            continue;
        }
        let payload = json_payload(entry);
        if entry.kind == EntryKind::Admin
            && payload.get("type").and_then(Value::as_str) == Some(REPRODUCE_PAYLOAD_TAG)
        {
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

pub(super) fn ledger_entries(path: &Path) -> ToolResult<Vec<LedgerEntry>> {
    let store = AsterLedgerCfStore::open(path)?;
    let mut entries = Vec::new();
    for row in store.scan()? {
        entries.push(decode(&row.bytes)?);
    }
    entries.sort_by_key(|entry| entry.seq);
    Ok(entries)
}

fn chain_end(store: &dyn LedgerCfStore) -> ToolResult<u64> {
    Ok(store
        .scan()?
        .into_iter()
        .map(|row| row.seq)
        .max()
        .map_or(0, |seq| seq.saturating_add(1)))
}

fn vault_paths() -> ToolResult<Vec<PathBuf>> {
    let root = home_dir()?.join("vaults");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|err| CalyxError::disk_pressure(format!("read vaults dir: {err}")))?
    {
        let entry = entry.map_err(|err| CalyxError::disk_pressure(format!("read vault: {err}")))?;
        if entry
            .file_type()
            .map_err(|err| CalyxError::disk_pressure(format!("read vault file type: {err}")))?
            .is_dir()
        {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

fn measured_slot(slots: &BTreeMap<SlotId, SlotVector>, slot: SlotId) -> Option<()> {
    slots
        .get(&slot)
        .filter(|vector| !vector.is_absent())
        .map(|_| ())
}

fn json_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).unwrap_or_else(|_| json!({}))
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
