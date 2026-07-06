use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::{DEFAULT_ASSOCIATION_KEY, DEFAULT_CHUNK_ROWS};

pub(super) struct ImportArgs {
    pub(super) rows_jsonl: PathBuf,
    pub(super) cf_root: PathBuf,
    pub(super) association_key: String,
    pub(super) target_class: usize,
    pub(super) anchor_name: Option<String>,
    pub(super) derive_anchor: String,
    pub(super) limit_per_class: Option<usize>,
    pub(super) chunk_rows: usize,
}

impl ImportArgs {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut rows_jsonl = None;
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut target_class = 1_usize;
        let mut anchor_name = None;
        let mut derive_anchor = "label".to_string();
        let mut limit_per_class = None;
        let mut chunk_rows = DEFAULT_CHUNK_ROWS;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--rows-jsonl" => rows_jsonl = Some(PathBuf::from(next()?)),
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--labels-key" | "--label-key" => association_key = next()?,
                "--target-class" => {
                    target_class = next()?.parse::<usize>().map_err(|error| {
                        CliError::usage(format!("invalid --target-class: {error}"))
                    })?;
                }
                "--anchor-name" => anchor_name = Some(next()?),
                "--derive-anchor" => derive_anchor = next()?,
                "--limit-per-class" => {
                    limit_per_class = Some(next()?.parse::<usize>().map_err(|error| {
                        CliError::usage(format!("invalid --limit-per-class: {error}"))
                    })?);
                }
                "--chunk-rows" => {
                    chunk_rows = next()?.parse::<usize>().map_err(|error| {
                        CliError::usage(format!("invalid --chunk-rows: {error}"))
                    })?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if association_key.trim().is_empty() {
            return Err(CliError::usage("--labels-key must be non-empty"));
        }
        if chunk_rows == 0 {
            return Err(CliError::usage("--chunk-rows must be > 0"));
        }
        if matches!(limit_per_class, Some(0)) {
            return Err(CliError::usage("--limit-per-class must be > 0"));
        }
        Ok(Self {
            rows_jsonl: rows_jsonl
                .ok_or_else(|| CliError::usage("--rows-jsonl <rows.jsonl> is required"))?,
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <dir> is required"))?,
            association_key,
            target_class,
            anchor_name,
            derive_anchor,
            limit_per_class,
            chunk_rows,
        })
    }
}
