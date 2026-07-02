mod data;
mod engine;
mod metrics;
mod request;

use data::CorpusSet;
use engine::evaluate_corpora;
use metrics::write_metric_outputs;
use request::LodestarKernelRequest;

use crate::error::CliError;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = LodestarKernelRequest::parse(args).map_err(CliError::usage)?;
    let data = CorpusSet::load(&request.corpora_dir)?;
    let report = evaluate_corpora(&data, &request)?;
    let evidence = write_metric_outputs(&request, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence)
            .map_err(|error| CliError::runtime(format!("serialize evidence: {error}")))?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
