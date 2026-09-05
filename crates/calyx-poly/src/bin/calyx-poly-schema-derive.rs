use std::env;
use std::path::PathBuf;
use std::process;

use calyx_core::SystemClock;
use calyx_poly::{
    PolyError, PolyLogEvent, PolyResultLogExt, SchemaDerivationRequest, StructuredLogSink,
    log_context, read_schema_derivation_report, require_schema_derivation_passed,
    run_schema_derivation,
};
use serde_json::json;

struct Cli {
    request: SchemaDerivationRequest,
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
                    "{\"ok\":false,\"code\":\"POLY_SCHEMA_DERIVATION_ERROR_ENCODE_FAILED\"}"
                        .to_string()
                })
            );
            process::exit(2);
        }
    }
}

fn run() -> calyx_poly::Result<i32> {
    let CliAction::Run(cli) = parse_cli(env::args().skip(1).collect())? else {
        println!("{}", usage());
        return Ok(0);
    };
    let sink = StructuredLogSink::new(cli.log_path.clone())?;
    let clock = SystemClock;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "schema_derivation",
        "run",
        "POLY_SCHEMA_DERIVATION_STARTED",
        "deriving schema contract from persisted large corpus artifacts",
        log_context(&[
            ("corpus_root", cli.request.corpus_root.display().to_string()),
            ("output_root", cli.request.output_root.display().to_string()),
        ]),
    )?)?;

    let report = run_schema_derivation(&cli.request, &clock).log_error_context(
        &clock,
        &sink,
        "schema_derivation",
        "derive",
        log_context(&[
            ("corpus_root", cli.request.corpus_root.display().to_string()),
            ("output_root", cli.request.output_root.display().to_string()),
        ]),
    )?;
    let report_path = PathBuf::from(&report.output_root).join("schema-derivation-report.json");
    let readback = read_schema_derivation_report(&report_path).log_error_context(
        &clock,
        &sink,
        "schema_derivation",
        "readback_report",
        log_context(&[("report_path", report_path.display().to_string())]),
    )?;
    let result = require_schema_derivation_passed(&readback);
    if let Err(error) = &result {
        sink.append_error(
            &clock,
            "schema_derivation",
            "require_passed",
            error,
            log_context(&[
                ("report_path", report_path.display().to_string()),
                ("status_code", readback.status_code.clone()),
            ]),
        )?;
    }
    let stdout = json!({
        "ok": readback.passed,
        "status_code": readback.status_code,
        "report_path": report_path,
        "schema_contract_path": readback.schema_contract_path,
        "schema_decision_note_path": readback.schema_decision_note_path,
        "edge_audit_path": readback.edge_audit_path,
        "dataset_count": readback.dataset_count,
        "field_count": readback.field_count,
        "missing_required_sources": readback.missing_required_sources,
        "missing_required_join_keys": readback.missing_required_join_keys,
        "nullable_or_union_field_count": readback.nullable_or_union_field_count,
        "blocked_runtime_sources": readback.blocked_runtime_sources,
        "before_files": readback.before_files,
        "after_files": readback.after_files,
        "in_memory_report_status": report.status_code
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&stdout).map_err(|err| {
            PolyError::raw_source(
                "POLY_SCHEMA_DERIVATION_STDOUT_ENCODE_FAILED",
                format!("encode schema derivation summary: {err}"),
            )
        })?
    );
    result?;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "schema_derivation",
        "require_passed",
        "POLY_SCHEMA_DERIVATION_PASSED",
        "schema derivation contract passed and was read back from disk",
        log_context(&[
            ("report_path", report_path.display().to_string()),
            ("dataset_count", readback.dataset_count.to_string()),
            ("field_count", readback.field_count.to_string()),
        ]),
    )?)?;
    Ok(0)
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut corpus_root = None;
    let mut output_root = None;
    let mut log_path = None;
    let mut extra_sources = Vec::new();
    let mut extra_join_keys = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--corpus-root" => {
                index += 1;
                corpus_root = Some(PathBuf::from(required_value(
                    &args,
                    index,
                    "--corpus-root",
                )?));
            }
            "--output-root" => {
                index += 1;
                output_root = Some(PathBuf::from(required_value(
                    &args,
                    index,
                    "--output-root",
                )?));
            }
            "--require-source" => {
                index += 1;
                extra_sources.push(required_value(&args, index, "--require-source")?.to_string());
            }
            "--require-join-key" => {
                index += 1;
                extra_join_keys
                    .push(required_value(&args, index, "--require-join-key")?.to_string());
            }
            "--log" => {
                index += 1;
                log_path = Some(PathBuf::from(required_value(&args, index, "--log")?));
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        index += 1;
    }
    let corpus_root = corpus_root.ok_or_else(|| usage_error("--corpus-root is required"))?;
    let output_root = output_root.ok_or_else(|| usage_error("--output-root is required"))?;
    let mut request = SchemaDerivationRequest::new(corpus_root, output_root);
    request.required_sources.extend(extra_sources);
    request.required_join_keys.extend(extra_join_keys);
    let log_path = log_path.unwrap_or_else(|| request.output_root.join("schema-derive.jsonl"));
    Ok(CliAction::Run(Cli { request, log_path }))
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(
        "POLY_SCHEMA_DERIVATION_USAGE",
        format!("{}\n{}", message.into(), usage()),
    )
}

fn usage() -> String {
    "usage: calyx-poly-schema-derive --corpus-root PATH --output-root PATH [--log PATH] [--require-source NAME] [--require-join-key NAME]"
        .to_string()
}
