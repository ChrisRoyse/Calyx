use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{
    CALYX_AB_RECALL_BELOW_BASELINE, CALYX_CONTRACT_NAME_MISMATCH, CALYX_LATENCY_REGRESSION,
    CALYX_ROUND_TRIP_GATE_FAILED, QueryVec, RecallBenchmark, RoundTripVerifier, VerifyReport,
    error,
};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Serialize)]
struct CliReport {
    summary: String,
    verify: VerifyReport,
    benchmark: Option<RecallBenchmark>,
    output_path: Option<PathBuf>,
}

pub(super) fn run_verify_round_trip(args: &[String]) -> CliResult {
    let args = parse_args(args)?;
    let verify = RoundTripVerifier::verify(&args.sqlite, &args.calyx)?;
    let benchmark = if args.benchmark {
        let queries = read_queries(
            args.queries
                .as_ref()
                .ok_or_else(|| CliError::usage("--benchmark requires --queries <jsonl>"))?,
        )?;
        Some(RoundTripVerifier::benchmark_recall(
            &args.sqlite,
            &args.calyx,
            &queries,
            args.top_k,
        )?)
    } else {
        None
    };
    let failed =
        !verify.gate_passes() || benchmark.as_ref().is_some_and(|bench| bench.gate != "PASS");
    let report = CliReport {
        summary: format!(
            "round-trip gate={} total={} matched={} mismatches={}",
            verify.gate,
            verify.total,
            verify.matched,
            verify.mismatches.len()
        ),
        verify,
        benchmark,
        output_path: args.output.clone(),
    };
    if let Some(path) = &args.output {
        write_json(path, &report)?;
    }
    print_json(&report)?;
    if failed {
        return Err(report_error(&report).into());
    }
    Ok(())
}

#[derive(Default)]
struct Args {
    sqlite: PathBuf,
    calyx: PathBuf,
    output: Option<PathBuf>,
    benchmark: bool,
    queries: Option<PathBuf>,
    top_k: usize,
}

fn parse_args(args: &[String]) -> CliResult<Args> {
    let mut out = Args {
        top_k: 10,
        ..Args::default()
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--sqlite" => out.sqlite = value(args, idx, "--sqlite")?.into(),
            "--calyx" => out.calyx = value(args, idx, "--calyx")?.into(),
            "--output" => out.output = Some(value(args, idx, "--output")?.into()),
            "--queries" => out.queries = Some(value(args, idx, "--queries")?.into()),
            "--top-k" => {
                out.top_k = value(args, idx, "--top-k")?
                    .parse()
                    .map_err(|err| CliError::usage(format!("parse --top-k: {err}")))?;
            }
            "--benchmark" => {
                out.benchmark = true;
                idx += 1;
                continue;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown verify-round-trip arg {other}"
                )));
            }
        }
        idx += 2;
    }
    if out.sqlite.as_os_str().is_empty() || out.calyx.as_os_str().is_empty() {
        return Err(CliError::usage(
            "usage: calyx leapable verify-round-trip --sqlite <db> --calyx <dir> [--output <json>] [--benchmark --queries <jsonl>] [--top-k <n>]",
        ));
    }
    Ok(out)
}

fn read_queries(path: &Path) -> CliResult<Vec<QueryVec>> {
    let file = File::open(path)
        .map_err(|err| CliError::io(format!("open queries {}: {err}", path.display())))?;
    let queries = BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            other => Some((idx, other)),
        })
        .map(|(idx, line)| {
            let line =
                line.map_err(|err| CliError::io(format!("read query line {}: {err}", idx + 1)))?;
            serde_json::from_str(&line)
                .map_err(|err| CliError::usage(format!("parse query line {}: {err}", idx + 1)))
        })
        .collect::<CliResult<Vec<_>>>()?;
    if queries.is_empty() {
        return Err(CliError::usage(
            "query file must contain at least one JSONL row",
        ));
    }
    Ok(queries)
}

fn write_json(path: &Path, report: &CliReport) -> CliResult {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::io(format!("create {}: {err}", parent.display())))?;
    }
    let mut file = File::create(path)
        .map_err(|err| CliError::io(format!("create {}: {err}", path.display())))?;
    serde_json::to_writer_pretty(&mut file, report)
        .map_err(|err| CliError::io(format!("write {}: {err}", path.display())))?;
    file.write_all(b"\n")
        .map_err(|err| CliError::io(format!("write newline {}: {err}", path.display())))?;
    file.sync_all()
        .map_err(|err| CliError::io(format!("sync {}: {err}", path.display())))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn report_error(report: &CliReport) -> calyx_core::CalyxError {
    if report
        .verify
        .mismatches
        .iter()
        .any(|detail| detail.code == CALYX_CONTRACT_NAME_MISMATCH)
    {
        return error(
            CALYX_CONTRACT_NAME_MISMATCH,
            "round-trip database_name contract mismatch",
            "inspect verify report mismatches and rerun migration from the source SQLite DB",
        );
    }
    if !report.verify.gate_passes() {
        return error(
            CALYX_ROUND_TRIP_GATE_FAILED,
            "round-trip verify gate failed",
            "inspect verify report mismatches and rerun migration from the source SQLite DB",
        );
    }
    if let Some(benchmark) = &report.benchmark {
        if benchmark.calyx_mean_recall + f64::EPSILON < benchmark.sqlite_mean_recall {
            return error(
                CALYX_AB_RECALL_BELOW_BASELINE,
                "Calyx recall fell below sqlite baseline",
                "inspect benchmark query rows and rerun migration before enabling V1",
            );
        }
        if benchmark.latency_calyx_p99_us > benchmark.latency_budget_us {
            return error(
                CALYX_LATENCY_REGRESSION,
                "Calyx p99 latency exceeded sqlite baseline budget",
                "inspect benchmark query rows and profile the Calyx read path",
            );
        }
    }
    error(
        CALYX_ROUND_TRIP_GATE_FAILED,
        "round-trip verify gate failed",
        "inspect verify report and rerun migration",
    )
}
