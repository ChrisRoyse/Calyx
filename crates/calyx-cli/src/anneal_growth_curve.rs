use std::path::PathBuf;
use std::sync::Arc;

use calyx_anneal::{AsterGrowthCf, GrowthCurve};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SystemClock, VaultId};
use serde_json::json;

use crate::error::CliError;

const GROWTH_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const GROWTH_VAULT_SALT: &[u8] = b"calyx-anneal-intelligence-report";
const DEFAULT_LAST: usize = 20;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = GrowthCurveRequest::parse(args)?;
    let vault_id = GROWTH_VAULT_ID.parse::<VaultId>().map_err(|error| {
        CliError::runtime(format!("CALYX_ANNEAL_GROWTH_INVALID_CONFIG: {error}"))
    })?;
    let vault = AsterVault::open(
        &request.vault,
        vault_id,
        GROWTH_VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )?;
    let cf = AsterGrowthCf::new(&vault);
    let curve = GrowthCurve::new(cf, Arc::new(SystemClock))?;
    let samples = curve
        .samples()
        .rev()
        .take(request.last)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let readback = json!({
        "source_of_truth": format!("{}/cf/anneal_growth", request.vault.display()),
        "last": request.last,
        "summary": curve.curve_summary_with_window(request.last),
        "plot_ascii": curve.plot_ascii(60, 10),
        "samples": samples,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| {
            CliError::runtime(format!("serialize growth-curve readback: {error}"))
        })?
    );
    Ok(())
}

struct GrowthCurveRequest {
    vault: PathBuf,
    last: usize,
}

impl GrowthCurveRequest {
    fn parse(args: &[String]) -> crate::error::CliResult<Self> {
        let mut vault = None;
        let mut last = DEFAULT_LAST;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--last" => {
                    last = args
                        .get(idx + 1)
                        .ok_or_else(|| CliError::usage("--last requires a value"))?
                        .parse::<usize>()
                        .map_err(|error| CliError::usage(format!("invalid --last: {error}")))?;
                    idx += 2;
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unknown growth-curve arg: {other}"
                    )));
                }
            }
        }
        Ok(Self {
            vault: vault.ok_or_else(|| CliError::usage("growth-curve requires --vault <dir>"))?,
            last,
        })
    }
}
