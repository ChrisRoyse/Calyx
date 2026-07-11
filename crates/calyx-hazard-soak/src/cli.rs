use super::Suite;
use calyx_hazard_soak::soak::{DEFAULT_SOAK_OPS, DEFAULT_SOAK_SEED};
use serde::Serialize;
use std::env;
use std::process::{Command, Output};

#[derive(Clone)]
pub(crate) struct RunConfig {
    pub(crate) suite: Suite,
    pub(crate) seed_input: String,
    pub(crate) seed: u64,
    pub(crate) soak_ops: u64,
}

impl RunConfig {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut suite = None;
        let mut seed_input = "0xCALYX59".to_string();
        let mut soak_ops = env_u64("PH59_FINAL_SOAK_OPS").unwrap_or(DEFAULT_SOAK_OPS);
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--all-hazards" => {
                    suite = Some(Suite::Stage13Exit);
                    idx += 1;
                }
                "--hazards" => {
                    let range = args
                        .get(idx + 1)
                        .ok_or_else(|| "--hazards requires a range".to_string())?;
                    suite = Some(Suite::from_hazards_range(range)?);
                    idx += 2;
                }
                "--seed" => {
                    seed_input = args
                        .get(idx + 1)
                        .ok_or_else(|| "--seed requires a value".to_string())?
                        .clone();
                    idx += 2;
                }
                "--ops" | "--soak-ops" => {
                    soak_ops = args
                        .get(idx + 1)
                        .ok_or_else(|| "--ops requires a value".to_string())?
                        .parse::<u64>()
                        .map_err(|error| format!("parse soak ops: {error}"))?;
                    idx += 2;
                }
                value => return Err(format!("unsupported arg {value:?}")),
            }
        }
        let seed = parse_seed(&seed_input);
        Ok(Self {
            suite: suite.unwrap_or(Suite::Hazards1To5),
            seed_input,
            seed,
            soak_ops,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OomEvidence {
    pub(crate) source: String,
    pub(crate) count: u64,
}

pub(crate) fn dmesg_oom_evidence() -> Result<OomEvidence, String> {
    let direct = Command::new("dmesg").output();
    if let Ok(output) = &direct
        && output.status.success()
    {
        return Ok(oom_evidence("dmesg", output));
    }
    let elevated = Command::new("sudo").args(["-n", "dmesg"]).output();
    if let Ok(output) = &elevated
        && output.status.success()
    {
        return Ok(oom_evidence("sudo -n dmesg", output));
    }
    let journal = Command::new("journalctl")
        .args(["-k", "--no-pager", "-q"])
        .output();
    if let Ok(output) = &journal
        && output.status.success()
    {
        return Ok(oom_evidence("journalctl -k --no-pager -q", output));
    }
    Err(format!(
        "kernel OOM evidence unavailable: dmesg={}; sudo -n dmesg={}; journalctl={}",
        command_failure(&direct),
        command_failure(&elevated),
        command_failure(&journal)
    ))
}

fn oom_evidence(source: &str, output: &Output) -> OomEvidence {
    let count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains("oom"))
        .count() as u64;
    OomEvidence {
        source: source.to_string(),
        count,
    }
}

fn command_failure(output: &std::io::Result<Output>) -> String {
    match output {
        Ok(output) => format!(
            "status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => error.to_string(),
    }
}

fn parse_seed(input: &str) -> u64 {
    if input.eq_ignore_ascii_case("0xCALYX59") {
        return DEFAULT_SOAK_SEED;
    }
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        && let Ok(value) = u64::from_str_radix(hex, 16)
    {
        return value;
    }
    input.parse().unwrap_or_else(|_| {
        let hash = blake3::hash(input.as_bytes());
        u64::from_be_bytes(hash.as_bytes()[..8].try_into().expect("hash prefix"))
    })
}

fn env_u64(name: &str) -> Option<u64> {
    env::var(name).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oom_evidence_counts_matching_kernel_log_lines() {
        let output = if cfg!(windows) {
            Command::new("cmd")
                .args(["/c", "echo Out of memory: OOM kill&&echo healthy"])
                .output()
                .unwrap()
        } else {
            Command::new("sh")
                .args(["-c", "printf 'Out of memory: OOM kill\\nhealthy\\n'"])
                .output()
                .unwrap()
        };

        let evidence = oom_evidence("test", &output);

        assert_eq!(evidence.count, 1);
        assert_eq!(evidence.source, "test");
    }
}
