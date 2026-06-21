//! Build an Assay corpus by measuring real registered lens runtimes.
//!
//! `assay bits-validate` consumes `vectors.jsonl` plus sidecar metadata. This
//! command creates those files through the Calyx registry runtimes themselves so
//! FSV does not depend on an out-of-tree embedding script.

mod data;
pub(crate) mod lens;
pub(crate) mod request;
mod worker;
mod write;

use crate::error::CliResult;
use crate::output::print_json;

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = request::CorpusBuildRequest::parse(args)?;
    if request.worker_report.is_some() {
        return worker::run_worker(&request);
    }
    write::ensure_fresh_output(&request)?;
    let rows = data::load_rows(&request)?;
    let measured = worker::measure_requested_lenses(&request, &rows)?;
    let evidence = write::write_outputs(&request, &rows, &measured)?;
    print_json(&evidence)
}

#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod tests;
