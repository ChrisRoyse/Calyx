use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Modality;
use serde::Serialize;

use super::template_store::{
    MIN_CONTENT_LENSES, TEMPLATE_INVALID, TemplateDraft, TemplateSave, TemplateStore,
    ensemble_card_from_capability_cards, lens_ref_from_catalog, template_error,
};
use super::{LensCatalogEntry, catalog_path, read_catalog};
use crate::cmd::vault;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Default)]
struct Flags {
    home: Option<PathBuf>,
    name: Option<String>,
    notes: Option<String>,
    from: Option<String>,
    template: Option<String>,
    vault: Option<String>,
    all_current: bool,
    modality: Option<Modality>,
    lenses: Vec<String>,
    cards: Vec<PathBuf>,
    card_dir: Option<PathBuf>,
    assay_card: Option<PathBuf>,
    a37_admission_card: Option<PathBuf>,
    require_a37_gate: bool,
}

#[derive(Serialize)]
struct SaveReport {
    action: &'static str,
    template_id: String,
    object_path: PathBuf,
    index_path: PathBuf,
    name: String,
    version: u32,
    content_lens_count: usize,
    time_control_count: usize,
    has_ensemble_card: bool,
    a37_gate_eligible: bool,
    a37_status: String,
}

#[derive(Serialize)]
struct ListReport {
    index_path: PathBuf,
    count: usize,
    templates: Vec<super::template_store::TemplateSummary>,
}

#[derive(Serialize)]
struct SeedReport {
    index_path: PathBuf,
    templates: Vec<SaveReport>,
}

pub(super) fn run(rest: &[String]) -> CliResult {
    let (command, args) = rest
        .split_first()
        .ok_or_else(|| CliError::usage("calyx panel template requires a subcommand"))?;
    match command.as_str() {
        "seed" => seed(args),
        "save" => save(args),
        "list" => list(args),
        "fork" => fork(args),
        "profile" => profile(args),
        "swap" => swap(args),
        other => Err(CliError::usage(format!(
            "unknown panel template subcommand {other}; expected seed, save, list, fork, profile, or swap"
        ))),
    }
}

fn seed(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home.clone())?;
    let catalog = read_catalog(&catalog_path(Some(&home))?)?;
    let all = catalog.lenses;
    let text = select_by_modality(&all, Some(Modality::Text));
    let store = TemplateStore::open(&home);
    let reports = vec![
        save_seed(
            &store,
            "text-deep",
            "ten real text/content lenses for deep semantic panels",
            text.clone(),
        )?,
        save_seed(
            &store,
            "literary-essence",
            "text/style/persona panel; temporal controls remain non-counting sidecars",
            text.clone(),
        )?,
        save_seed(
            &store,
            "code-oracle",
            "code-oriented template seeded from available real text encoders until code lenses are commissioned",
            text,
        )?,
        save_seed(
            &store,
            "video-capture",
            "available text, image, and audio lenses for video capture; temporal controls are time manipulation sidecars",
            all,
        )?,
    ];
    print_json(&SeedReport {
        index_path: home.join("panels").join("templates").join("index.json"),
        templates: reports,
    })
}

fn save(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home.clone())?;
    let lenses = select_lenses(&home, &flags)?;
    let name = flags
        .name
        .ok_or_else(|| CliError::usage("panel template save requires --name <name>"))?;
    let store = TemplateStore::open(&home);
    let save = store.save(
        TemplateDraft {
            name,
            notes: flags.notes.unwrap_or_default(),
            lenses,
            ensemble_card: None,
        },
        vault::now_ms(),
    )?;
    print_json(&save_report("save", save))
}

fn list(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home)?;
    let store = TemplateStore::open(&home);
    let templates = store.list()?;
    print_json(&ListReport {
        index_path: home.join("panels").join("templates").join("index.json"),
        count: templates.len(),
        templates,
    })
}

fn fork(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home)?;
    let from = flags
        .from
        .ok_or_else(|| CliError::usage("panel template fork requires --from <name-or-id>"))?;
    let name = flags
        .name
        .ok_or_else(|| CliError::usage("panel template fork requires --name <name>"))?;
    let store = TemplateStore::open(&home);
    let save = store.fork(&from, name, flags.notes, vault::now_ms())?;
    print_json(&save_report("fork", save))
}

