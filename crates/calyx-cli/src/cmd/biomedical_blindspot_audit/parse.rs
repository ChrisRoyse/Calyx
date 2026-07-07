use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::super::{Subcommand, value};
use super::model::BiomedicalBlindspotAuditArgs;
use super::util::{parse_u64, parse_unit, require, require_path};

pub(crate) fn parse_biomedical_blindspot_audit(rest: &[String]) -> CliResult<Subcommand> {
    let mut args = BiomedicalBlindspotAuditArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--hypotheses-report" => {
                idx += 1;
                args.hypotheses_reports.push(PathBuf::from(value(
                    rest,
                    idx,
                    "--hypotheses-report",
                )?));
            }
            "--literature-audit" => {
                idx += 1;
                args.literature_audit = value(rest, idx, "--literature-audit")?.into();
            }
            "--stability-audit" => {
                idx += 1;
                args.stability_audit = value(rest, idx, "--stability-audit")?.into();
            }
            "--drug-lifecycle" => {
                idx += 1;
                args.drug_lifecycle = value(rest, idx, "--drug-lifecycle")?.into();
            }
            "--transcriptomic-audit" => {
                idx += 1;
                args.transcriptomic_audit = value(rest, idx, "--transcriptomic-audit")?.into();
            }
            "--out-dir" => {
                idx += 1;
                args.out_dir = value(rest, idx, "--out-dir")?.into();
            }
            "--known-literature-threshold" => {
                idx += 1;
                args.known_literature_threshold = parse_u64(
                    value(rest, idx, "--known-literature-threshold")?,
                    "--known-literature-threshold",
                    1,
                )?;
            }
            "--min-stability-frequency" => {
                idx += 1;
                args.min_stability_frequency =
                    parse_unit(value(rest, idx, "--min-stability-frequency")?)?;
            }
            "--max-transcriptomic-class-breadth" => {
                idx += 1;
                args.max_transcriptomic_class_breadth = parse_u64(
                    value(rest, idx, "--max-transcriptomic-class-breadth")?,
                    "--max-transcriptomic-class-breadth",
                    1,
                )?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected biomedical-blindspot-audit flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require(!args.hypotheses_reports.is_empty(), "--hypotheses-report")?;
    require_path(&args.literature_audit, "--literature-audit")?;
    require_path(&args.stability_audit, "--stability-audit")?;
    require_path(&args.drug_lifecycle, "--drug-lifecycle")?;
    require_path(&args.transcriptomic_audit, "--transcriptomic-audit")?;
    require_path(&args.out_dir, "--out-dir")?;
    Ok(Subcommand::BiomedicalBlindspotAudit(args))
}
