use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, AnchorKind, CalyxError, CxId, SlotId, SlotVector, VaultStore};
use calyx_ledger::{
    EntryKind, LedgerCfStore, LedgerEntry, QuarantineLookup, REPRODUCE_PAYLOAD_TAG, SubjectId,
    VerifyResult, decode, get_provenance, verify_chain,
};
use calyx_registry::load_vault_panel_state;
use serde::Serialize;
use serde_json::{Value, json};

use super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};
use crate::ledger_store::AsterLedgerCfStore;
use crate::output::print_json;

mod lineage_support;
mod status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProvenanceArgs {
    pub vault: String,
    pub cx_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifyChainArgs {
    pub vault: String,
    pub from: Option<u64>,
    pub to: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReproduceArgs {
    pub vault: String,
    pub answer_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnnealStatusArgs {
    pub vault: String,
}

#[derive(Debug, Serialize)]
struct LineageOut {
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
struct VerifyChainOut {
    status: &'static str,
    checked: u64,
    break_at: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ReproduceOut {
    bit_parity: bool,
    original_hash: String,
    reproduced_hash: String,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Provenance(args) => run_provenance(args),
        Subcommand::VerifyChain(args) => run_verify_chain(args),
        Subcommand::Reproduce(args) => run_reproduce(args),
        Subcommand::AnnealStatus(args) => run_anneal_status(args),
        _ => unreachable!("non-provenance command routed to provenance module"),
    }
}

pub(crate) fn parse_provenance(rest: &[String]) -> CliResult<Subcommand> {
    match rest {
        [vault, cx_id] => Ok(Subcommand::Provenance(ProvenanceArgs {
            vault: vault.clone(),
            cx_id: cx_id.clone(),
        })),
        _ => Err(CliError::usage("provenance requires <vault> <cx_id>")),
    }
}

pub(crate) fn parse_verify_chain(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("verify-chain requires <vault>"))?
        .clone();
    let mut from = None;
    let mut to = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--from" => {
                idx += 1;
                from = Some(parse_seq(value(rest, idx, "--from")?, "--from")?);
            }
            "--to" => {
                idx += 1;
                to = Some(parse_seq(value(rest, idx, "--to")?, "--to")?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected verify-chain flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::VerifyChain(VerifyChainArgs { vault, from, to }))
}

pub(crate) fn parse_reproduce(rest: &[String]) -> CliResult<Subcommand> {
    match rest {
        [vault, answer_id] => Ok(Subcommand::Reproduce(ReproduceArgs {
            vault: vault.clone(),
            answer_id: answer_id.clone(),
        })),
        _ => Err(CliError::usage("reproduce requires <vault> <answer_id>")),
    }
}

pub(crate) fn parse_anneal_status(rest: &[String]) -> CliResult<Subcommand> {
    match rest {
        [vault] => Ok(Subcommand::AnnealStatus(AnnealStatusArgs {
            vault: vault.clone(),
        })),
        _ => Err(CliError::usage("anneal-status requires <vault>")),
    }
}

fn run_provenance(args: ProvenanceArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let cx_id = CxId::from_str(&args.cx_id)
        .map_err(|err| CliError::usage(format!("parse <cx_id> {}: {err}", args.cx_id)))?;
    print_json(&lineage(&resolved, cx_id)?)
}

fn run_verify_chain(args: VerifyChainArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let store = AsterLedgerCfStore::open(&resolved.path)?;
    let from = args.from.unwrap_or(0);
    let to = args.to.unwrap_or_else(|| chain_end(&store).unwrap_or(from));
    if from > to {
        return Err(CliError::usage(format!(
            "verify-chain --from {from} must be <= --to {to}"
        )));
    }
    match verify_chain(&store, from..to)? {
        VerifyResult::Intact { count } => print_json(&VerifyChainOut {
            status: "ok",
            checked: count,
            break_at: None,
        }),
        VerifyResult::Broken { at_seq, .. } => {
            print_json(&VerifyChainOut {
                status: "broken",
                checked: at_seq.saturating_sub(from),
                break_at: Some(at_seq),
            })?;
            Err(
                CalyxError::ledger_chain_broken(format!("ledger chain broken at seq={at_seq}"))
                    .into(),
            )
        }
        VerifyResult::Corrupt { at_seq, reason } => {
            print_json(&VerifyChainOut {
                status: "broken",
                checked: at_seq.saturating_sub(from),
                break_at: Some(at_seq),
            })?;
            Err(
                CalyxError::ledger_corrupt(format!("ledger corrupt at seq={at_seq}: {reason}"))
                    .into(),
            )
        }
    }
}

fn run_reproduce(args: ReproduceArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let entries = ledger_entries(&resolved.path)?;
    let answer_id = parse_answer_id(&args.answer_id)?;
    let report = reproduce_report(&entries, &answer_id)?;
    print_json(&report)?;
    if report.bit_parity {
        Ok(())
    } else {
        Err(CalyxError::reproduce_drift_exceeded(format!(
            "original_hash={} reproduced_hash={}",
            report.original_hash, report.reproduced_hash
        ))
        .into())
    }
}

fn run_anneal_status(args: AnnealStatusArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    status::run(&resolved)
}

fn lineage(resolved: &ResolvedVault, cx_id: CxId) -> CliResult<LineageOut> {
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
    let current = entries
        .iter()
        .find(|entry| entry.seq == stored.provenance.seq)
        .ok_or_else(|| {
            CalyxError::ledger_corrupt(format!(
                "missing provenance ledger seq {}",
                stored.provenance.seq
            ))
        })?;
    if current.entry_hash != stored.provenance.hash {
        return Err(CalyxError::ledger_chain_broken(format!(
            "base provenance hash for {cx_id} does not match ledger seq {}",
            stored.provenance.seq
        ))
        .into());
    }
    let ingest = lineage_support::primary_ingest_entry(cx_id, current, &entries)?;
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
        ledger_chain_hash: hex_bytes(&current.entry_hash),
        lens_measures,
        anchors,
    })
}

fn anchor_outputs(
    cx_id: CxId,
    ingest_seq: u64,
    anchors: &[Anchor],
    entries: &[LedgerEntry],
) -> CliResult<Vec<AnchorOut>> {
    let mut used = BTreeSet::new();
    let mut out = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        let kind = anchor_kind_key(&anchor.kind);
        let seq = match_anchor_entry(cx_id, ingest_seq, &kind, entries, &mut used)?;
        out.push(AnchorOut {
            kind,
            ledger_seq: seq,
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
) -> CliResult<u64> {
    let mut fallback = None;
    for entry in entries {
        if used.contains(&entry.seq) || entry.seq == ingest_seq || entry.seq <= ingest_seq {
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
        if mode == Some("cli-anchor") && anchor_kind == Some(kind) {
            used.insert(entry.seq);
            return Ok(entry.seq);
        }
        if fallback.is_none()
            && (mode == Some("cli-anchor") || (mode.is_none() && anchor_kind.is_none()))
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

fn reproduce_report(entries: &[LedgerEntry], answer_id: &[u8]) -> CliResult<ReproduceOut> {
    if let Some(payload) = latest_reproduce_payload(entries, answer_id)? {
        return reproduce_from_payload(&payload);
    }
    if entries.iter().any(|entry| {
        entry.kind == EntryKind::Answer
            && matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
    }) {
        return Err(CalyxError::reproduce_nondeterministic(format!(
            "answer_id {} has no reproduce_v1 ledger row",
            hex_bytes(answer_id)
        ))
        .into());
    }
    Err(
        CalyxError::vault_access_denied(format!("answer_id {} not found", hex_bytes(answer_id)))
            .into(),
    )
}

fn latest_reproduce_payload(entries: &[LedgerEntry], answer_id: &[u8]) -> CliResult<Option<Value>> {
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

fn reproduce_from_payload(payload: &Value) -> CliResult<ReproduceOut> {
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

fn ledger_entries(path: &Path) -> CliResult<Vec<LedgerEntry>> {
    let store = AsterLedgerCfStore::open(path)?;
    let mut entries = Vec::new();
    for row in store.scan()? {
        entries.push(decode(&row.bytes)?);
    }
    entries.sort_by_key(|entry| entry.seq);
    Ok(entries)
}

fn chain_end(store: &AsterLedgerCfStore) -> CliResult<u64> {
    Ok(store
        .scan()?
        .into_iter()
        .map(|row| row.seq)
        .max()
        .map_or(0, |seq| seq.saturating_add(1)))
}

fn open_vault(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

fn resolve_cli_vault(vault: &str) -> CliResult<ResolvedVault> {
    resolve_vault_info(&home_dir()?, vault)
}

fn measured_slot(
    slots: &std::collections::BTreeMap<SlotId, SlotVector>,
    slot: SlotId,
) -> Option<()> {
    slots
        .get(&slot)
        .filter(|vector| !vector.is_absent())
        .map(|_| ())
}

fn json_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).unwrap_or_else(|_| json!({}))
}

fn hash_json(value: &Value) -> CliResult<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(hex_bytes(blake3::hash(&bytes).as_bytes()))
}

fn parse_seq(raw: &str, flag: &str) -> CliResult<u64> {
    raw.parse::<u64>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))
}

fn parse_answer_id(raw: &str) -> CliResult<Vec<u8>> {
    if raw.is_empty() {
        return Err(CliError::usage("answer_id must not be empty"));
    }
    if raw.len().is_multiple_of(2)
        && raw.bytes().all(|byte| byte.is_ascii_hexdigit())
        && let Some(bytes) = decode_hex(raw)
    {
        return Ok(bytes);
    }
    Ok(raw.as_bytes().to_vec())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Some((hex_value(chunk[0])? << 4) | hex_value(chunk[1])?))
        .collect()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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

struct NoQuarantine;

impl QuarantineLookup for NoQuarantine {
    fn contains_quarantined(&self, _range: Range<u64>) -> calyx_core::Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests;
