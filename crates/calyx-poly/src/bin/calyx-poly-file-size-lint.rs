use std::env;
use std::path::PathBuf;
use std::process;

use calyx_poly::{
    DEFAULT_FILE_SIZE_LINE_LIMIT, FileSizeLintRequest, PolyError, PolyLogEvent, PolyResultLogExt,
    StructuredLogSink, evaluate_file_size_lint, log_context, require_file_size_lint_passed,
    write_file_size_lint_report,
};
use serde_json::json;

struct Cli {
    roots: Vec<PathBuf>,
    crate_root: PathBuf,
    report_path: PathBuf,
    log_path: PathBuf,
    line_limit: usize,
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
                    "{\"ok\":false,\"code\":\"POLY_FILE_SIZE_LINT_ERROR_ENCODE_FAILED\"}"
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
    let request = if cli.roots.is_empty() {
        let mut request = FileSizeLintRequest::calyx_poly_crate(&cli.crate_root);
        request.line_limit = cli.line_limit;
        request
    } else {
        FileSizeLintRequest {
            roots: cli.roots,
            line_limit: cli.line_limit,
        }
    };

    let start_event = PolyLogEvent::info(
        "file_size_lint",
        "evaluate",
        "POLY_FILE_SIZE_LINT_STARTED",
        "evaluating local file-size lint gate",
        log_context(&[
            ("report_path", cli.report_path.display().to_string()),
            ("log_path", sink.path().display().to_string()),
            ("root_count", request.roots.len().to_string()),
            ("line_limit", request.line_limit.to_string()),
        ]),
    )?;
    sink.append_event(&start_event)?;

    let report = evaluate_file_size_lint(&request);
    write_file_size_lint_report(&cli.report_path, &report).log_error_context(
        &sink,
        "file_size_lint",
        "write_report",
        log_context(&[("report_path", cli.report_path.display().to_string())]),
    )?;
    let stdout_json = serde_json::to_string_pretty(&report).map_err(|err| {
        PolyError::file_size_lint(
            "POLY_FILE_SIZE_LINT_STDOUT_ENCODE",
            format!("encode report for stdout: {err}"),
        )
    })?;
    println!("{stdout_json}");

    match require_file_size_lint_passed(&report) {
        Ok(()) => {
            let passed_event = PolyLogEvent::info(
                "file_size_lint",
                "require_passed",
                report.status_code.clone(),
                "file-size lint report passed and was readied for caller inspection",
                log_context(&[
                    ("report_path", cli.report_path.display().to_string()),
                    ("checked_file_count", report.checked_file_count.to_string()),
                    ("max_line_count", report.max_line_count.to_string()),
                ]),
            )?;
            sink.append_event(&passed_event)?;
            Ok(0)
        }
        Err(error) => {
            sink.append_error(
                "file_size_lint",
                "require_passed",
                &error,
                log_context(&[
                    ("report_path", cli.report_path.display().to_string()),
                    ("status_code", report.status_code.clone()),
                ]),
            )?;
            Ok(1)
        }
    }
}

fn parse_cli(args: Vec<String>) -> calyx_poly::Result<CliAction> {
    let mut roots = Vec::new();
    let mut crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut report_path = env::current_dir()
        .map_err(|err| {
            PolyError::file_size_lint(
                "POLY_FILE_SIZE_LINT_CURRENT_DIR",
                format!("read current directory: {err}"),
            )
        })?
        .join("target/fsv/file_size_lint/report.json");
    let mut log_path = None;
    let mut line_limit = DEFAULT_FILE_SIZE_LINE_LIMIT;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => return Ok(CliAction::Help),
            "--root" => {
                index += 1;
                roots.push(PathBuf::from(required_value(&args, index, "--root")?));
            }
            "--crate-root" => {
                index += 1;
                crate_root = PathBuf::from(required_value(&args, index, "--crate-root")?);
            }
            "--report" => {
                index += 1;
                report_path = PathBuf::from(required_value(&args, index, "--report")?);
            }
            "--log" => {
                index += 1;
                log_path = Some(PathBuf::from(required_value(&args, index, "--log")?));
            }
            "--limit" => {
                index += 1;
                let value = required_value(&args, index, "--limit")?;
                line_limit = value.parse::<usize>().map_err(|err| {
                    PolyError::file_size_lint(
                        "POLY_FILE_SIZE_LINT_LIMIT_PARSE",
                        format!("parse --limit {value}: {err}"),
                    )
                })?;
            }
            other => return Err(usage_error(format!("unknown argument {other}"))),
        }
        index += 1;
    }

    let log_path = log_path.unwrap_or_else(|| report_path.with_extension("jsonl"));
    Ok(CliAction::Run(Cli {
        roots,
        crate_root,
        report_path,
        log_path,
        line_limit,
    }))
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> calyx_poly::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| usage_error(format!("{flag} requires a value")))
}

fn usage_error(message: impl Into<String>) -> PolyError {
    PolyError::config(
        "POLY_FILE_SIZE_LINT_USAGE",
        format!("{}\n{}", message.into(), usage()),
    )
}

fn usage() -> String {
    "usage: calyx-poly-file-size-lint [--crate-root PATH] [--root PATH ...] [--report PATH] [--log PATH] [--limit N]"
        .to_string()
}
