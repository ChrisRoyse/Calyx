mod healthcheck;
mod ingest;
mod intelligence;
mod lens;
mod provenance;
mod readback;
mod search;
pub(crate) mod vault;

pub(crate) use ingest::{
    measure_constellation as measure_ingest_constellation, text_input as ingest_text_input,
};
pub(crate) use search::{
    PersistedSearchIndexes, load_docs as load_search_docs, measure_text_query_vectors,
    rebuild_persistent_indexes,
};

use std::path::PathBuf;

use calyx_core::Modality;

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

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Subcommand {
    CreateVault(CreateVaultArgs),
    AddLens(AddLensArgs),
    RetireLens(SlotCommandArgs),
    ParkLens(SlotCommandArgs),
    ListPanel(VaultRefArgs),
    ProfileLens(ProfileLensArgs),
    Ingest(IngestArgs),
    Anchor(AnchorArgs),
    Measure(MeasureArgs),
    Search(search::SearchArgs),
    KernelAnswer(search::KernelAnswerArgs),
    Bits(intelligence::BitsArgs),
    Kernel(intelligence::KernelArgs),
    Guard(intelligence::GuardArgs),
    Abundance(intelligence::AbundanceArgs),
    ProposeLens(intelligence::ProposeLensArgs),
    Provenance(provenance::ProvenanceArgs),
    VerifyChain(provenance::VerifyChainArgs),
    Reproduce(provenance::ReproduceArgs),
    AnnealStatus(provenance::AnnealStatusArgs),
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
pub(crate) struct IngestArgs {
    pub vault: String,
    pub text: Option<String>,
    pub batch: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub modality: Option<Modality>,
    pub idempotent: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnchorArgs {
    pub vault: String,
    pub cx_id: String,
    pub kind: String,
    pub value: String,
    pub confidence: Option<f32>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MeasureArgs {
    pub vault: String,
    pub text: String,
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
    if let Some(result) = readback::try_run(args) {
        return Some(result);
    }
    if let Some(result) = healthcheck::try_run(args) {
        return Some(result);
    }
    if !args.first().is_some_and(|command| is_cmd(command)) {
        return None;
    }
    if args
        .first()
        .is_some_and(|command| command == "verify-chain")
        && args.get(1).is_some_and(|arg| arg.starts_with("--"))
    {
        return None;
    }
    Some(parse(args).and_then(run))
}

fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::CreateVault(_)
        | Subcommand::AddLens(_)
        | Subcommand::RetireLens(_)
        | Subcommand::ParkLens(_)
        | Subcommand::ListPanel(_)
        | Subcommand::ProfileLens(_) => vault::run(command),
        Subcommand::Ingest(_) | Subcommand::Anchor(_) | Subcommand::Measure(_) => {
            ingest::run(command)
        }
        Subcommand::Search(_) | Subcommand::KernelAnswer(_) => search::run(command),
        Subcommand::Bits(_)
        | Subcommand::Kernel(_)
        | Subcommand::Guard(_)
        | Subcommand::Abundance(_)
        | Subcommand::ProposeLens(_) => intelligence::run(command),
        Subcommand::Provenance(_)
        | Subcommand::VerifyChain(_)
        | Subcommand::Reproduce(_)
        | Subcommand::AnnealStatus(_) => provenance::run(command),
    }
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
        "ingest" => ingest::parse_ingest(rest),
        "anchor" => ingest::parse_anchor(rest),
        "measure" => ingest::parse_measure(rest),
        "search" => search::parse_search(rest),
        "kernel-answer" => search::parse_kernel_answer(rest),
        "bits" => intelligence::parse_bits(rest),
        "kernel" => intelligence::parse_kernel(rest),
        "guard" => intelligence::parse_guard(rest),
        "abundance" => intelligence::parse_abundance(rest),
        "propose-lens" => intelligence::parse_propose_lens(rest),
        "provenance" => provenance::parse_provenance(rest),
        "verify-chain" => provenance::parse_verify_chain(rest),
        "reproduce" => provenance::parse_reproduce(rest),
        "anneal-status" => provenance::parse_anneal_status(rest),
        other => Err(CliError::usage(format!("unknown PH62 command {other}"))),
    }
}

fn is_cmd(command: &str) -> bool {
    matches!(
        command,
        "create-vault"
            | "add-lens"
            | "retire-lens"
            | "park-lens"
            | "list-panel"
            | "profile-lens"
            | "ingest"
            | "anchor"
            | "measure"
            | "search"
            | "kernel-answer"
            | "bits"
            | "kernel"
            | "guard"
            | "abundance"
            | "propose-lens"
            | "provenance"
            | "verify-chain"
            | "reproduce"
            | "anneal-status"
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

pub(super) fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

#[cfg(test)]
mod tests;
