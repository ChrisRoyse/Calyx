use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::command_types::{AddLensArgs, ProfileLensArgs, Subcommand};
use super::parse_helpers::value;

pub(super) fn parse_add_lens(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("add-lens requires <vault>"))?
        .clone();
    let mut flags = LensFlags::default();
    flags.parse(&rest[1..], "add-lens")?;
    let name = flags
        .name
        .ok_or_else(|| CliError::usage("add-lens requires --name <n>"))?;
    let runtime = flags
        .runtime
        .ok_or_else(|| CliError::usage("add-lens requires --runtime <r>"))?;
    Ok(Subcommand::AddLens(AddLensArgs {
        vault,
        name,
        runtime,
        endpoint: flags.endpoint,
        weights: flags.weights,
        shape: flags.shape,
        modality: flags.modality,
    }))
}

pub(super) fn parse_profile_lens(rest: &[String]) -> CliResult<Subcommand> {
    let mut flags = LensFlags::default();
    flags.parse(rest, "profile-lens")?;
    Ok(Subcommand::ProfileLens(ProfileLensArgs {
        name: flags.name,
        runtime: flags.runtime,
        endpoint: flags.endpoint,
        weights: flags.weights,
        shape: flags.shape,
        modality: flags.modality,
        probe: flags.probe,
    }))
}

#[derive(Default)]
struct LensFlags {
    name: Option<String>,
    runtime: Option<String>,
    endpoint: Option<String>,
    weights: Option<PathBuf>,
    shape: Option<String>,
    modality: Option<String>,
    probe: Option<PathBuf>,
}

impl LensFlags {
    fn parse(&mut self, args: &[String], command: &str) -> CliResult {
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--name" => {
                    idx += 1;
                    self.name = Some(value(args, idx, "--name")?.to_string());
                }
                "--runtime" => {
                    idx += 1;
                    self.runtime = Some(value(args, idx, "--runtime")?.to_string());
                }
                "--endpoint" => {
                    idx += 1;
                    self.endpoint = Some(value(args, idx, "--endpoint")?.to_string());
                }
                "--weights" => {
                    idx += 1;
                    self.weights = Some(value(args, idx, "--weights")?.into());
                }
                "--shape" => {
                    idx += 1;
                    self.shape = Some(value(args, idx, "--shape")?.to_string());
                }
                "--modality" => {
                    idx += 1;
                    self.modality = Some(value(args, idx, "--modality")?.to_string());
                }
                "--probe" if command == "profile-lens" => {
                    idx += 1;
                    self.probe = Some(value(args, idx, "--probe")?.into());
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected {command} flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(())
    }
}
