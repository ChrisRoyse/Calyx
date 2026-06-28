use super::*;

impl Flags {
    pub(super) fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--template" => {
                    idx += 1;
                    flags.template = Some(value(args, idx, "--template")?.to_string());
                }
                "--hold-secs" => {
                    idx += 1;
                    let raw = value(args, idx, "--hold-secs")?;
                    flags.hold_secs = raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --hold-secs {raw}: {err}"))
                    })?;
                }
                "--out" => {
                    idx += 1;
                    flags.out = Some(value(args, idx, "--out")?.into());
                }
                "--progress-out" => {
                    idx += 1;
                    flags.progress_out = Some(value(args, idx, "--progress-out")?.into());
                }
                "--max-resident-vram-mib" => {
                    idx += 1;
                    let raw = value(args, idx, "--max-resident-vram-mib")?;
                    flags.max_resident_vram_mib = Some(raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --max-resident-vram-mib {raw}: {err}"))
                    })?);
                }
                "--resident-overhead-multiplier" => {
                    idx += 1;
                    let raw = value(args, idx, "--resident-overhead-multiplier")?;
                    flags.resident_overhead_multiplier_milli = Some(parse_multiplier_milli(raw)?);
                }
                "--max-load-secs" => {
                    idx += 1;
                    let raw = value(args, idx, "--max-load-secs")?;
                    flags.max_load_secs = Some(raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --max-load-secs {raw}: {err}"))
                    })?);
                }
                "--load-parallelism" => {
                    idx += 1;
                    let raw = value(args, idx, "--load-parallelism")?;
                    let value = raw.parse::<usize>().map_err(|err| {
                        CliError::usage(format!("parse --load-parallelism {raw}: {err}"))
                    })?;
                    if value == 0 {
                        return Err(CliError::usage("--load-parallelism must be > 0"));
                    }
                    flags.load_parallelism = Some(value);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel warm flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }
}
