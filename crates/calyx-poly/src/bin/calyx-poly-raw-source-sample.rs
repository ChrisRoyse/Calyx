use std::env;
use std::path::PathBuf;
use std::process;

use calyx_poly::{
    PolyError, PolyLogEvent, PolyResultLogExt, RawSourceSamplingRequest, StructuredLogSink,
    log_context, read_raw_source_inventory, readback_raw_source_inventory,
    require_raw_source_readback_passed, require_raw_source_sampling_passed,
    run_polymarket_raw_source_sampling,
};
use serde_json::json;

struct Cli {
    request: RawSourceSamplingRequest,
    log_path: PathBuf,
}

enum CliAction {
    Help,
    Run(Cli),
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            let payload = json!({
                "ok": false,
                "error": error.diagnostic()
            });
            eprintln!(
                "{}",
                serde_json::to_string(&payload).unwrap_or_else(|_| {
                    "{\"ok\":false,\"code\":\"POLY_RAW_SOURCE_ERROR_ENCODE_FAILED\"}".to_string()
                })
            );
            process::exit(2);
        }
    }
}

fn run() -> calyx_poly::Result<i32> {
    let CliAction::Run(mut cli) = parse_cli(env::args().skip(1).collect())? else {
        println!("{}", usage());
        return Ok(0);
    };
    cli.request = cli.request.normalized()?;
    let sink = StructuredLogSink::new(cli.log_path.clone())?;
    let start_event = PolyLogEvent::info(
        "raw_source_sampling",
        "run",
        "POLY_RAW_SOURCE_SAMPLE_STARTED",
        "capturing public read-only Polymarket source samples",
        log_context(&[
            ("output_root", cli.request.output_root.display().to_string()),
            ("log_path", sink.path().display().to_string()),
            ("timeout_secs", cli.request.timeout_secs.to_string()),
            ("max_body_bytes", cli.request.max_body_bytes.to_string()),
        ]),
    )?;
    sink.append_event(&start_event)?;

    let inventory = run_polymarket_raw_source_sampling(cli.request.clone()).log_error_context(
        &sink,
        "raw_source_sampling",
        "capture",
        log_context(&[("output_root", cli.request.output_root.display().to_string())]),
    )?;
    let inventory_path = cli.request.output_root.join("source-inventory.json");
    let readback = read_raw_source_inventory(&inventory_path).log_error_context(
        &sink,
        "raw_source_sampling",
        "readback_inventory",
        log_context(&[("inventory_path", inventory_path.display().to_string())]),
    )?;
    let physical_readback = readback_raw_source_inventory(&cli.request.output_root)
        .log_error_context(
            &sink,
            "raw_source_sampling",
            "readback_physical_files",
            log_context(&[("output_root", cli.request.output_root.display().to_string())]),
        )?;
    if let Err(error) = require_raw_source_readback_passed(&physical_readback) {
        sink.append_error(
            "raw_source_sampling",
            "readback_physical_files",
            &error,
            log_context(&[
                ("output_root", cli.request.output_root.display().to_string()),
                ("failure_count", physical_readback.failure_count.to_string()),
            ]),
        )?;
        return Err(error);
    }
    let result = require_raw_source_sampling_passed(&readback);
    if let Err(error) = &result {
        sink.append_error(
            "raw_source_sampling",
            "require_passed",
            error,
            log_context(&[
                ("inventory_path", inventory_path.display().to_string()),
                ("status_code", readback.status_code.clone()),
            ]),
        )?;
    }

    let stdout = json!({
        "ok": readback.passed,
        "status_code": readback.status_code,
        "inventory_path": inventory_path,
        "sample_count": readback.coverage.sample_count,
        "required_success_count": readback.coverage.required_success_count,
        "required_failure_count": readback.coverage.required_failure_count,
        "edge_case_count": readback.coverage.edge_case_count,
        "total_body_bytes": readback.coverage.total_body_bytes,
        "readback_sha_mismatches": readback.coverage.readback_sha_mismatches,
        "docs_index_status_code": readback.docs_index_coverage.status_code,
        "docs_index_row_count": readback.docs_index_coverage.row_count,
        "docs_index_not_yet_sampled_count": readback.docs_index_coverage.not_yet_sampled_count,
        "docs_index_blocked_runtime_count": readback.docs_index_coverage.blocked_runtime_count,
        "join_map": readback.join_map,
        "unsampled_sources": readback.coverage.unsampled_sources
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&stdout).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_STDOUT_ENCODE_FAILED",
                format!("encode raw source summary: {err}"),
            )
        })?
    );

    result?;
    let status_code = inventory.status_code.clone();
    let done_event = PolyLogEvent::info(
        "raw_source_sampling",
        "require_passed",
        status_code,
        "raw source sampling passed and was read back from the physical corpus",
        log_context(&[
            ("inventory_path", inventory_path.display().to_string()),
            ("sample_count", inventory.coverage.sample_count.to_string()),
            (
                "total_body_bytes",
                inventory.coverage.total_body_bytes.to_string(),
            ),
        ]),
    )?;
    sink.append_event(&done_event)?;
    Ok(0)
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut request = RawSourceSamplingRequest::target_default();
    let mut log_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--output-root" => {
                index += 1;
                request.output_root = PathBuf::from(required_value(&args, index, "--output-root")?);
            }
            "--timeout-secs" => {
                index += 1;
                let value = required_value(&args, index, "--timeout-secs")?;
                request.timeout_secs = value.parse::<u64>().map_err(|err| {
                    PolyError::raw_source(
                        "POLY_RAW_SOURCE_TIMEOUT_PARSE_FAILED",
                        format!("parse --timeout-secs {value}: {err}"),
                    )
                })?;
            }
            "--max-body-bytes" => {
                index += 1;
                let value = required_value(&args, index, "--max-body-bytes")?;
                request.max_body_bytes = value.parse::<usize>().map_err(|err| {
                    PolyError::raw_source(
                        "POLY_RAW_SOURCE_BODY_LIMIT_PARSE_FAILED",
                        format!("parse --max-body-bytes {value}: {err}"),
                    )
                })?;
            }
            "--log" => {
                index += 1;
                log_path = Some(PathBuf::from(required_value(&args, index, "--log")?));
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        index += 1;
    }
    let log_path = log_path.unwrap_or_else(|| request.output_root.join("raw-source-sample.jsonl"));
    Ok(CliAction::Run(Cli { request, log_path }))
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(
        "POLY_RAW_SOURCE_USAGE",
        format!("{}\n{}", message.into(), usage()),
    )
}

fn usage() -> String {
    "usage: calyx-poly-raw-source-sample [--output-root PATH] [--log PATH] [--timeout-secs N] [--max-body-bytes N]\n\
     \n\
     --timeout-secs N is a per-source network/read timeout. It is not a total command deadline."
        .to_string()
}
