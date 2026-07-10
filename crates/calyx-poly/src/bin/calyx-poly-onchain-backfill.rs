use std::any::Any;
use std::env;
use std::panic;
use std::path::PathBuf;
use std::process;

use calyx_core::SystemClock;
use calyx_poly::onchain_backfill_lease::{
    DEFAULT_HEARTBEAT_INTERVAL_SECS, DEFAULT_MAX_RUNTIME_SECS, acquire, lease_path_for_output_root,
};
use calyx_poly::{
    OnchainBackfillReadbackScope, OnchainBackfillRunRequest, PolyError, PolyLogEvent,
    PolyResultLogExt, StructuredLogSink, log_context, readback_onchain_backfill_run_scoped,
    require_onchain_backfill_readback_passed, run_onchain_backfill_once_with_readback_scope,
};
use serde_json::json;

struct Cli {
    request: OnchainBackfillRunRequest,
    log_path: PathBuf,
    readback_only: bool,
    readback_scope: OnchainBackfillReadbackScope,
    max_runtime_secs: u64,
    heartbeat_secs: u64,
    lease_path: Option<PathBuf>,
    takeover: bool,
}

enum CliAction {
    Help,
    Run(Cli),
}

fn main() {
    match panic::catch_unwind(run) {
        Ok(Ok(code)) => process::exit(code),
        Ok(Err(error)) => exit_with_error(error, 2),
        Err(payload) => exit_with_error(
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_PANIC",
                format!(
                    "on-chain backfill panicked: {}",
                    panic_message(payload.as_ref())
                ),
            ),
            101,
        ),
    }
}

fn exit_with_error(error: PolyError, code: i32) -> ! {
    let payload = json!({
        "ok": false,
        "error": error.diagnostic()
    });
    eprintln!(
        "{}",
        serde_json::to_string(&payload).unwrap_or_else(|_| {
            "{\"ok\":false,\"code\":\"POLY_ONCHAIN_BACKFILL_ERROR_ENCODE_FAILED\"}".to_string()
        })
    );
    process::exit(code);
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "non-string panic payload".to_string()
}

