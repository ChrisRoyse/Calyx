//! Export a persisted multi-lens Assay corpus into Sextant `.fbin` inputs.
//!
//! This bridges `assay corpus-build` / `assay bits-validate` into the PH68
//! partitioned-RRF scale gate without relying on an out-of-tree converter.

mod args;
mod data;
mod timeline;
mod write;

use calyx_core::CalyxError;
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

use args::Args;

pub(super) const MIN_A35_LENSES: usize = 10;
pub(super) const DEFAULT_MIN_BITS: f32 = 0.05;

#[derive(Clone, Debug, Serialize)]
pub(super) struct ExportEvidence {
    pub(super) out_dir: String,
    pub(super) vectors_path: String,
    pub(super) plan_path: String,
    pub(super) export_report_path: String,
    pub(super) timeline_path: String,
    pub(super) vault_root: String,
    pub(super) rows: usize,
    pub(super) query_count: usize,
    pub(super) temporal: timeline::TimelineScan,
    pub(super) lens_roster: Vec<LensEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct LensEvidence {
    pub(super) slot: u16,
    pub(super) name: String,
    pub(super) lens_id: String,
    pub(super) weights_sha256: String,
    pub(super) signal_kind: String,
    pub(super) bits_about: f32,
    pub(super) dim: usize,
    pub(super) native_dim: usize,
    pub(super) assay_projection: String,
    pub(super) corpus_path: String,
    pub(super) queries_path: String,
    pub(super) vault_path: String,
    pub(super) corpus_rows_written: usize,
    pub(super) query_rows_written: usize,
}

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let evidence = export_fbin(&args)?;
    print_json(&evidence)
}

fn export_fbin(args: &Args) -> CliResult<ExportEvidence> {
    write::ensure_fresh_output(args)?;
    let vectors_path = args.corpus_dir.join("vectors.jsonl");
    let scan = data::scan_vectors(&vectors_path)?;
    if args.query_count > scan.rows {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_QUERY_TOO_LARGE",
            format!(
                "query_count={} exceeds corpus rows={}",
                args.query_count, scan.rows
            ),
            "choose a query-count at or below the persisted vectors.jsonl row count",
        ));
    }
    let catalog = data::load_lens_catalog(&args.corpus_dir, &scan.lens_dims)?;
    let bits = data::load_bits_report(&args.bits_report)?;
    let selected = data::selected_lenses(&catalog.order, &catalog.meta, &bits, args.min_bits)?;
    write::write_export(args, &vectors_path, &scan, &catalog.meta, &bits, &selected)
}

pub(super) fn local_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

pub(super) fn io_error(error: std::io::Error) -> CliError {
    CliError::io(error.to_string())
}

#[cfg(test)]
#[path = "assay_fbin_export_tests.rs"]
mod tests;
