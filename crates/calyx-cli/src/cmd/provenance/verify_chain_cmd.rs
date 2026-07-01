use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use calyx_aster::ledger_head::read_head_anchor;
use calyx_aster::ledger_view::{read_ledger_seq, read_ledger_seqs};
use calyx_core::CalyxError;
use calyx_ledger::{StreamingChainVerifier, StreamingStart, VerifyResult};
use serde::Serialize;
use serde_json::json;

use crate::bounded_progress::{Deadline, ProgressSink, parse_nonzero_u64, parse_nonzero_usize};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

use super::resolve_cli_vault;

const DEFAULT_VERIFY_BATCH_SIZE: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifyChainArgs {
    pub vault: String,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub progress_jsonl: Option<String>,
    pub time_budget_ms: Option<u64>,
    pub batch_size: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct VerifyChainOut {
    pub(crate) status: &'static str,
    pub(crate) checked: u64,
    pub(crate) break_at: Option<u64>,
}

pub(crate) fn parse_verify_chain(rest: &[String]) -> CliResult<crate::cmd::Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("verify-chain requires <vault>"))?
        .clone();
    let mut from = None;
    let mut to = None;
    let mut progress_jsonl = None;
    let mut time_budget_ms = None;
    let mut batch_size = DEFAULT_VERIFY_BATCH_SIZE;
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
            "--progress-jsonl" => {
                idx += 1;
                progress_jsonl = Some(value(rest, idx, "--progress-jsonl")?.to_string());
            }
            "--time-budget-ms" => {
                idx += 1;
                time_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--time-budget-ms")?,
                    "--time-budget-ms",
                )?);
            }
            "--batch-size" => {
                idx += 1;
                batch_size =
                    parse_nonzero_usize(value(rest, idx, "--batch-size")?, "--batch-size")?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected verify-chain flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(crate::cmd::Subcommand::VerifyChain(VerifyChainArgs {
        vault,
        from,
        to,
        progress_jsonl,
        time_budget_ms,
        batch_size,
    }))
}

pub(crate) fn run_verify_chain(args: VerifyChainArgs) -> CliResult {
    let resolved = resolve_verify_vault(&args.vault)?;
    let anchor = read_head_anchor(&resolved.path)?;
    let from = args.from.unwrap_or(0);
    let to = args
        .to
        .or_else(|| anchor.as_ref().map(|anchor| anchor.height))
        .ok_or_else(|| {
            CalyxError::ledger_corrupt(format!(
                "verify-chain {} requires --to because the vault has no ledger head anchor",
                resolved.path.display()
            ))
        })?;
    if from > to {
        return Err(CliError::usage(format!(
            "verify-chain --from {from} must be <= --to {to}"
        )));
    }
    let mut progress = ProgressSink::from_arg(args.progress_jsonl.as_deref())?;
    let deadline = Deadline::new(args.time_budget_ms);
    progress.emit(json!({
        "event": "verify_chain.progress",
        "phase": "start",
        "vault": resolved.path.display().to_string(),
        "range_start": from,
        "range_end": to,
        "checked": 0,
        "elapsed_ms": deadline.elapsed_ms(),
    }))?;
    let result = verify_vault_streaming(
        &resolved.path,
        from..to,
        anchor,
        args.batch_size,
        &deadline,
        &mut progress,
    )?;
    emit_result(from, result, &deadline, &mut progress)
}