fn run() -> calyx_poly::Result<i32> {
    let CliAction::Run(mut cli) = parse_cli(env::args().skip(1).collect())? else {
        println!("{}", usage());
        return Ok(0);
    };
    cli.request = cli.request.normalized()?;
    let sink = StructuredLogSink::new(cli.log_path.clone())?;
    let clock = SystemClock;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "onchain_backfill",
        "run",
        "POLY_ONCHAIN_BACKFILL_RUN_STARTED",
        "capturing resumable public Polygon OrderFilled chunks",
        log_context(&[
            ("state_path", cli.request.state_path.display().to_string()),
            ("output_root", cli.request.output_root.display().to_string()),
            ("log_path", sink.path().display().to_string()),
            (
                "max_chunks_per_contract",
                cli.request.max_chunks_per_contract.to_string(),
            ),
            (
                "max_blocks_per_chunk",
                cli.request.max_blocks_per_chunk.to_string(),
            ),
            ("readback_only", cli.readback_only.to_string()),
            ("readback_scope", cli.readback_scope.as_str().to_string()),
        ]),
    )?)?;
    // Acquire the cross-session lease before doing any work: this reaps a stale
    // prior runner (so a leftover orphan cannot silently pin the binary), refuses
    // to collide with a live peer, and arms the max-runtime watchdog. The guard
    // releases the lease when this function returns (or the watchdog force-exits).
    let lease_path = cli
        .lease_path
        .clone()
        .unwrap_or_else(|| lease_path_for_output_root(&cli.request.output_root));
    let _lease = acquire(
        &lease_path,
        onchain_backfill_command_line(),
        cli.max_runtime_secs,
        cli.heartbeat_secs,
        cli.takeover,
        &sink,
        clock,
    )?;
    let report = if !cli.readback_only {
        run_onchain_backfill_once_with_readback_scope(
            cli.request.clone(),
            cli.readback_scope,
            &clock,
        )
        .log_error_context(
            &clock,
            &sink,
            "onchain_backfill",
            "capture",
            log_context(&[
                ("state_path", cli.request.state_path.display().to_string()),
                ("output_root", cli.request.output_root.display().to_string()),
                (
                    "max_blocks_per_chunk",
                    cli.request.max_blocks_per_chunk.to_string(),
                ),
            ]),
        )?
        .readback
    } else {
        readback_onchain_backfill_run_scoped(
            &cli.request.output_root,
            cli.request.max_blocks_per_chunk,
            cli.readback_scope,
        )
        .log_error_context(
            &clock,
            &sink,
            "onchain_backfill",
            "readback",
            log_context(&[("output_root", cli.request.output_root.display().to_string())]),
        )?
    };
    let result = require_onchain_backfill_readback_passed(&report);
    if let Err(error) = &result {
        sink.append_error(
            &clock,
            "onchain_backfill",
            "require_passed",
            error,
            log_context(&[
                ("output_root", cli.request.output_root.display().to_string()),
                ("status_code", report.status_code.clone()),
            ]),
        )?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": report.passed,
            "status_code": report.status_code,
            "output_root": report.output_root,
            "run_report_path": report.run_report_path,
            "checkpoint_path": report.checkpoint_path,
            "readback_report_path": report.readback_report_path,
            "readback_scope": report.readback_scope,
            "readback_progress_path": report.readback_progress_path,
            "checked_file_count": report.checked_file_count,
            "unique_file_read_count": report.unique_file_read_count,
            "deduplicated_file_read_count": report.deduplicated_file_read_count,
            "json_parse_count": report.json_parse_count,
            "readback_bytes_read": report.readback_bytes_read,
            "readback_body_bytes_read": report.readback_body_bytes_read,
            "readback_request_bytes_read": report.readback_request_bytes_read,
            "readback_metadata_bytes_read": report.readback_metadata_bytes_read,
            "progress_event_count": report.progress_event_count,
            "missing_files": report.missing_files,
            "sha_mismatches": report.sha_mismatches,
            "parse_failures": report.parse_failures,
            "total_pages": report.total_pages,
            "current_run_page_count": report.current_run_page_count,
            "checkpoint_range_count": report.checkpoint_range_count,
            "total_records": report.total_records,
            "total_body_bytes": report.total_body_bytes,
            "max_blocks_per_chunk": cli.request.max_blocks_per_chunk,
            "all_order_filled_backfill_complete": report.all_order_filled_backfill_complete,
            "readback_only": cli.readback_only
        }))
        .map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_STDOUT_ENCODE_FAILED",
                format!("encode on-chain backfill stdout: {err}"),
            )
        })?
    );
    result?;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "onchain_backfill",
        "require_passed",
        "POLY_ONCHAIN_BACKFILL_READBACK_PASSED",
        "on-chain backfill capture and physical readback passed",
        log_context(&[
            ("output_root", cli.request.output_root.display().to_string()),
            ("checked_file_count", report.checked_file_count.to_string()),
            ("total_pages", report.total_pages.to_string()),
        ]),
    )?)?;
    Ok(0)
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut request = OnchainBackfillRunRequest::target_default();
    let mut log_path = None;
    let mut readback_only = false;
    let mut readback_scope = OnchainBackfillReadbackScope::Full;
    let mut max_runtime_secs = DEFAULT_MAX_RUNTIME_SECS;
    let mut heartbeat_secs = DEFAULT_HEARTBEAT_INTERVAL_SECS;
    let mut lease_path = None;
    let mut takeover = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--readback-only" => readback_only = true,
            "--readback-scope" => {
                index += 1;
                readback_scope = parse_readback_scope(&args, index, "--readback-scope")?;
            }
            "--takeover" => takeover = true,
            "--max-runtime-secs" => {
                index += 1;
                max_runtime_secs = parse_number(&args, index, "--max-runtime-secs")?;
            }
            "--heartbeat-secs" => {
                index += 1;
                heartbeat_secs = parse_number(&args, index, "--heartbeat-secs")?;
            }
            "--lease-path" => {
                index += 1;
                lease_path = Some(PathBuf::from(required_value(&args, index, "--lease-path")?));
            }
            "--state-path" => {
                index += 1;
                request.state_path = PathBuf::from(required_value(&args, index, "--state-path")?);
            }
            "--output-root" => {
                index += 1;
                request.output_root = PathBuf::from(required_value(&args, index, "--output-root")?);
            }
            "--timeout-secs" => {
                index += 1;
                request.timeout_secs = parse_number(&args, index, "--timeout-secs")?;
            }
            "--max-body-bytes" => {
                index += 1;
                request.max_body_bytes = parse_number(&args, index, "--max-body-bytes")?;
            }
            "--max-chunks-per-contract" => {
                index += 1;
                request.max_chunks_per_contract =
                    parse_number(&args, index, "--max-chunks-per-contract")?;
            }
            "--max-blocks-per-chunk" => {
                index += 1;
                request.max_blocks_per_chunk =
                    parse_number(&args, index, "--max-blocks-per-chunk")?;
            }
            "--log" => {
                index += 1;
                log_path = Some(PathBuf::from(required_value(&args, index, "--log")?));
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        index += 1;
    }
    if max_runtime_secs == 0 {
        return Err(usage_error("--max-runtime-secs must be greater than zero"));
    }
    if heartbeat_secs == 0 {
        return Err(usage_error("--heartbeat-secs must be greater than zero"));
    }
    let log_path = log_path.unwrap_or_else(|| request.output_root.join("onchain-backfill.jsonl"));
    Ok(CliAction::Run(Cli {
        request,
        log_path,
        readback_only,
        readback_scope,
        max_runtime_secs,
        heartbeat_secs,
        lease_path,
        takeover,
    }))
}

