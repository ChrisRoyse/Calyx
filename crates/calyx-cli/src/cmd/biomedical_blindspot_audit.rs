//! `calyx biomedical-blindspot-audit` hardens biomedical hypothesis rows against
//! report-section blindspots: context inversion, dead drugs, literature novelty,
//! run stability, benchmark exportability, and transcriptomic specificity.

mod checks;
mod io;
mod model;
mod parse;
mod report;
mod source;
mod util;

#[cfg(test)]
mod tests;

pub(crate) use model::BiomedicalBlindspotAuditArgs;
pub(crate) use parse::parse_biomedical_blindspot_audit;

use crate::error::CliResult;
use crate::output::print_json;

use super::Subcommand;
use io::persist;
use model::CliSummary;
use report::build_report;

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::BiomedicalBlindspotAudit(args) = command else {
        unreachable!("non-biomedical-blindspot-audit command routed here");
    };
    let report = build_report(&args)?;
    let readback = persist(&args.out_dir, &report)?;
    print_json(&CliSummary {
        status: "ok",
        out_dir: args.out_dir.display().to_string(),
        report: readback.report.clone(),
        report_sha256: readback.report_sha256.clone(),
        audited_hypotheses_jsonl: readback.audited_hypotheses.clone(),
        audited_hypotheses_sha256: readback.audited_hypotheses_sha256.clone(),
        ready_hypotheses_jsonl: readback.ready_hypotheses.clone(),
        ready_hypotheses_sha256: readback.ready_hypotheses_sha256.clone(),
        blocked_hypotheses_jsonl: readback.blocked_hypotheses.clone(),
        blocked_hypotheses_sha256: readback.blocked_hypotheses_sha256.clone(),
        benchmark_export_jsonl: readback.benchmark_export.clone(),
        benchmark_export_sha256: readback.benchmark_export_sha256.clone(),
        metrics_json: readback.metrics.clone(),
        metrics_sha256: readback.metrics_sha256.clone(),
        audited_count: report.audited_count,
        ready_count: report.ready_count,
        blocked_count: report.blocked_count,
        pending_count: report.pending_count,
        readback,
    })
}
