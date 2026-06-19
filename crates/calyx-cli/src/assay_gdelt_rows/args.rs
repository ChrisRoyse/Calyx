use std::path::PathBuf;

use crate::error::{CliError, CliResult};

#[derive(Clone, Debug)]
pub(super) struct Args {
    pub(super) source_dir: PathBuf,
    pub(super) out: PathBuf,
    pub(super) manifest: PathBuf,
    pub(super) dataset: String,
    pub(super) target_class: usize,
    pub(super) limit_per_class: Option<usize>,
    pub(super) max_rows: Option<usize>,
    pub(super) actor_country: String,
    pub(super) action_country: String,
    pub(super) action_name_contains: String,
}

impl Args {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut source_dir = None;
        let mut out = None;
        let mut manifest = None;
        let mut dataset = "gdelt-v2-events".to_string();
        let mut target_class = 1usize;
        let mut limit_per_class = None;
        let mut max_rows = None;
        let mut actor_country = "USA".to_string();
        let mut action_country = "US".to_string();
        let mut action_name_contains = "United States".to_string();
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--source-dir" => source_dir = Some(PathBuf::from(next()?)),
                "--out" => out = Some(PathBuf::from(next()?)),
                "--manifest" => manifest = Some(PathBuf::from(next()?)),
                "--dataset" => dataset = next()?,
                "--target-class" => target_class = parse_usize(&next()?, flag)?,
                "--limit-per-class" => limit_per_class = Some(parse_usize(&next()?, flag)?),
                "--max-rows" => max_rows = Some(parse_usize(&next()?, flag)?),
                "--actor-country" => actor_country = next()?,
                "--action-country" => action_country = next()?,
                "--action-name-contains" => action_name_contains = next()?,
                other => {
                    return Err(CliError::usage(format!(
                        "unknown assay gdelt-rows arg: {other}"
                    )));
                }
            }
        }
        let args = Self {
            source_dir: source_dir
                .ok_or_else(|| CliError::usage("--source-dir <dir> is required"))?,
            out: out.ok_or_else(|| CliError::usage("--out <rows.jsonl> is required"))?,
            manifest: manifest
                .ok_or_else(|| CliError::usage("--manifest <manifest.json> is required"))?,
            dataset,
            target_class,
            limit_per_class,
            max_rows,
            actor_country: normalize_code(&actor_country),
            action_country: normalize_code(&action_country),
            action_name_contains,
        };
        args.validate()?;
        Ok(args)
    }

    fn validate(&self) -> CliResult {
        if self.dataset.trim().is_empty() {
            return Err(CliError::usage("--dataset must be non-empty"));
        }
        if self.target_class != 1 {
            return Err(CliError::usage(
                "--target-class must be 1 for the current binary GDELT label rule",
            ));
        }
        if matches!(self.limit_per_class, Some(0)) || matches!(self.max_rows, Some(0)) {
            return Err(CliError::usage(
                "--limit-per-class and --max-rows must be > 0 when provided",
            ));
        }
        if self.actor_country.is_empty() || self.action_country.is_empty() {
            return Err(CliError::usage(
                "--actor-country and --action-country must be non-empty",
            ));
        }
        if self.action_name_contains.trim().is_empty() {
            return Err(CliError::usage("--action-name-contains must be non-empty"));
        }
        Ok(())
    }
}

fn normalize_code(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn parse_usize(value: &str, flag: &str) -> CliResult<usize> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("{flag} expects usize: {error}")))
}
