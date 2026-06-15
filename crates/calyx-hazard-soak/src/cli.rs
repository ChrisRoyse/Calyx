use super::Suite;
use calyx_hazard_soak::soak::{DEFAULT_SOAK_OPS, DEFAULT_SOAK_SEED};
use std::env;
use std::process::Command;

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

pub(crate) fn dmesg_oom_count() -> Option<u64> {
    let output = Command::new("sh")
        .args(["-lc", "dmesg 2>/dev/null | grep -ci oom || true"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
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
