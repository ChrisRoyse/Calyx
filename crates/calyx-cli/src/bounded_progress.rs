use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use calyx_core::CalyxError;
use serde_json::Value;

use crate::error::{CliError, CliResult};

const CALYX_CLI_TIMEOUT: &str = "CALYX_CLI_TIMEOUT";
const TIMEOUT_REMEDIATION: &str =
    "increase the time budget, narrow the requested range, or inspect the emitted progress phase";

pub(crate) struct Deadline {
    start: Instant,
    budget: Option<Duration>,
}

impl Deadline {
    pub(crate) fn new(budget_ms: Option<u64>) -> Self {
        Self {
            start: Instant::now(),
            budget: budget_ms.map(Duration::from_millis),
        }
    }

    pub(crate) fn elapsed_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    pub(crate) fn check(&self, operation: &'static str, phase: &str, processed: u64) -> CliResult {
        let Some(budget) = self.budget else {
            return Ok(());
        };
        let elapsed = self.start.elapsed();
        if elapsed <= budget {
            return Ok(());
        }
        Err(CalyxError {
            code: CALYX_CLI_TIMEOUT,
            message: format!(
                "{operation} exceeded {} ms during {phase} after processing {processed} rows",
                budget.as_millis()
            ),
            remediation: TIMEOUT_REMEDIATION,
        }
        .into())
    }
}

pub(crate) enum ProgressSink {
    Disabled,
    Stderr,
    File(File),
}

impl ProgressSink {
    pub(crate) fn from_arg(arg: Option<&str>) -> CliResult<Self> {
        let Some(raw) = arg else {
            return Ok(Self::Disabled);
        };
        if matches!(raw, "stderr" | "-") {
            return Ok(Self::Stderr);
        }
        let path = PathBuf::from(raw);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| {
                CliError::io(format!(
                    "open progress JSONL {} for append: {error}",
                    path.display()
                ))
            })?;
        Ok(Self::File(file))
    }

    pub(crate) fn emit(&mut self, value: Value) -> CliResult {
        match self {
            Self::Disabled => Ok(()),
            Self::Stderr => {
                eprintln!("{}", serialize_progress(&value)?);
                Ok(())
            }
            Self::File(file) => {
                writeln!(file, "{}", serialize_progress(&value)?).map_err(CliError::from)?;
                file.flush().map_err(CliError::from)
            }
        }
    }
}

fn serialize_progress(value: &Value) -> CliResult<String> {
    serde_json::to_string(value)
        .map_err(|error| CliError::runtime(format!("serialize progress event: {error}")))
}

pub(crate) fn parse_nonzero_usize(raw: &str, flag: &str) -> CliResult<usize> {
    let parsed = raw
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("invalid {flag} {raw}: {error}")))?;
    if parsed == 0 {
        return Err(CliError::usage(format!("{flag} must be at least 1")));
    }
    Ok(parsed)
}

pub(crate) fn parse_nonzero_u64(raw: &str, flag: &str) -> CliResult<u64> {
    let parsed = raw
        .parse::<u64>()
        .map_err(|error| CliError::usage(format!("invalid {flag} {raw}: {error}")))?;
    if parsed == 0 {
        return Err(CliError::usage(format!("{flag} must be at least 1")));
    }
    Ok(parsed)
}
