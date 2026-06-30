use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::erase::{EraseRegistry, EraseScope};
use calyx_aster::vault::{AsterVault, QuotaConfig, VaultContext, VaultOptions, encode};
use calyx_core::{CalyxError, CxId, VaultStore};
use calyx_ledger::{
    ErasureScope as LedgerErasureScope, ErasureTombstone, LedgerCfStore, VerifyResult,
    find_tombstone, verify_chain,
};
use serde::Serialize;

use super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::ledger_store::AsterLedgerCfStore;
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EraseArgs {
    pub vault: String,
    pub cx_id: String,
    pub fsv_out: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct EraseReport {
    command: &'static str,
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_path: String,
    vault_id: String,
    cx_id: String,
    base_visible_before: bool,
    base_visible_after: bool,
    slot_rows_checked_before: usize,
    slot_rows_visible_before: usize,
    slot_rows_checked_after: usize,
    slot_rows_visible_after: usize,
    records_deleted: usize,
    ledger_tombstone_present: bool,
    tombstone_seq: u64,
    tombstone_records_deleted: usize,
    verify_chain_status: &'static str,
    verify_chain_checked: u64,
    verify_chain_break_at: Option<u64>,
    context_key_shredded: bool,
    fsv_out: Option<String>,
}

struct CxReadback {
    base_visible: bool,
    slot_rows_checked: usize,
    slot_rows_visible: usize,
}

struct ChainReadback {
    status: &'static str,
    checked: u64,
    break_at: Option<u64>,
}

struct ReportInput<'a> {
    status: &'static str,
    resolved: &'a ResolvedVault,
    cx_id: CxId,
    before: &'a CxReadback,
    after: &'a CxReadback,
    records_deleted: usize,
    tombstone: &'a ErasureTombstone,
    chain: ChainReadback,
    context_key_shredded: bool,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Erase(args) => run_erase(args),
        _ => unreachable!("non-erase command routed to erase module"),
    }
}

pub(crate) fn parse_erase(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("erase requires <vault> --cx-id <cx_id>"))?
        .clone();
    let mut cx_id = None;
    let mut fsv_out = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--cx-id" => {
                idx += 1;
                cx_id = Some(value(rest, idx, "--cx-id")?.to_string());
            }
            "--fsv-out" => {
                idx += 1;
                fsv_out = Some(PathBuf::from(value(rest, idx, "--fsv-out")?));
            }
            other => return Err(CliError::usage(format!("unexpected erase flag {other}"))),
        }
        idx += 1;
    }
    let cx_id = cx_id.ok_or_else(|| CliError::usage("erase requires --cx-id <cx_id>"))?;
    Ok(Subcommand::Erase(EraseArgs {
        vault,
        cx_id,
        fsv_out,
    }))
}

fn run_erase(args: EraseArgs) -> CliResult {
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let cx_id = parse_cx_id(&args.cx_id)?;
    let report = erase_report(&resolved, cx_id, args.fsv_out.as_deref())?;
    print_json(&report)
}

fn erase_report(
    resolved: &ResolvedVault,
    cx_id: CxId,
    fsv_out: Option<&Path>,
) -> CliResult<EraseReport> {
    let vault = open_vault(resolved)?;
    let before = read_cx_state(&vault, cx_id)?;
    let tombstone_before = read_tombstone(resolved, cx_id)?;
    let report = match (before.base_visible, tombstone_before) {
        (false, None) => {
            return Err(CalyxError::vault_access_denied(format!(
                "cx_id {cx_id} is absent and has no Cx erasure tombstone"
            ))
            .into());
        }
        (true, Some(tombstone)) => {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger tombstone seq {} exists while Base CF row for cx_id {cx_id} is visible",
                tombstone.seq
            ))
            .into());
        }
        (false, Some(tombstone)) => already_tombstoned_report(resolved, cx_id, before, tombstone)?,
        (true, None) => erase_visible_cx(resolved, &vault, cx_id, before)?,
    };
    write_fsv_report(fsv_out, &report)
}

fn already_tombstoned_report(
    resolved: &ResolvedVault,
    cx_id: CxId,
    before: CxReadback,
    tombstone: ErasureTombstone,
) -> CliResult<EraseReport> {
    let vault = open_vault(resolved)?;
    let after = read_cx_state(&vault, cx_id)?;
    if after.base_visible {
        return Err(CalyxError::ledger_corrupt(format!(
            "cx_id {cx_id} base row became visible during idempotent erase readback"
        ))
        .into());
    }
    let chain = verify_full_chain(&resolved.path)?;
    Ok(report(ReportInput {
        status: "already_tombstoned",
        resolved,
        cx_id,
        before: &before,
        after: &after,
        records_deleted: tombstone.records_deleted,
        tombstone: &tombstone,
        chain,
        context_key_shredded: false,
    }))
}