fn profile(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home.clone())?;
    let cards = card_paths(&flags)?;
    let selector = flags
        .template
        .as_deref()
        .ok_or_else(|| CliError::usage("panel template profile requires --template <name-or-id>"))?
        .to_string();
    let store = TemplateStore::open(&home);
    let template = store.load(&selector)?;
    let card = ensemble_card_from_capability_cards(
        &template,
        &cards,
        flags.assay_card.as_deref(),
        flags.a37_admission_card.as_deref(),
    )?;
    let save = store.profile(&selector, card, vault::now_ms())?;
    print_json(&save_report("profile", save))
}

fn swap(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home)?;
    let selector = flags
        .template
        .ok_or_else(|| CliError::usage("panel template swap requires --template <name-or-id>"))?;
    let vault_name = flags
        .vault
        .ok_or_else(|| CliError::usage("panel template swap requires --vault <vault>"))?;
    let vault_dir = vault::resolve_vault(&home, &vault_name)?;
    let store = TemplateStore::open(&home);
    let report = store.swap_into_vault(
        &selector,
        &vault_dir,
        vault::now_ms(),
        flags.require_a37_gate,
    )?;
    print_json(&report)
}

fn save_seed(
    store: &TemplateStore,
    name: &str,
    notes: &str,
    entries: Vec<LensCatalogEntry>,
) -> CliResult<SaveReport> {
    let lenses = entries
        .iter()
        .map(lens_ref_from_catalog)
        .collect::<CliResult<Vec<_>>>()?;
    let save = store.save(
        TemplateDraft {
            name: name.to_string(),
            notes: notes.to_string(),
            lenses,
            ensemble_card: None,
        },
        vault::now_ms(),
    )?;
    Ok(save_report("seed", save))
}

fn select_lenses(
    home: &Path,
    flags: &Flags,
) -> CliResult<Vec<super::template_store::TemplateLensRef>> {
    let catalog = read_catalog(&catalog_path(Some(home))?)?;
    let entries = if flags.all_current {
        select_by_modality(&catalog.lenses, flags.modality)
    } else {
        select_named(&catalog.lenses, &flags.lenses)?
    };
    if entries.len() < MIN_CONTENT_LENSES {
        return Err(template_error(
            TEMPLATE_INVALID,
            format!(
                "panel template selected {} content lenses; minimum is {MIN_CONTENT_LENSES}",
                entries.len()
            ),
            "add real frozen content lenses until the template has at least ten",
        ));
    }
    entries
        .iter()
        .map(lens_ref_from_catalog)
        .collect::<CliResult<Vec<_>>>()
}

fn select_named(
    catalog: &[LensCatalogEntry],
    names: &[String],
) -> CliResult<Vec<LensCatalogEntry>> {
    if names.is_empty() {
        return Err(CliError::usage(
            "panel template save requires --all-current or at least one --lens <name-or-id>",
        ));
    }
    let mut selected = Vec::new();
    for name in names {
        let entry = catalog
            .iter()
            .find(|entry| entry.name == *name || entry.lens_id == *name)
            .ok_or_else(|| CliError::usage(format!("lens {name} not found in catalog")))?;
        if !selected
            .iter()
            .any(|item: &LensCatalogEntry| item.lens_id == entry.lens_id)
        {
            selected.push(entry.clone());
        }
    }
    Ok(selected)
}

