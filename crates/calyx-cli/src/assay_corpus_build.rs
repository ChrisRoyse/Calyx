//! Build an Assay corpus by measuring real registered lens runtimes.
//!
//! `assay bits-validate` consumes `vectors.jsonl` plus sidecar metadata. This
//! command creates those files through the Calyx registry runtimes themselves so
//! FSV does not depend on an out-of-tree embedding script.

mod data;
mod lens;
mod request;
mod write;

use crate::error::CliResult;
use crate::output::print_json;

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = request::CorpusBuildRequest::parse(args)?;
    let rows = data::load_rows(&request)?;
    let lenses = lens::load_lenses(&request)?;
    let measured = lens::measure_lenses(&request, &rows, lenses)?;
    let evidence = write::write_outputs(&request, &rows, &measured)?;
    print_json(&evidence)
}