/// Best-effort reconstruction of this process's command line for the lease
/// record (so an operator inspecting a leftover lease can see how it was run).
fn onchain_backfill_command_line() -> String {
    env::args().collect::<Vec<_>>().join(" ")
}

fn parse_number<T>(args: &[String], index: usize, flag: &str) -> calyx_poly::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = required_value(args, index, flag)?;
    value.parse::<T>().map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_ARG_PARSE_FAILED",
            format!("parse {flag} value {value}: {err}"),
        )
    })
}

fn parse_readback_scope(
    args: &[String],
    index: usize,
    flag: &str,
) -> calyx_poly::Result<OnchainBackfillReadbackScope> {
    match required_value(args, index, flag)? {
        "full" => Ok(OnchainBackfillReadbackScope::Full),
        "current-run" | "current_run" => Ok(OnchainBackfillReadbackScope::CurrentRun),
        value => Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_READBACK_SCOPE_INVALID",
            format!("{flag} must be full or current-run, got {value}"),
        )),
    }
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(
        "POLY_ONCHAIN_BACKFILL_USAGE",
        format!("{}\n{}", message.into(), usage()),
    )
}

fn usage() -> String {
    "usage: calyx-poly-onchain-backfill [--readback-only] [--readback-scope full|current-run] --state-path PATH [--output-root PATH] [--log PATH] [--timeout-secs N] [--max-body-bytes N] [--max-chunks-per-contract N] [--max-blocks-per-chunk N] [--max-runtime-secs N] [--heartbeat-secs N] [--lease-path PATH] [--takeover]\n\
     \n\
     Captures a bounded, resumable set of public Polygon OrderFilled eth_getLogs chunks from a persisted onchain-backfill-state.json.\n\
     Reuse the same --output-root to resume from the checkpoint. Use --readback-only to verify physical artifacts without network calls. The default readback scope is full; current-run verifies only current-run artifacts plus checkpoint membership for bounded continuation batches.\n\
     \n\
     Lease/lifecycle (issue #217): the process writes a lease file (pid + started_at + heartbeat) under --lease-path (default <output-root>/onchain-backfill.lease.json).\n\
     --max-runtime-secs N (default 1800) is a hard wall-clock ceiling; the watchdog force-terminates past it (safe: resumable from checkpoint).\n\
     A stale prior lease (heartbeat older than the staleness window, or past its deadline) is auto-reaped; a live lease is refused unless --takeover is given.\n\
     --timeout-secs N is a per-request network timeout, NOT the total command deadline (that is --max-runtime-secs)."
        .to_string()
}
