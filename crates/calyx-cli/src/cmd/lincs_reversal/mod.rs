//! `calyx materialize-lincs-reversal` writes CREEDS disease signatures,
//! L1000CDS2 reverse perturbation scores, and unsupported candidate-drug
//! evidence into an Aster Graph CF PlainGraph collection.

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

pub(crate) const DEFAULT_COLLECTION: &str = "biomed_lincs_cmap_reversal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializeLincsReversalArgs {
    pub vault: String,
    pub root: PathBuf,
    pub metadata_root: Option<PathBuf>,
    pub collection: Option<String>,
    pub report: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

pub(crate) fn parse_materialize_lincs_reversal(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-lincs-reversal requires <vault>"))?
        .clone();
    let mut root = None;
    let mut metadata_root = None;
    let mut collection = None;
    let mut report = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--root" => {
                idx += 1;
                root = Some(value(rest, idx, "--root")?.into());
            }
            "--metadata-root" => {
                idx += 1;
                metadata_root = Some(value(rest, idx, "--metadata-root")?.into());
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
                    "unexpected materialize-lincs-reversal flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::MaterializeLincsReversal(
        MaterializeLincsReversalArgs {
            vault,
            root: root.ok_or_else(|| {
                CliError::usage("materialize-lincs-reversal requires --root <dir>")
            })?,
            metadata_root,
            collection,
            report,
            home,
        },
    ))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::MaterializeLincsReversal(args) = command else {
        unreachable!("non-materialize-lincs-reversal command routed here");
    };
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let report = materialize_with_home(&home, args)?;
    print_json(&report)
}

fn materialize_with_home(
    home: &Path,
    args: MaterializeLincsReversalArgs,
) -> CliResult<write::MaterializeLincsReversalReport> {
    eprintln!(
        "lincs-reversal: verifying source root {}",
        args.root.display()
    );
    let (draft, source_report) = source::load_root(&args.root, args.metadata_root.as_deref())?;
    eprintln!(
        "lincs-reversal: source verified nodes={} edges={}",
        draft.nodes.len(),
        draft.edges.len()
    );
    write::write_to_calyx(home, &args, draft, source_report)
}
