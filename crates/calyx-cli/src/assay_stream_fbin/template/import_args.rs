use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::{DEFAULT_ASSOCIATION_KEY, LensTemplateRecord, direct, record_from_manifests};

pub(super) struct ImportArgs {
    pub(super) cf_root: PathBuf,
    pub(super) association_key: String,
    manifests: Vec<PathBuf>,
    direct_lenses: Vec<direct::DirectLensSource>,
}

impl ImportArgs {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut manifests = Vec::new();
        let mut direct_lenses = Vec::new();
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--manifest" => manifests.push(PathBuf::from(next()?)),
                "--tei" => {
                    let name = next()?;
                    let endpoint = next()?;
                    let dim = parse_u32("--tei <name> <endpoint> <dim>", &next()?)?;
                    direct_lenses.push(direct::DirectLensSource::Tei {
                        name,
                        endpoint,
                        dim,
                    });
                }
                "--algorithmic" => {
                    let name = next()?;
                    let kind = next()?;
                    let dim = parse_u32("--algorithmic <name> <kind> <dim>", &next()?)?;
                    direct_lenses.push(direct::DirectLensSource::Algorithmic { name, kind, dim });
                }
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--lens-template-key" => association_key = next()?,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if association_key.trim().is_empty() {
            return Err(CliError::usage("--lens-template-key must be non-empty"));
        }
        Ok(Self {
            manifests,
            direct_lenses,
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <dir> is required"))?,
            association_key,
        })
    }

    pub(super) fn record(&self) -> CliResult<LensTemplateRecord> {
        if !self.manifests.is_empty() && !self.direct_lenses.is_empty() {
            return Err(CliError::usage(
                "lens template import cannot mix --manifest with direct --tei/--algorithmic lenses",
            ));
        }
        if !self.direct_lenses.is_empty() {
            return direct::record_from_direct_lenses(&self.direct_lenses);
        }
        record_from_manifests(&self.manifests)
    }
}

fn parse_u32(flag: &str, value: &str) -> CliResult<u32> {
    value
        .parse::<u32>()
        .map_err(|err| CliError::usage(format!("parse {flag} dimension: {err}")))
}
