//! `calyx materialize-evidence-substrate` writes biomedical evidence,
//! outcome, source, hash, and validation rows into an Aster Graph CF
//! PlainGraph collection for association discovery.

use std::path::{Path, PathBuf};

use super::vault::home_dir;
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod model;
mod source;
#[cfg(test)]
mod tests;
mod write;

pub(crate) const DEFAULT_COLLECTION: &str = "biomed_evidence_substrate";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializeEvidenceSubstrateArgs {
    pub vault: String,
    pub pubtator_root: PathBuf,
    pub clinicaltrials_root: PathBuf,
    pub dgidb_root: PathBuf,
    pub collection: Option<String>,
    pub report: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

pub(crate) fn parse_materialize_evidence_substrate(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-evidence-substrate requires <vault>"))?
        .clone();
    let mut pubtator_root = None;
    let mut clinicaltrials_root = None;
    let mut dgidb_root = None;
    let mut collection = None;
    let mut report = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--pubtator-root" => {
                idx += 1;
                pubtator_root = Some(value(rest, idx, "--pubtator-root")?.into());
            }
            "--clinicaltrials-root" => {
                idx += 1;
                clinicaltrials_root = Some(value(rest, idx, "--clinicaltrials-root")?.into());
            }
            "--dgidb-root" => {
                idx += 1;
                dgidb_root = Some(value(rest, idx, "--dgidb-root")?.into());
            }
            "--collection" => {
                idx += 1;
                collection = Some(value(rest, idx, "--collection")?.to_string());
            }
            "--report" => {
                idx += 1;
                report = Some(value(rest, idx, "--report")?.into());
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-evidence-substrate flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::MaterializeEvidenceSubstrate(
        MaterializeEvidenceSubstrateArgs {
            vault,
            pubtator_root: pubtator_root.ok_or_else(|| {
                CliError::usage("materialize-evidence-substrate requires --pubtator-root <dir>")
            })?,
            clinicaltrials_root: clinicaltrials_root.ok_or_else(|| {
                CliError::usage(
                    "materialize-evidence-substrate requires --clinicaltrials-root <dir>",
                )
            })?,
            dgidb_root: dgidb_root.ok_or_else(|| {
                CliError::usage("materialize-evidence-substrate requires --dgidb-root <dir>")
            })?,
            collection,
            report,
            home,
        },
    ))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::MaterializeEvidenceSubstrate(args) = command else {
        unreachable!("non-materialize-evidence-substrate command routed here");
    };
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let report = materialize_with_home(&home, args)?;
    print_json(&report)
}

fn materialize_with_home(
    home: &Path,
    args: MaterializeEvidenceSubstrateArgs,
) -> CliResult<write::MaterializeEvidenceSubstrateReport> {
    let (draft, source_report) = source::load_sources(
        &args.pubtator_root,
        &args.clinicaltrials_root,
        &args.dgidb_root,
    )?;
    write::write_to_calyx(home, &args, draft, source_report)
}
