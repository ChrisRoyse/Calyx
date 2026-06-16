use std::path::Path;

use calyx_anneal::{ComponentHealth, TripwireMetric, decode_anneal_ledger_payload};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, VaultStore};
use calyx_ledger::EntryKind;
use serde::Serialize;
use serde_json::Value;

use super::{ledger_entries, open_vault};
use crate::cmd::vault::ResolvedVault;
use crate::error::CliResult;
use crate::output::print_json;

#[derive(Serialize)]
pub(super) struct AnnealStatusOut {
    phase: &'static str,
    tripwires: Vec<TripwireOut>,
    proposals: Vec<ProposalOut>,
    last_soak_at: Option<u64>,
    p99_latency_ms: Option<f64>,
    health: Vec<HealthOut>,
    recent_changes: Vec<RecentAnnealOut>,
}

#[derive(Serialize)]
struct TripwireOut {
    name: String,
    state: &'static str,
}

#[derive(Serialize)]
struct ProposalOut {
    #[serde(rename = "type")]
    proposal_type: String,
    rationale: Option<String>,
    name: Option<String>,
}

#[derive(Serialize)]
struct HealthOut {
    component: String,
    state: String,
    updated_at: u64,
}

#[derive(Serialize)]
struct RecentAnnealOut {
    seq: u64,
    action: String,
    ts: u64,
    description: String,
}

pub(super) fn run(resolved: &ResolvedVault) -> CliResult {
    let vault = open_vault(resolved)?;
    print_json(&anneal_status(&resolved.path, &vault)?)
}

pub(super) fn anneal_status(path: &Path, vault: &AsterVault) -> CliResult<AnnealStatusOut> {
    let tripwires = tripwire_rows(path)?;
    let proposals = proposal_rows(vault)?;
    let health = health_rows(vault)?;
    let recent_changes = recent_anneal(path)?;
    if tripwires.is_empty()
        && proposals.is_empty()
        && health.is_empty()
        && recent_changes.is_empty()
    {
        return Err(CalyxError::stale_derived(
            "anneal-status has no tripwire, proposal, health, or anneal ledger state",
        )
        .into());
    }
    let healing = health.iter().any(|row| row.state != "Ok");
    let phase = if healing {
        "healing"
    } else if !proposals.is_empty() || !recent_changes.is_empty() {
        "tuning"
    } else {
        "stable"
    };
    let last_soak_at = recent_changes
        .iter()
        .map(|row| row.ts)
        .chain(health.iter().map(|row| row.updated_at))
        .max();
    Ok(AnnealStatusOut {
        phase,
        tripwires,
        proposals,
        last_soak_at,
        p99_latency_ms: latest_p99(path)?,
        health,
        recent_changes,
    })
}

fn tripwire_rows(path: &Path) -> CliResult<Vec<TripwireOut>> {
    let config = calyx_anneal::tripwire_config_path(path);
    if !config.exists() {
        return Ok(Vec::new());
    }
    Ok(calyx_anneal::read_tripwire_config_from_vault(path)?
        .thresholds
        .into_iter()
        .map(|entry| TripwireOut {
            name: tripwire_metric_name(entry.metric),
            state: "armed",
        })
        .collect())
}

fn proposal_rows(vault: &AsterVault) -> CliResult<Vec<ProposalOut>> {
    let mut out = Vec::new();
    for (_key, value) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::AnnealOperators)? {
        let row: Value = serde_json::from_slice(&value).map_err(|error| {
            CalyxError::ledger_corrupt(format!("decode anneal proposal row: {error}"))
        })?;
        out.push(ProposalOut {
            proposal_type: row
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("add_lens")
                .to_string(),
            rationale: row
                .get("rationale")
                .and_then(Value::as_str)
                .map(str::to_string),
            name: row.get("name").and_then(Value::as_str).map(str::to_string),
        });
    }
    Ok(out)
}

fn health_rows(vault: &AsterVault) -> CliResult<Vec<HealthOut>> {
    let mut out = Vec::new();
    for (_key, value) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::AnnealHealth)? {
        let row = calyx_anneal::decode_health_value(&value)?;
        out.push(HealthOut {
            component: row.kind.to_string(),
            state: health_state(&row.health).to_string(),
            updated_at: row.updated_at,
        });
    }
    Ok(out)
}

fn recent_anneal(path: &Path) -> CliResult<Vec<RecentAnnealOut>> {
    let mut out = Vec::new();
    for entry in ledger_entries(path)? {
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal = decode_anneal_ledger_payload(&entry.payload)?;
        out.push(RecentAnnealOut {
            seq: entry.seq,
            action: format!("{:?}", anneal.action),
            ts: anneal.ts,
            description: anneal.description,
        });
    }
    if out.len() > 16 {
        out.drain(0..out.len() - 16);
    }
    Ok(out)
}

fn latest_p99(path: &Path) -> CliResult<Option<f64>> {
    let mut latest = None;
    for entry in ledger_entries(path)? {
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal = decode_anneal_ledger_payload(&entry.payload)?;
        for metric in anneal.metrics.metrics {
            if metric.metric == TripwireMetric::SearchP99 {
                latest = Some(metric.candidate_value);
            }
        }
    }
    Ok(latest)
}

fn tripwire_metric_name(metric: TripwireMetric) -> String {
    serde_json::to_value(metric)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{metric:?}"))
}

fn health_state(health: &ComponentHealth) -> &'static str {
    match health {
        ComponentHealth::Ok => "Ok",
        ComponentHealth::Degraded { .. } => "Degraded",
        ComponentHealth::Failing { .. } => "Failing",
        ComponentHealth::Parked { .. } => "Parked",
    }
}
