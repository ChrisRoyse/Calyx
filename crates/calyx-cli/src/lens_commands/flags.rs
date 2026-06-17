use std::path::PathBuf;

use crate::error::{CliError, CliResult};

#[derive(Default)]
pub(crate) struct Flags {
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) home: Option<PathBuf>,
    pub(crate) input: Option<String>,
    pub(crate) repeat: Option<usize>,
}

impl Flags {
    pub(crate) fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--manifest" => {
                    idx += 1;
                    flags.manifest = Some(value(args, idx, "--manifest")?.into());
                }
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--input" => {
                    idx += 1;
                    flags.input = Some(value(args, idx, "--input")?.to_string());
                }
                "--repeat" => {
                    idx += 1;
                    let raw = value(args, idx, "--repeat")?;
                    flags.repeat = Some(raw.parse().map_err(|err| {
                        CliError::usage(format!("parse --repeat value {raw}: {err}"))
                    })?);
                }
                other => {
                    return Err(CliError::usage(format!("unexpected lens flag {other}")));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }

    pub(crate) fn reject_measure_flags(&self, command: &str) -> CliResult {
        if self.input.is_some() || self.repeat.is_some() {
            return Err(CliError::usage(format!(
                "{command} does not accept --input or --repeat"
            )));
        }
        Ok(())
    }
}

pub(crate) fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}