fn select_by_modality(
    catalog: &[LensCatalogEntry],
    modality: Option<Modality>,
) -> Vec<LensCatalogEntry> {
    catalog
        .iter()
        .filter(|entry| {
            modality
                .map(|value| entry.modality == modality_name(value))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn card_paths(flags: &Flags) -> CliResult<Vec<PathBuf>> {
    let mut paths = flags.cards.clone();
    if let Some(dir) = &flags.card_dir {
        let mut from_dir = fs::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        from_dir.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
        from_dir.sort();
        paths.extend(from_dir);
    }
    if paths.is_empty() {
        return Err(CliError::usage(
            "panel template profile requires --card <json> or --card-dir <dir>",
        ));
    }
    Ok(paths)
}

impl Flags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--name" => {
                    idx += 1;
                    flags.name = Some(value(args, idx, "--name")?.to_string());
                }
                "--notes" => {
                    idx += 1;
                    flags.notes = Some(value(args, idx, "--notes")?.to_string());
                }
                "--from" => {
                    idx += 1;
                    flags.from = Some(value(args, idx, "--from")?.to_string());
                }
                "--template" => {
                    idx += 1;
                    flags.template = Some(value(args, idx, "--template")?.to_string());
                }
                "--vault" => {
                    idx += 1;
                    flags.vault = Some(value(args, idx, "--vault")?.to_string());
                }
                "--all-current" => flags.all_current = true,
                "--modality" => {
                    idx += 1;
                    flags.modality = Some(parse_modality(value(args, idx, "--modality")?)?);
                }
                "--lens" => {
                    idx += 1;
                    flags.lenses.push(value(args, idx, "--lens")?.to_string());
                }
                "--card" => {
                    idx += 1;
                    flags.cards.push(value(args, idx, "--card")?.into());
                }
                "--card-dir" => {
                    idx += 1;
                    flags.card_dir = Some(value(args, idx, "--card-dir")?.into());
                }
                "--assay-card" => {
                    idx += 1;
                    flags.assay_card = Some(value(args, idx, "--assay-card")?.into());
                }
                "--a37-admission-card" => {
                    idx += 1;
                    flags.a37_admission_card =
                        Some(value(args, idx, "--a37-admission-card")?.into());
                }
                "--require-a37-gate" => flags.require_a37_gate = true,
                other => return Err(CliError::usage(format!("unexpected template flag {other}"))),
            }
            idx += 1;
        }
        Ok(flags)
    }
}

fn save_report(action: &'static str, save: TemplateSave) -> SaveReport {
    let content_lens_count = save.template.content_lens_count();
    let time_control_count = save.template.time_controls.len();
    let has_ensemble_card = save.template.ensemble_card.is_some();
    let a37 = save.template.a37_admission();
    SaveReport {
        action,
        template_id: save.template_id,
        object_path: save.object_path,
        index_path: save.index_path,
        name: save.template.name,
        version: save.template.version,
        content_lens_count,
        time_control_count,
        has_ensemble_card,
        a37_gate_eligible: a37.gate_eligible,
        a37_status: a37.status,
    }
}

fn home(value: Option<PathBuf>) -> CliResult<PathBuf> {
    match value {
        Some(path) => Ok(path),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>")),
    }
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_modality(value: &str) -> CliResult<Modality> {
    match value {
        "text" => Ok(Modality::Text),
        "code" => Ok(Modality::Code),
        "image" => Ok(Modality::Image),
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        "protein" => Ok(Modality::Protein),
        "dna" => Ok(Modality::Dna),
        "molecule" => Ok(Modality::Molecule),
        "structured" => Ok(Modality::Structured),
        "mixed" => Ok(Modality::Mixed),
        other => Err(CliError::usage(format!("unknown modality {other}"))),
    }
}

fn modality_name(value: Modality) -> &'static str {
    match value {
        Modality::Text => "text",
        Modality::Code => "code",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Protein => "protein",
        Modality::Dna => "dna",
        Modality::Molecule => "molecule",
        Modality::Structured => "structured",
        Modality::Mixed => "mixed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_lenses() {
        let flags = Flags::parse(&[
            "--name".to_string(),
            "text-deep".to_string(),
            "--lens".to_string(),
            "a".to_string(),
            "--lens".to_string(),
            "b".to_string(),
        ])
        .unwrap();

        assert_eq!(flags.name.as_deref(), Some("text-deep"));
        assert_eq!(flags.lenses, ["a", "b"]);
    }

    #[test]
    fn parses_a37_template_flags() {
        let flags = Flags::parse(&[
            "--template".to_string(),
            "text-deep".to_string(),
            "--assay-card".to_string(),
            "ensemble_card.json".to_string(),
            "--require-a37-gate".to_string(),
            "--a37-admission-card".to_string(),
            "multi_anchor_card.json".to_string(),
        ])
        .unwrap();

        assert_eq!(flags.template.as_deref(), Some("text-deep"));
        assert_eq!(
            flags.assay_card.as_deref(),
            Some(Path::new("ensemble_card.json"))
        );
        assert_eq!(
            flags.a37_admission_card.as_deref(),
            Some(Path::new("multi_anchor_card.json"))
        );
        assert!(flags.require_a37_gate);
    }

    #[test]
    fn modality_parser_matches_catalog_strings() {
        assert_eq!(modality_name(parse_modality("text").unwrap()), "text");
        assert!(parse_modality("temporal").is_err());
    }
}
