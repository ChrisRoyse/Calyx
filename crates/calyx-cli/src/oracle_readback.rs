mod butterfly;
mod predict;
mod reverse_query;
mod super_intelligence;

use std::path::Path;
use std::str::FromStr;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, FixedClock, Panel, SlotId, SystemClock, VaultId};
use calyx_oracle::{DomainId, OracleError, check_sufficiency, oracle_self_consistency};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn is_topic(topic: &str) -> bool {
    matches!(
        topic,
        "oracle_self_consistency"
            | "oracle_sufficiency"
            | "oracle_predict"
            | "oracle_expand"
            | "reverse_query"
            | "super_intelligence"
    )
}

pub(crate) fn readback_oracle(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "oracle_self_consistency" => readback_oracle_self_consistency(args),
        "oracle_sufficiency" => readback_oracle_sufficiency(args),
        "oracle_predict" => predict::readback_oracle_predict(args),
        "oracle_expand" => butterfly::readback_oracle_expand(args),
        "reverse_query" => reverse_query::readback_reverse_query(args),
        "super_intelligence" => super_intelligence::readback_super_intelligence(args),
        _ => Err("unknown oracle readback topic".to_string().into()),
    }
}

pub(crate) fn readback_oracle_self_consistency(args: &[String]) -> crate::error::CliResult {
    match args {
        [vault_flag, vault, domain_flag, domain, vault_id_flag, vault_id, salt_flag, salt]
            if vault_flag == "--vault"
                && domain_flag == "--domain"
                && vault_id_flag == "--vault-id"
                && salt_flag == "--salt" =>
        {
            let vault_id =
                VaultId::from_str(vault_id).map_err(|error| format!("invalid --vault-id: {error}"))?;
            let vault = AsterVault::new_durable(
                Path::new(vault),
                vault_id,
                salt.as_bytes().to_vec(),
                VaultOptions::default(),
            )
            .map_err(|error| error.to_string())?;
            let clock = SystemClock;
            match oracle_self_consistency(&vault, DomainId::from(domain.clone()), &clock) {
                Ok(result) => {
                    vault
                        .flush()
                        .map_err(|error| format!("flush oracle ledger row: {error}"))?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
                    );
                    Ok(())
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "domain": domain,
                            "error_code": error.code(),
                            "error": error.to_string(),
                        }))
                        .map_err(|error| error.to_string())?
                    );
                    Err(error.to_string().into())
                }
            }
        }
        _ => Err("usage: calyx readback oracle_self_consistency --vault <dir> --domain <domain> --vault-id <id> --salt <s>".to_string().into()),
    }
}

fn readback_oracle_sufficiency(args: &[String]) -> crate::error::CliResult {
    match args {
        [vault_flag, vault, fixture_flag, fixture, vault_id_flag, vault_id, salt_flag, salt]
            if vault_flag == "--vault"
                && fixture_flag == "--fixture"
                && vault_id_flag == "--vault-id"
                && salt_flag == "--salt" =>
        {
            let vault_id =
                VaultId::from_str(vault_id).map_err(|error| format!("invalid --vault-id: {error}"))?;
            let vault = AsterVault::new_durable(
                Path::new(vault),
                vault_id,
                salt.as_bytes().to_vec(),
                VaultOptions::default(),
            )
            .map_err(|error| error.to_string())?;
            let fixture = SufficiencyFixture::read(Path::new(fixture))?;
            let rows_written = fixture.persist_assay_rows(&vault)?;
            let clock = FixedClock::new(fixture.clock_ts);
            match check_sufficiency(
                &vault,
                &fixture.panel,
                DomainId::from(fixture.domain.clone()),
                &clock,
            ) {
                Ok(bound) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "domain": fixture.domain,
                            "assay_rows_written": rows_written,
                            "bound": bound,
                        }))
                        .map_err(|error| error.to_string())?
                    );
                    Ok(())
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&error_payload(
                            &fixture.domain,
                            rows_written,
                            &error
                        ))
                        .map_err(|error| error.to_string())?
                    );
                    Err(error.to_string().into())
                }
            }
        }
        _ => Err("usage: calyx readback oracle_sufficiency --vault <dir> --fixture <json> --vault-id <id> --salt <s>".to_string().into()),
    }
}

#[derive(Debug, Deserialize)]
struct SufficiencyFixture {
    domain: String,
    panel: Panel,
    #[serde(rename = "I_panel_oracle")]
    panel_bits: f32,
    outcome_entropy_bits: f32,
    slot_bits: Vec<FixtureSlotBits>,
    #[serde(default = "default_samples")]
    n_samples: usize,
    #[serde(default = "default_trust")]
    trust: TrustTag,
    clock_ts: u64,
}

#[derive(Debug, Deserialize)]
struct FixtureSlotBits {
    slot: SlotId,
    bits: f32,
}

impl SufficiencyFixture {
    fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("read fixture: {error}"))?;
        let fixture: Self =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse fixture: {error}"))?;
        fixture.validate()?;
        Ok(fixture)
    }

    fn validate(&self) -> Result<(), String> {
        validate_bits(self.panel_bits, "I_panel_oracle")?;
        validate_bits(self.outcome_entropy_bits, "outcome_entropy_bits")?;
        for slot in &self.slot_bits {
            validate_bits(slot.bits, "slot_bits")?;
        }
        if self.n_samples == 0 {
            return Err("oracle_sufficiency fixture n_samples must be positive".to_string());
        }
        Ok(())
    }

    fn persist_assay_rows(&self, vault: &AsterVault) -> Result<usize, String> {
        let key = AssayCacheKey::scoped(
            self.panel.version,
            self.domain.clone(),
            vault.vault_id(),
            AnchorKind::Reward,
        );
        let mut store = AssayStore::default();
        store.put(
            key.clone(),
            AssaySubject::Panel,
            self.estimate(self.panel_bits, EstimatorKind::PanelSufficiency),
            "oracle sufficiency panel bits fixture",
            self.clock_ts,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            self.estimate(self.outcome_entropy_bits, EstimatorKind::OutcomeEntropy),
            "oracle sufficiency outcome entropy fixture",
            self.clock_ts,
        );
        for slot in &self.slot_bits {
            store.put(
                key.clone(),
                AssaySubject::Lens { slot: slot.slot },
                self.estimate(slot.bits, EstimatorKind::Ksg),
                "oracle sufficiency lens bits fixture",
                self.clock_ts,
            );
        }
        store
            .persist_to_vault(vault)
            .map_err(|error| error.to_string())
    }

    fn estimate(&self, bits: f32, estimator: EstimatorKind) -> MiEstimate {
        MiEstimate::point(bits, self.n_samples, estimator, self.trust)
    }
}

fn validate_bits(value: f32, name: &str) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and non-negative"))
    }
}

fn error_payload(domain: &str, rows_written: usize, error: &OracleError) -> serde_json::Value {
    let mut payload = json!({
        "domain": domain,
        "assay_rows_written": rows_written,
        "error_code": error.code(),
        "error": error.to_string(),
        "remediation": error.remediation(),
    });
    if let OracleError::Insufficient { bound } = error {
        payload["bound"] = json!(bound);
    }
    payload
}

fn default_samples() -> usize {
    120
}

fn default_trust() -> TrustTag {
    TrustTag::Trusted
}
