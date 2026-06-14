use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::encode;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, AnchorValue, Clock, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, Panel, SlotId, VaultId, content_address,
};
use calyx_oracle::{
    Action, DomainId, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, OracleError,
    oracle_predict,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn readback_oracle_predict(args: &[String]) -> crate::error::CliResult {
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
            let fixture = PredictionFixture::read(Path::new(fixture))?;
            let rows_written = fixture.persist_rows(&vault)?;
            vault
                .flush()
                .map_err(|error| format!("flush oracle predict fixture rows: {error}"))?;
            let clock = FixedClock::new(fixture.clock_ts);
            let action = Action {
                action_id: fixture.action_id.clone(),
                panel: fixture.panel.clone(),
                guard: None,
            };
            match oracle_predict(&vault, &action, DomainId::from(fixture.domain.clone()), &clock) {
                Ok(prediction) => {
                    vault
                        .flush()
                        .map_err(|error| format!("flush oracle predict: {error}"))?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "domain": fixture.domain,
                            "action_id": fixture.action_id,
                            "rows_written": rows_written,
                            "prediction": prediction,
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
        _ => Err("usage: calyx readback oracle_predict --vault <dir> --fixture <json> --vault-id <id> --salt <s>".to_string().into()),
    }
}

#[derive(Debug, Deserialize)]
struct PredictionFixture {
    domain: String,
    action_id: String,
    panel: Panel,
    #[serde(rename = "I_panel_oracle")]
    panel_bits: f32,
    outcome_entropy_bits: f32,
    slot_bits: Vec<FixtureSlotBits>,
    prediction_observations: Vec<PredictionObservation>,
    self_consistency_series: Vec<Vec<ConsistencyObservation>>,
    #[serde(default = "default_samples")]
    n_samples: usize,
    #[serde(default = "default_trust")]
    trust: TrustTag,
    clock_ts: u64,
}

#[derive(Debug, Deserialize)]
struct PredictionObservation {
    outcome: AnchorValue,
    count: usize,
    #[serde(default)]
    consequence: Option<FixtureConsequence>,
}

#[derive(Debug, Deserialize)]
struct ConsistencyObservation {
    outcome: AnchorValue,
    #[serde(default)]
    ground_truth: Option<AnchorValue>,
}

#[derive(Debug, Deserialize)]
struct FixtureConsequence {
    action_or_event: String,
    #[serde(default)]
    domain: Option<String>,
    outcome: AnchorValue,
}

#[derive(Debug, Deserialize)]
struct FixtureSlotBits {
    slot: SlotId,
    bits: f32,
}

impl PredictionFixture {
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
        if self.action_id.trim().is_empty() {
            return Err("oracle_predict fixture action_id must be non-empty".to_string());
        }
        if self.n_samples == 0 {
            return Err("oracle_predict fixture n_samples must be positive".to_string());
        }
        Ok(())
    }

    fn persist_rows<C>(&self, vault: &AsterVault<C>) -> Result<usize, String>
    where
        C: Clock,
    {
        let mut row_count = self.persist_assay_rows(vault)?;
        for (index, series) in self.self_consistency_series.iter().enumerate() {
            let series_rows = series.iter().map(FixtureRow::from).collect::<Vec<_>>();
            row_count += write_series(
                vault,
                &self.domain,
                "oracle_predict_self_consistency",
                &format!("self-{index}"),
                &series_rows,
            )?;
        }
        let mut index = 0_usize;
        for observation in &self.prediction_observations {
            for _ in 0..observation.count {
                let row = FixtureRow {
                    outcome: observation.outcome.clone(),
                    ground_truth: None,
                    consequence: observation.consequence.as_ref(),
                };
                row_count += write_series(
                    vault,
                    &self.domain,
                    &self.action_id,
                    &format!("predict-{index}"),
                    &[row],
                )?;
                index += 1;
            }
        }
        Ok(row_count)
    }

    fn persist_assay_rows<C>(&self, vault: &AsterVault<C>) -> Result<usize, String>
    where
        C: Clock,
    {
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
            "oracle predict panel bits fixture",
            self.clock_ts,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            self.estimate(self.outcome_entropy_bits, EstimatorKind::OutcomeEntropy),
            "oracle predict entropy fixture",
            self.clock_ts,
        );
        for slot in &self.slot_bits {
            store.put(
                key.clone(),
                AssaySubject::Lens { slot: slot.slot },
                self.estimate(slot.bits, EstimatorKind::Ksg),
                "oracle predict lens bits fixture",
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

struct FixtureRow<'a> {
    outcome: AnchorValue,
    ground_truth: Option<AnchorValue>,
    consequence: Option<&'a FixtureConsequence>,
}

impl From<&ConsistencyObservation> for FixtureRow<'_> {
    fn from(value: &ConsistencyObservation) -> Self {
        Self {
            outcome: value.outcome.clone(),
            ground_truth: value.ground_truth.clone(),
            consequence: None,
        }
    }
}

fn write_series<C>(
    vault: &AsterVault<C>,
    domain: &str,
    action_id: &str,
    series_key: &str,
    rows: &[FixtureRow<'_>],
) -> Result<usize, String>
where
    C: Clock,
{
    let cx_id = CxId::from_bytes(content_address([
        domain.as_bytes(),
        action_id.as_bytes(),
        series_key.as_bytes(),
    ]));
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&fixture_constellation(
                vault.vault_id(),
                cx_id,
                domain,
                action_id,
            ))
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    for (index, row) in rows.iter().enumerate() {
        let occurrence = Occurrence {
            id: OccurrenceId(index as u64),
            t_k: EpochSecs(1_000 + index as i64),
            context: OccurrenceContext::new(fixture_context(domain, action_id, row))
                .map_err(|error| error.to_string())?,
        };
        vault
            .write_cf(
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, index as u64),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(1 + rows.len())
}

fn fixture_constellation(
    vault_id: VaultId,
    cx_id: CxId,
    domain: &str,
    action_id: &str,
) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(
        ORACLE_ACTION_METADATA_KEY.to_string(),
        action_id.to_string(),
    );
    Constellation {
        cx_id,
        vault_id,
        panel_version: 432,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn fixture_context(domain: &str, action_id: &str, row: &FixtureRow<'_>) -> Vec<u8> {
    let mut value = json!({
        "action": action_id,
        "oracle_verdict": { "value": &row.outcome },
        "outcome_anchor": { "value": &row.outcome }
    });
    if let Some(truth) = &row.ground_truth {
        value["ground_truth_anchor"] = json!({ "value": truth });
    }
    if let Some(consequence) = row.consequence {
        value["consequences"] = json!([{
            "action_or_event": &consequence.action_or_event,
            "domain": consequence.domain.as_deref().unwrap_or(domain),
            "outcome": { "value": &consequence.outcome }
        }]);
    }
    serde_json::to_vec(&value).expect("fixture context json")
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