fn verify_vault_streaming(
    vault: &Path,
    range: Range<u64>,
    anchor: Option<calyx_ledger::LedgerHeadAnchor>,
    batch_size: usize,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> CliResult<VerifyResult> {
    check_deadline(deadline, progress, "open", 0)?;
    let previous = if range.start == 0 {
        None
    } else {
        read_ledger_seq(vault, range.start - 1)?
    };
    let mut verifier =
        match StreamingChainVerifier::start(range.clone(), anchor, previous.as_ref())? {
            StreamingStart::Complete(result) => return Ok(result),
            StreamingStart::Ready(verifier) => verifier,
        };
    while verifier.next_seq() < verifier.end() {
        let start = verifier.next_seq();
        let end = verifier.end().min(start.saturating_add(batch_size as u64));
        let wanted = (start..end).collect::<BTreeSet<_>>();
        let rows = read_ledger_seqs(vault, &wanted)?;
        for seq in start..end {
            if let Some(result) = verifier.verify_next(rows.get(&seq).cloned())? {
                return Ok(result);
            }
        }
        progress.emit(json!({
            "event": "verify_chain.progress",
            "phase": "verify_batch",
            "vault": vault.display().to_string(),
            "range_start": range.start,
            "range_end": range.end,
            "batch_start": start,
            "batch_end": end,
            "checked": verifier.count(),
            "elapsed_ms": deadline.elapsed_ms(),
        }))?;
        check_deadline(deadline, progress, "verify_batch", verifier.count())?;
    }
    Ok(VerifyResult::Intact {
        count: verifier.count(),
    })
}

fn emit_result(
    from: u64,
    result: VerifyResult,
    deadline: &Deadline,
    progress: &mut ProgressSink,
) -> CliResult {
    match result {
        VerifyResult::Intact { count } => {
            progress.emit(json!({
                "event": "verify_chain.progress",
                "phase": "complete",
                "status": "ok",
                "checked": count,
                "elapsed_ms": deadline.elapsed_ms(),
            }))?;
            print_json(&VerifyChainOut {
                status: "ok",
                checked: count,
                break_at: None,
            })
        }
        VerifyResult::Broken { at_seq, .. } => {
            progress.emit(json!({
                "event": "verify_chain.progress",
                "phase": "complete",
                "status": "broken",
                "break_at": at_seq,
                "elapsed_ms": deadline.elapsed_ms(),
            }))?;
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
            progress.emit(json!({
                "event": "verify_chain.progress",
                "phase": "complete",
                "status": "corrupt",
                "break_at": at_seq,
                "elapsed_ms": deadline.elapsed_ms(),
            }))?;
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

fn check_deadline(
    deadline: &Deadline,
    progress: &mut ProgressSink,
    phase: &str,
    processed: u64,
) -> CliResult {
    match deadline.check("verify-chain", phase, processed) {
        Ok(()) => Ok(()),
        Err(error) => {
            progress.emit(json!({
                "event": "verify_chain.progress",
                "phase": "timeout",
                "checked": processed,
                "elapsed_ms": deadline.elapsed_ms(),
                "error_code": error.code(),
                "error": error.message(),
            }))?;
            Err(error)
        }
    }
}

struct ResolvedVerifyVault {
    path: PathBuf,
}

fn resolve_verify_vault(vault: &str) -> CliResult<ResolvedVerifyVault> {
    let direct = Path::new(vault);
    // A bare ref (one path component) is a vault id or CLI-index name and
    // must never be captured by an incidental same-named cwd entry (#1082).
    // Explicit filesystem paths (absolute or multi-component like ./dir)
    // keep direct verification semantics for unregistered vault dirs.
    let explicit_path = direct.is_absolute() || direct.components().count() > 1;
    if explicit_path {
        if direct.exists() {
            return Ok(ResolvedVerifyVault {
                path: direct.to_path_buf(),
            });
        }
        return Err(CalyxError::vault_access_denied(format!(
            "direct vault path {} does not exist; pass an existing vault directory, a vault id, or a CLI-index name",
            direct.display()
        ))
        .into());
    }
    let resolved = resolve_cli_vault(vault)?;
    Ok(ResolvedVerifyVault {
        path: resolved.path,
    })
}

fn parse_seq(value: &str, flag: &str) -> CliResult<u64> {
    value
        .parse::<u64>()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_bounded_batch_without_progress() {
        let parsed = parse_verify_chain(&tokens(["v", "--from", "1", "--to", "3"])).unwrap();
        assert_eq!(
            parsed,
            crate::cmd::Subcommand::VerifyChain(VerifyChainArgs {
                vault: "v".to_string(),
                from: Some(1),
                to: Some(3),
                progress_jsonl: None,
                time_budget_ms: None,
                batch_size: DEFAULT_VERIFY_BATCH_SIZE,
            })
        );
    }

    #[test]
    fn parse_progress_budget_and_batch_size() {
        let parsed = parse_verify_chain(&tokens([
            "v",
            "--progress-jsonl",
            "stderr",
            "--time-budget-ms",
            "10",
            "--batch-size",
            "2",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            crate::cmd::Subcommand::VerifyChain(VerifyChainArgs {
                vault: "v".to_string(),
                from: None,
                to: None,
                progress_jsonl: Some("stderr".to_string()),
                time_budget_ms: Some(10),
                batch_size: 2,
            })
        );
    }

    fn tokens<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(str::to_string).collect()
    }
}
