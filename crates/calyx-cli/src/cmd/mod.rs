mod lens;
pub(crate) mod vault;

use std::path::PathBuf;

use crate::error::{CliError, CliResult};

pub(crate) const PANEL_TEMPLATES: &[&str] = &[
    "text-default",
    "code-default",
    "civic-default",
    "legal-default",
    "medical-default",
    "bio-default",
    "media-default",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Subcommand {
    CreateVault(CreateVaultArgs),
    AddLens(AddLensArgs),
    RetireLens(SlotCommandArgs),
    ParkLens(SlotCommandArgs),
    ListPanel(VaultRefArgs),
    ProfileLens(ProfileLensArgs),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateVaultArgs {
    pub name: String,
    pub panel_template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AddLensArgs {
    pub vault: String,
    pub name: String,
    pub runtime: String,
    pub endpoint: Option<String>,
    pub weights: Option<PathBuf>,
    pub shape: Option<String>,
    pub modality: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SlotCommandArgs {
    pub vault: String,
    pub slot: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultRefArgs {
    pub vault: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileLensArgs {
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub endpoint: Option<String>,
    pub weights: Option<PathBuf>,
    pub shape: Option<String>,
    pub modality: Option<String>,
    pub probe: Option<PathBuf>,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    if !args.first().is_some_and(|command| is_cmd(command)) {
        return None;
    }
    Some(parse(args).and_then(vault::run))
}

pub(crate) fn parse(args: &[String]) -> CliResult<Subcommand> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| CliError::usage("missing command"))?;
    match command.as_str() {
        "create-vault" => parse_create_vault(rest),
        "add-lens" => parse_add_lens(rest),
        "retire-lens" => parse_slot_command(rest).map(Subcommand::RetireLens),
        "park-lens" => parse_slot_command(rest).map(Subcommand::ParkLens),
        "list-panel" => parse_vault_ref(rest).map(Subcommand::ListPanel),
        "profile-lens" => parse_profile_lens(rest),
        other => Err(CliError::usage(format!("unknown PH62 command {other}"))),
    }
}

fn is_cmd(command: &str) -> bool {
    matches!(
        command,
        "create-vault" | "add-lens" | "retire-lens" | "park-lens" | "list-panel" | "profile-lens"
    )
}

pub(crate) fn validate_vault_name(name: &str) -> CliResult {
    if name.is_empty() {
        return Err(CliError::usage("vault name must not be empty"));
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(CliError::usage(
            "vault name must be a name, not a filesystem path",
        ));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(CliError::usage("vault name must not contain spaces"));
    }
    Ok(())
}

pub(crate) fn validate_panel_template_name(value: &str) -> CliResult {
    if PANEL_TEMPLATES.contains(&value) {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "unknown --panel-template {value}; expected one of {}",
            PANEL_TEMPLATES.join(", ")
        )))
    }
}

fn parse_create_vault(rest: &[String]) -> CliResult<Subcommand> {
    let name = rest
        .first()
        .ok_or_else(|| CliError::usage("create-vault requires <name>"))?
        .clone();
    validate_vault_name(&name)?;
    let mut panel_template = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--panel-template" => {
                idx += 1;
                let value = value(rest, idx, "--panel-template")?;
                validate_panel_template_name(value)?;
                panel_template = Some(value.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected create-vault flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::CreateVault(CreateVaultArgs {
        name,
        panel_template,
    }))
}

fn parse_add_lens(rest: &[String]) -> CliResult<Subcommand> {
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

fn parse_slot_command(rest: &[String]) -> CliResult<SlotCommandArgs> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("lens lifecycle command requires <vault>"))?
        .clone();
    let mut slot = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--slot" => {
                idx += 1;
                let raw = value(rest, idx, "--slot")?;
                slot = Some(
                    raw.parse::<u16>()
                        .map_err(|err| CliError::usage(format!("parse --slot {raw}: {err}")))?,
                );
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected lifecycle flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(SlotCommandArgs {
        vault,
        slot: slot
            .ok_or_else(|| CliError::usage("lens lifecycle command requires --slot <u16>"))?,
    })
}

fn parse_vault_ref(rest: &[String]) -> CliResult<VaultRefArgs> {
    match rest {
        [vault] => Ok(VaultRefArgs {
            vault: vault.clone(),
        }),
        _ => Err(CliError::usage("list-panel requires exactly <vault>")),
    }
}

fn parse_profile_lens(rest: &[String]) -> CliResult<Subcommand> {
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

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

#[cfg(test)]
impl Subcommand {
    fn to_cli_tokens(&self) -> Vec<String> {
        match self {
            Self::CreateVault(args) => {
                let mut out = vec!["create-vault".to_string(), args.name.clone()];
                if let Some(template) = &args.panel_template {
                    out.extend(["--panel-template".to_string(), template.clone()]);
                }
                out
            }
            Self::AddLens(args) => {
                let mut out = vec![
                    "add-lens".to_string(),
                    args.vault.clone(),
                    "--name".to_string(),
                    args.name.clone(),
                    "--runtime".to_string(),
                    args.runtime.clone(),
                ];
                push_opt(&mut out, "--endpoint", args.endpoint.as_deref());
                push_opt(
                    &mut out,
                    "--weights",
                    args.weights.as_ref().and_then(|p| p.to_str()),
                );
                push_opt(&mut out, "--shape", args.shape.as_deref());
                push_opt(&mut out, "--modality", args.modality.as_deref());
                out
            }
            Self::RetireLens(args) => slot_tokens("retire-lens", args),
            Self::ParkLens(args) => slot_tokens("park-lens", args),
            Self::ListPanel(args) => vec!["list-panel".to_string(), args.vault.clone()],
            Self::ProfileLens(args) => {
                let mut out = vec!["profile-lens".to_string()];
                push_opt(&mut out, "--name", args.name.as_deref());
                push_opt(&mut out, "--runtime", args.runtime.as_deref());
                push_opt(&mut out, "--endpoint", args.endpoint.as_deref());
                push_opt(
                    &mut out,
                    "--weights",
                    args.weights.as_ref().and_then(|p| p.to_str()),
                );
                push_opt(&mut out, "--shape", args.shape.as_deref());
                push_opt(&mut out, "--modality", args.modality.as_deref());
                push_opt(
                    &mut out,
                    "--probe",
                    args.probe.as_ref().and_then(|p| p.to_str()),
                );
                out
            }
        }
    }
}

#[cfg(test)]
fn push_opt(out: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        out.extend([flag.to_string(), value.to_string()]);
    }
}

#[cfg(test)]
fn slot_tokens(command: &str, args: &SlotCommandArgs) -> Vec<String> {
    vec![
        command.to_string(),
        args.vault.clone(),
        "--slot".to_string(),
        args.slot.to_string(),
    ]
}

#[cfg(test)]
mod tests;
