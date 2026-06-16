use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::super::Subcommand;
use super::super::value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BitsArgs {
    pub vault: String,
    pub anchor_kind: String,
    pub explain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelArgs {
    pub vault: String,
    pub anchor: Option<String>,
    pub rebuild: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AbundanceArgs {
    pub vault: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProposeLensArgs {
    pub vault: String,
    pub anchor: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GuardArgs {
    pub vault: String,
    pub command: GuardCommand,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GuardCommand {
    Calibrate {
        domain: String,
        set: PathBuf,
        target_far: f32,
    },
    Check {
        cx_id: String,
        identity_cx: Option<String>,
    },
    Generate {
        candidate_text: String,
        identity_cx: Option<String>,
    },
}

pub(crate) fn parse_bits(rest: &[String]) -> CliResult<Subcommand> {
    let (vault, anchor_kind, flags) = match rest {
        [vault, anchor_kind, flags @ ..] => (vault.clone(), anchor_kind.clone(), flags),
        _ => {
            return Err(CliError::usage(
                "bits requires <vault> <anchor-kind> [--explain]",
            ));
        }
    };
    let mut explain = false;
    for flag in flags {
        match flag.as_str() {
            "--explain" => explain = true,
            other => return Err(CliError::usage(format!("unexpected bits flag {other}"))),
        }
    }
    Ok(Subcommand::Bits(BitsArgs {
        vault,
        anchor_kind,
        explain,
    }))
}

pub(crate) fn parse_kernel(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("kernel requires <vault>"))?
        .clone();
    let mut anchor = None;
    let mut rebuild = false;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--anchor" => {
                idx += 1;
                anchor = Some(value(rest, idx, "--anchor")?.to_string());
            }
            "--rebuild" => rebuild = true,
            other => return Err(CliError::usage(format!("unexpected kernel flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Kernel(KernelArgs {
        vault,
        anchor,
        rebuild,
    }))
}

pub(crate) fn parse_abundance(rest: &[String]) -> CliResult<Subcommand> {
    match rest {
        [vault] => Ok(Subcommand::Abundance(AbundanceArgs {
            vault: vault.clone(),
        })),
        _ => Err(CliError::usage("abundance requires exactly <vault>")),
    }
}

pub(crate) fn parse_propose_lens(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("propose-lens requires <vault> --anchor <kind>"))?
        .clone();
    let mut anchor = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--anchor" => {
                idx += 1;
                anchor = Some(value(rest, idx, "--anchor")?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected propose-lens flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::ProposeLens(ProposeLensArgs {
        vault,
        anchor: anchor.ok_or_else(|| CliError::usage("propose-lens requires --anchor <kind>"))?,
    }))
}

pub(crate) fn parse_guard(rest: &[String]) -> CliResult<Subcommand> {
    let (vault, subcmd, flags) = guard_layout(rest)?;
    let command = match subcmd.as_str() {
        "calibrate" => parse_guard_calibrate(flags)?,
        "check" => parse_guard_check(flags)?,
        "generate" => parse_guard_generate(flags)?,
        other => return Err(CliError::usage(format!("unknown guard subcommand {other}"))),
    };
    Ok(Subcommand::Guard(GuardArgs { vault, command }))
}

fn guard_layout(rest: &[String]) -> CliResult<(String, String, &[String])> {
    if rest.len() < 2 {
        return Err(CliError::usage(
            "guard requires <vault> <calibrate|check|generate>",
        ));
    }
    if is_guard_subcommand(&rest[0]) {
        Ok((rest[1].clone(), rest[0].clone(), &rest[2..]))
    } else {
        Ok((rest[0].clone(), rest[1].clone(), &rest[2..]))
    }
}

fn parse_guard_calibrate(flags: &[String]) -> CliResult<GuardCommand> {
    let mut domain = None;
    let mut set = None;
    let mut target_far = None;
    let mut idx = 0;
    while idx < flags.len() {
        match flags[idx].as_str() {
            "--domain" => {
                idx += 1;
                domain = Some(value(flags, idx, "--domain")?.to_string());
            }
            "--set" => {
                idx += 1;
                set = Some(PathBuf::from(value(flags, idx, "--set")?));
            }
            "--target-far" => {
                idx += 1;
                target_far = Some(parse_f32(
                    value(flags, idx, "--target-far")?,
                    "--target-far",
                )?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected guard calibrate flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(GuardCommand::Calibrate {
        domain: domain.ok_or_else(|| CliError::usage("guard calibrate requires --domain <d>"))?,
        set: set.ok_or_else(|| CliError::usage("guard calibrate requires --set <jsonl>"))?,
        target_far: target_far
            .ok_or_else(|| CliError::usage("guard calibrate requires --target-far <f32>"))?,
    })
}

fn parse_guard_check(flags: &[String]) -> CliResult<GuardCommand> {
    let mut cx_id = None;
    let mut identity_cx = None;
    let mut idx = 0;
    while idx < flags.len() {
        match flags[idx].as_str() {
            "--cx" => {
                idx += 1;
                cx_id = Some(value(flags, idx, "--cx")?.to_string());
            }
            "--identity-cx" => {
                idx += 1;
                identity_cx = Some(value(flags, idx, "--identity-cx")?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected guard check flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(GuardCommand::Check {
        cx_id: cx_id.ok_or_else(|| CliError::usage("guard check requires --cx <cx_id>"))?,
        identity_cx,
    })
}

fn parse_guard_generate(flags: &[String]) -> CliResult<GuardCommand> {
    let mut candidate_text = None;
    let mut identity_cx = None;
    let mut idx = 0;
    while idx < flags.len() {
        match flags[idx].as_str() {
            "--candidate-text" => {
                idx += 1;
                candidate_text = Some(value(flags, idx, "--candidate-text")?.to_string());
            }
            "--identity-cx" => {
                idx += 1;
                identity_cx = Some(value(flags, idx, "--identity-cx")?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected guard generate flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(GuardCommand::Generate {
        candidate_text: candidate_text
            .ok_or_else(|| CliError::usage("guard generate requires --candidate-text <s>"))?,
        identity_cx,
    })
}

fn parse_f32(raw: &str, flag: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CliError::usage(format!("{flag} must be finite")))
    }
}

fn is_guard_subcommand(value: &str) -> bool {
    matches!(value, "calibrate" | "check" | "generate")
}

#[cfg(test)]
pub(crate) fn bits_tokens(args: &BitsArgs) -> Vec<String> {
    let mut out = vec![
        "bits".to_string(),
        args.vault.clone(),
        args.anchor_kind.clone(),
    ];
    if args.explain {
        out.push("--explain".to_string());
    }
    out
}

#[cfg(test)]
pub(crate) fn kernel_tokens(args: &KernelArgs) -> Vec<String> {
    let mut out = vec!["kernel".to_string(), args.vault.clone()];
    if let Some(anchor) = &args.anchor {
        out.extend(["--anchor".to_string(), anchor.clone()]);
    }
    if args.rebuild {
        out.push("--rebuild".to_string());
    }
    out
}

#[cfg(test)]
pub(crate) fn abundance_tokens(args: &AbundanceArgs) -> Vec<String> {
    vec!["abundance".to_string(), args.vault.clone()]
}

#[cfg(test)]
pub(crate) fn propose_lens_tokens(args: &ProposeLensArgs) -> Vec<String> {
    vec![
        "propose-lens".to_string(),
        args.vault.clone(),
        "--anchor".to_string(),
        args.anchor.clone(),
    ]
}

#[cfg(test)]
pub(crate) fn guard_tokens(args: &GuardArgs) -> Vec<String> {
    let mut out = vec!["guard".to_string(), args.vault.clone()];
    match &args.command {
        GuardCommand::Calibrate {
            domain,
            set,
            target_far,
        } => {
            out.push("calibrate".to_string());
            out.extend(["--domain".to_string(), domain.clone()]);
            out.extend(["--set".to_string(), set.display().to_string()]);
            out.extend(["--target-far".to_string(), target_far.to_string()]);
        }
        GuardCommand::Check { cx_id, identity_cx } => {
            out.push("check".to_string());
            out.extend(["--cx".to_string(), cx_id.clone()]);
            if let Some(identity) = identity_cx {
                out.extend(["--identity-cx".to_string(), identity.clone()]);
            }
        }
        GuardCommand::Generate {
            candidate_text,
            identity_cx,
        } => {
            out.push("generate".to_string());
            out.extend(["--candidate-text".to_string(), candidate_text.clone()]);
            if let Some(identity) = identity_cx {
                out.extend(["--identity-cx".to_string(), identity.clone()]);
            }
        }
    }
    out
}
