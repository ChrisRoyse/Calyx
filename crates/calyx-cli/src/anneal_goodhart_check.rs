use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, ArtifactKey, ArtifactPtr, AsterAnnealLedgerStore, AsterRollbackStorage,
    GoodhartChecker, GoodhartLedgerContext, HeldOutSet, JValue, LensContributionDelta,
    RollbackStore, WardGtau, add_goodhart_penalty_to_vault, read_goodhart_state_from_vault,
    record_goodhart_report,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, FixedClock, Result as CalyxResult, SystemClock, VaultId};
use calyx_ledger::{ActorId, LedgerAppender};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = Request::parse(args)?;
    let bytes = fs::read(&request.fixture)
        .map_err(|error| format!("read fixture {}: {error}", request.fixture.display()))?;
    let fixture = serde_json::from_slice::<Fixture>(&bytes)
        .map_err(|error| format!("parse fixture {}: {error}", request.fixture.display()))?;
    let vault_id = VaultId::from_str(&request.vault_id)
        .map_err(|error| format!("invalid --vault-id: {error}"))?;
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.salt.into_bytes(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let prior_hash = fixture.prior_ptr_hash.unwrap_or([0x11; 32]);
    let candidate_hash = fixture.candidate_ptr_hash.unwrap_or([0x22; 32]);
    let state_before = read_goodhart_state_from_vault(&request.vault).map_err(|e| e.to_string())?;
    let checker = fixture.checker()?;
    let report = checker
        .check(
            &fixture.before,
            &fixture.after,
            &fixture.lens_contribution_deltas,
        )
        .map_err(|error| error.to_string())?;
    if fixture.expect_pass != report.passed {
        return Err(format!(
            "fixture expected passed={} but report passed={}",
            fixture.expect_pass, report.passed
        )
        .into());
    }
    let clock = FixedClock::new(fixture.ledger_ts);
    let rollback_readback = apply_synthetic_rollback(
        &vault,
        &clock,
        fixture.rollback_seed.unwrap_or(424),
        prior_hash,
        candidate_hash,
        &fixture.artifact_id,
        report.passed,
    )?;
    let change_id = rollback_readback.snapshot.change_id;
    let mut ledger = open_ledger(&vault, fixture.ledger_ts)?;
    let ledger_ref = record_goodhart_report(
        &report,
        GoodhartLedgerContext {
            change_id,
            artifact_id: fixture.artifact_id,
            prior_ptr_hash: prior_hash,
            candidate_ptr_hash: candidate_hash,
            ts: fixture.ledger_ts,
        },
        &mut ledger,
    )
    .map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())?;
    let state_after = if report.p_goodhart_increment > 0.0 {
        add_goodhart_penalty_to_vault(&request.vault, report.p_goodhart_increment)
            .map_err(|e| e.to_string())?
    } else {
        state_before
    };
    let readback = json!({
        "source_of_truth": "Aster anneal_rollback CF, Aster ledger CF/WAL, and .anneal/goodhart_state.toml",
        "fixture_path": request.fixture.display().to_string(),
        "fixture_len": bytes.len(),
        "fixture_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "vault": request.vault.display().to_string(),
        "goodhart_state_path": request.vault.join(".anneal/goodhart_state.toml").display().to_string(),
        "state_before": state_before,
        "state_after": state_after,
        "rollback_readback": rollback_readback,
        "ledger_ref": ledger_ref,
        "report": report,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|e| e.to_string())?
    );
    Ok(())
}

struct Request {
    fixture: PathBuf,
    vault: PathBuf,
    vault_id: String,
    salt: String,
}

impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut fixture = None;
        let mut vault = None;
        let mut vault_id = None;
        let mut salt = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--fixture" => {
                    fixture = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--vault-id" => {
                    vault_id = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--salt" => {
                    salt = args.get(idx + 1).cloned();
                    idx += 2;
                }
                other => return Err(format!("unknown goodhart-check arg: {other}")),
            }
        }
        Ok(Self {
            fixture: fixture
                .ok_or_else(|| "goodhart-check requires --fixture <json>".to_string())?,
            vault: vault.ok_or_else(|| "goodhart-check requires --vault <dir>".to_string())?,
            vault_id: vault_id
                .ok_or_else(|| "goodhart-check requires --vault-id <id>".to_string())?,
            salt: salt.ok_or_else(|| "goodhart-check requires --salt <s>".to_string())?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    artifact_id: String,
    ledger_ts: u64,
    before: JValue,
    after: JValue,
    held_out_set: HeldOutSet,
    #[serde(default)]
    ward_in_region_frac: Option<f64>,
    #[serde(default)]
    ward_unavailable: bool,
    #[serde(default)]
    lens_contribution_deltas: Vec<LensContributionDelta>,
    #[serde(default)]
    gtau_threshold: Option<f64>,
    #[serde(default)]
    cross_lens_threshold: Option<f64>,
    #[serde(default)]
    held_out_min_gain_fraction: Option<f64>,
    #[serde(default)]
    violation_penalty_weight: Option<f64>,
    #[serde(default)]
    prior_ptr_hash: Option<[u8; 32]>,
    #[serde(default)]
    candidate_ptr_hash: Option<[u8; 32]>,
    #[serde(default)]
    rollback_seed: Option<u64>,
    expect_pass: bool,
}

impl Fixture {
    fn checker(&self) -> Result<GoodhartChecker, String> {
        let ward = Arc::new(FixtureWard {
            in_region_frac: self.ward_in_region_frac,
            unavailable: self.ward_unavailable,
        });
        let mut checker = GoodhartChecker::new(self.held_out_set.clone(), ward);
        if let Some(value) = self.gtau_threshold {
            checker = checker.with_gtau_threshold(value);
        }
        if let Some(value) = self.cross_lens_threshold {
            checker = checker.with_cross_lens_threshold(value);
        }
        if let Some(value) = self.held_out_min_gain_fraction {
            checker = checker.with_held_out_min_gain_fraction(value);
        }
        if let Some(value) = self.violation_penalty_weight {
            checker = checker.with_violation_penalty_weight(value);
        }
        Ok(checker)
    }
}

struct FixtureWard {
    in_region_frac: Option<f64>,
    unavailable: bool,
}

impl WardGtau for FixtureWard {
    fn in_region_fraction(&self, _held_out_set: &HeldOutSet) -> CalyxResult<Option<f64>> {
        if self.unavailable {
            return Err(CalyxError {
                code: "CALYX_WARD_GTAU_UNAVAILABLE",
                message: "fixture ward marked unavailable".to_string(),
                remediation: "treat Gtau as fail-closed zero",
            });
        }
        Ok(self.in_region_frac)
    }
}

fn apply_synthetic_rollback(
    vault: &AsterVault,
    clock: &FixedClock,
    seed: u64,
    prior_hash: [u8; 32],
    candidate_hash: [u8; 32],
    artifact_id: &str,
    passed: bool,
) -> Result<calyx_anneal::RollbackReadback, String> {
    let store = RollbackStore::open(clock, seed, AsterRollbackStorage::new(vault))
        .map_err(|error| error.to_string())?;
    let key = ArtifactKey::ConfigCache(*blake3::hash(artifact_id.as_bytes()).as_bytes());
    store
        .install_live_ptr(key.clone(), ArtifactPtr::ConfigCacheKeyHash(prior_hash))
        .map_err(|error| error.to_string())?;
    let change_id = store
        .prepare_with_description(
            key,
            ArtifactPtr::ConfigCacheKeyHash(candidate_hash),
            "PH48 Goodhart synthetic candidate",
        )
        .map_err(|error| error.to_string())?;
    if passed {
        store
            .promote(change_id)
            .map_err(|error| error.to_string())?;
    } else {
        store
            .rollback(change_id)
            .map_err(|error| error.to_string())?;
    }
    store.readback(change_id).map_err(|error| error.to_string())
}

fn open_ledger<'a>(
    vault: &'a AsterVault,
    ts: u64,
) -> Result<AnnealLedger<AsterAnnealLedgerStore<'a, SystemClock>, FixedClock>, String> {
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), FixedClock::new(ts))
        .map_err(|error| error.to_string())?;
    AnnealLedger::new(
        appender,
        ActorId::Service("calyx-goodhart-check".to_string()),
    )
    .map_err(|error| error.to_string())
}