fn erase_visible_cx(
    resolved: &ResolvedVault,
    vault: &AsterVault,
    cx_id: CxId,
    before: CxReadback,
) -> CliResult<EraseReport> {
    let mut context = VaultContext::new(
        resolved.vault_id,
        format!("calyx-cli-erase-context-v1:{cx_id}").as_bytes(),
        QuotaConfig::default(),
        "calyx-cli",
    )?;
    let result = vault.erase(EraseScope::Cx(cx_id), &mut context, &EraseRegistry::new())?;
    vault.flush()?;
    let after = read_cx_state(vault, cx_id)?;
    if after.base_visible || after.slot_rows_visible != 0 {
        return Err(CalyxError::ledger_corrupt(format!(
            "erase postcondition failed for {cx_id}: base_visible_after={} slot_rows_visible_after={}",
            after.base_visible, after.slot_rows_visible
        ))
        .into());
    }
    let tombstone = read_tombstone(resolved, cx_id)?.ok_or_else(|| {
        CalyxError::ledger_corrupt(format!(
            "erase for {cx_id} did not write a ledger tombstone"
        ))
    })?;
    if tombstone.records_deleted != result.records_deleted {
        return Err(CalyxError::ledger_corrupt(format!(
            "erase result records_deleted {} != tombstone records_deleted {} for {cx_id}",
            result.records_deleted, tombstone.records_deleted
        ))
        .into());
    }
    let chain = verify_full_chain(&resolved.path)?;
    Ok(report(ReportInput {
        status: "erased",
        resolved,
        cx_id,
        before: &before,
        after: &after,
        records_deleted: result.records_deleted,
        tombstone: &tombstone,
        chain,
        context_key_shredded: context.is_key_shredded_for_erasure(),
    }))
}

fn report(input: ReportInput<'_>) -> EraseReport {
    EraseReport {
        command: "erase",
        status: input.status,
        source_of_truth: "Aster Base/slot CF rows read at vault snapshot plus Aster Ledger CF/WAL tombstone read through AsterLedgerCfStore and verify_chain",
        vault: input.resolved.name.clone(),
        vault_path: input.resolved.path.display().to_string(),
        vault_id: input.resolved.vault_id.to_string(),
        cx_id: input.cx_id.to_string(),
        base_visible_before: input.before.base_visible,
        base_visible_after: input.after.base_visible,
        slot_rows_checked_before: input.before.slot_rows_checked,
        slot_rows_visible_before: input.before.slot_rows_visible,
        slot_rows_checked_after: input.after.slot_rows_checked,
        slot_rows_visible_after: input.after.slot_rows_visible,
        records_deleted: input.records_deleted,
        ledger_tombstone_present: true,
        tombstone_seq: input.tombstone.seq,
        tombstone_records_deleted: input.tombstone.records_deleted,
        verify_chain_status: input.chain.status,
        verify_chain_checked: input.chain.checked,
        verify_chain_break_at: input.chain.break_at,
        context_key_shredded: input.context_key_shredded,
        fsv_out: None,
    }
}

fn read_cx_state(vault: &AsterVault, cx_id: CxId) -> CliResult<CxReadback> {
    let snapshot = vault.snapshot();
    let base = vault.read_cf_at(snapshot, ColumnFamily::Base, &base_key(cx_id))?;
    let slots = base
        .as_deref()
        .map(encode::decode_constellation_base)
        .transpose()?
        .map(|cx| cx.slots.keys().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut visible = 0;
    let mut checked = 0;
    for slot in &slots {
        for cf in [ColumnFamily::slot(*slot), ColumnFamily::slot_raw(*slot)] {
            checked += 1;
            if vault.read_cf_at(snapshot, cf, &slot_key(cx_id))?.is_some() {
                visible += 1;
            }
        }
    }
    Ok(CxReadback {
        base_visible: base.is_some(),
        slot_rows_checked: checked,
        slot_rows_visible: visible,
    })
}

fn read_tombstone(resolved: &ResolvedVault, cx_id: CxId) -> CliResult<Option<ErasureTombstone>> {
    let store = AsterLedgerCfStore::open(&resolved.path)?;
    Ok(find_tombstone(
        resolved.vault_id,
        &LedgerErasureScope::Cx(cx_id),
        &store,
    )?)
}

fn verify_full_chain(vault_path: &Path) -> CliResult<ChainReadback> {
    let store = AsterLedgerCfStore::open(vault_path)?;
    let end = store
        .scan()?
        .into_iter()
        .map(|row| row.seq)
        .max()
        .map_or(0, |seq| seq.saturating_add(1));
    match verify_chain(&store, 0..end)? {
        VerifyResult::Intact { count } => Ok(ChainReadback {
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

fn write_fsv_report(fsv_out: Option<&Path>, report: &EraseReport) -> CliResult<EraseReport> {
    let Some(path) = fsv_out else {
        return Ok(report.clone());
    };
    let mut report = report.clone();
    report.fsv_out = Some(path.display().to_string());
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&report)?;
    fs::write(path, &bytes)?;
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CalyxError::ledger_corrupt(format!(
            "FSV report readback mismatch at {}",
            path.display()
        ))
        .into());
    }
    Ok(report)
}

fn open_vault(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

fn parse_cx_id(raw: &str) -> CliResult<CxId> {
    CxId::from_str(raw).map_err(|error| CliError::usage(format!("parse --cx-id {raw}: {error}")))
}

#[cfg(test)]
mod tests;
