use std::path::PathBuf;

use crate::error::{CliError, CliResult};

#[derive(Default)]
pub(crate) struct Flags {
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) home: Option<PathBuf>,
    pub(crate) input: Option<String>,
    pub(crate) input_file: Option<PathBuf>,
    pub(crate) repeat: Option<usize>,
    pub(crate) full_vector: bool,
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
                "--input-file" => {
                    idx += 1;
                    flags.input_file = Some(value(args, idx, "--input-file")?.into());
                }
                "--repeat" => {
                    idx += 1;
                    let raw = value(args, idx, "--repeat")?;
                    flags.repeat = Some(raw.parse().map_err(|err| {
                        CliError::usage(format!("parse --repeat value {raw}: {err}"))
                    })?);
                }
                "--full-vector" => {
                    flags.full_vector = true;
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
        if self.input.is_some()
            || self.input_file.is_some()
            || self.repeat.is_some()
            || self.full_vector
        {
            return Err(CliError::usage(format!(
                "{command} does not accept --input, --input-file, --repeat, or --full-vector"
            )));
        }
        Ok(())
    }

    pub(crate) fn reject_list_flags(&self, command: &str) -> CliResult {
        if self.home.is_some() || self.repeat.is_some() || self.full_vector {
            return Err(CliError::usage(format!(
                "{command} does not accept --home, --repeat, or --full-vector"
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
