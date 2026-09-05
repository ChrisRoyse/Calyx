use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::{DEFAULT_ASSOCIATION_KEY, LensTemplateRecord, direct, record_from_import_sources};

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
                    let dim = parse_u32(
                        "--tei <name> <endpoint> <dim> <physical-model-artifact>",
                        &next()?,
                    )?;
                    let weights_path = PathBuf::from(next()?);
                    direct_lenses.push(direct::DirectLensSource::Tei {
                        name,
                        endpoint,
                        dim,
                        weights_path,
                    });
                }
                "--algorithmic" => {
                    let name = next()?;
                    let kind = next()?;
                    let dim = parse_u32("--algorithmic <name> <kind> <dim>", &next()?)?;
                    direct_lenses.push(direct::DirectLensSource::Algorithmic { name, kind, dim });
                }
                "--structured" => {
                    let name = next()?;
                    let binding_path = PathBuf::from(next()?);
                    direct_lenses.push(direct::DirectLensSource::Structured { name, binding_path });
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
        record_from_import_sources(&self.manifests, &self.direct_lenses)
    }
}

fn parse_u32(flag: &str, value: &str) -> CliResult<u32> {
    value
        .parse::<u32>()
        .map_err(|err| CliError::usage(format!("parse {flag} dimension: {err}")))
}
