use std::env;
use std::path::PathBuf;
use std::process;

use calyx_poly::{
    LargeCorpusRequest, PolyError, PolyLogEvent, PolyResultLogExt, StructuredLogSink, log_context,
    read_large_corpus_manifest, readback_large_corpus_with_exhaustive, require_large_corpus_passed,
    run_large_corpus_capture,
};
use serde_json::json;

struct Cli {
    request: LargeCorpusRequest,
    log_path: PathBuf,
    readback_only: bool,
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
                    "{\"ok\":false,\"code\":\"POLY_LARGE_CORPUS_ERROR_ENCODE_FAILED\"}".to_string()
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
    sink.append_event(&PolyLogEvent::info(
        "large_corpus",
        "run",
        "POLY_LARGE_CORPUS_CAPTURE_STARTED",
        "capturing large public read-only Polymarket corpus sample",
        log_context(&[
            ("output_root", cli.request.output_root.display().to_string()),
            ("log_path", sink.path().display().to_string()),
            ("page_size", cli.request.page_size.to_string()),
            (
                "max_pages_per_dataset",
                cli.request.max_pages_per_dataset.to_string(),
            ),
            (
                "require_exhaustive",
                cli.request.require_exhaustive.to_string(),
            ),
        ]),
    )?)?;

    let manifest = if cli.readback_only {
        read_large_corpus_manifest(&cli.request.output_root.join("large-corpus-manifest.json"))
            .log_error_context(
                &sink,
                "large_corpus",
                "read_manifest",
                log_context(&[("output_root", cli.request.output_root.display().to_string())]),
            )?
    } else {
        run_large_corpus_capture(cli.request.clone()).log_error_context(
            &sink,
            "large_corpus",
            "capture",
            log_context(&[("output_root", cli.request.output_root.display().to_string())]),
        )?
    };
    let report = readback_large_corpus_with_exhaustive(
        &cli.request.output_root,
        Some(cli.request.require_exhaustive),
    )
    .log_error_context(
        &sink,
        "large_corpus",
        "readback",
        log_context(&[("output_root", cli.request.output_root.display().to_string())]),
    )?;
    let result = require_large_corpus_passed(&report);
    if let Err(error) = &result {
        sink.append_error(
            "large_corpus",
            "require_passed",
            error,
            log_context(&[
                ("output_root", cli.request.output_root.display().to_string()),
                ("status_code", report.status_code.clone()),
            ]),
        )?;
    }

    let stdout = json!({
        "ok": report.passed,
        "status_code": report.status_code,
        "output_root": cli.request.output_root,
        "manifest_path": report.manifest_path,
        "readback_report_path": cli.request.output_root.join("large-corpus-readback-report.json"),
        "total_pages": report.total_pages,
        "total_records": report.total_records,
        "total_body_bytes": report.total_body_bytes,
        "edge_case_count": report.edge_case_count,
        "capture_goal": &report.capture_goal,
        "require_exhaustive": report.require_exhaustive,
        "bounded_incomplete_datasets": &report.bounded_incomplete_datasets,
        "checked_file_count": report.checked_file_count,
        "missing_files": report.missing_files,
        "sha_mismatches": report.sha_mismatches,
        "parse_failures": report.parse_failures,
            "manifest_status_code": manifest.status_code,
            "readback_only": cli.readback_only,
        "field_profile_paths": manifest.field_profile_paths,
        "join_profile_path": manifest.join_profile_path,
        "schema_decision_input_path": manifest.schema_decision_input_path
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&stdout).map_err(|err| {
            PolyError::raw_source(
                "POLY_LARGE_CORPUS_STDOUT_ENCODE_FAILED",
                format!("encode large corpus summary: {err}"),
            )
        })?
    );
    result?;
    sink.append_event(&PolyLogEvent::info(
        "large_corpus",
        "require_passed",
        "POLY_LARGE_CORPUS_READBACK_PASSED",
        "large corpus capture and physical readback passed",
        log_context(&[
            ("output_root", cli.request.output_root.display().to_string()),
            ("total_records", report.total_records.to_string()),
            ("checked_file_count", report.checked_file_count.to_string()),
        ]),
    )?)?;
    Ok(0)
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut request = LargeCorpusRequest::target_default();
    let mut log_path = None;
    let mut readback_only = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--readback-only" => {
                readback_only = true;
            }
            "--require-exhaustive" => {
                request.require_exhaustive = true;
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
            "--page-size" => {
                index += 1;
                request.page_size = parse_number(&args, index, "--page-size")?;
            }
            "--max-pages-per-dataset" => {
                index += 1;
                request.max_pages_per_dataset =
                    parse_number(&args, index, "--max-pages-per-dataset")?;
            }
            "--log" => {
                index += 1;
                log_path = Some(PathBuf::from(required_value(&args, index, "--log")?));
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        index += 1;
    }
    let log_path = log_path.unwrap_or_else(|| request.output_root.join("large-corpus.jsonl"));
    Ok(CliAction::Run(Cli {
        request,
        log_path,
        readback_only,
    }))
}

fn parse_number<T>(args: &[String], index: usize, flag: &str) -> calyx_poly::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = required_value(args, index, flag)?;
    value.parse::<T>().map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ARG_PARSE_FAILED",
            format!("parse {flag} value {value}: {err}"),
        )
    })
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(
        "POLY_LARGE_CORPUS_USAGE",
        format!("{}\n{}", message.into(), usage()),
    )
}

fn usage() -> String {
    "usage: calyx-poly-large-corpus-sample [--readback-only] [--require-exhaustive] [--output-root PATH] [--log PATH] [--timeout-secs N] [--max-body-bytes N] [--page-size N] [--max-pages-per-dataset N]\n\
     \n\
     --require-exhaustive fails closed if a paginated dataset hits --max-pages-per-dataset without a terminal empty/short page.\n\
     --timeout-secs N is a per-source network/read timeout. It is not a total command deadline."
        .to_string()
}
