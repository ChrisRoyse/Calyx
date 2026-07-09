//! Vault-backed calyx-oracle forward prediction as a first-class forecast component (issue #215,
//! split from #83).
//!
//! Issue #83's formula layer (`oracle_ceiling` for the confidence cap #87, `super_intelligence` for
//! the superiority predicate #88) is already composed by the CalyxNative producer. What was deferred
//! (#215) is the **vault-backed forward predictor** — `calyx_oracle::oracle_predict`, which reads a
//! real AsterVault recurrence (recurrence → outcome + confidence) — wired as a blended
//! [`ForecastComponent`] alongside kNN (#81) and bits-vote (#84).
//!
//! This module runs that predictor against a seeded AsterVault and turns its `Prediction` into an
//! [`ComponentKind::Oracle`] component:
//!
//! - **probability** — the oracle predicts the modal outcome with a calibrated confidence in
//!   `[0, 1]`. For a binary YES/NO market this maps monotonically to `P(YES)`: modal == YES →
//!   `0.5 + 0.5·confidence`, modal == not-YES → `0.5 − 0.5·confidence`. A coin-flip oracle
//!   (`confidence = 0`) contributes `p = 0.5` and moves the blend nowhere.
//! - **reliability** — the domain's measured oracle self-consistency `validity·(1 − flakiness)`
//!   (`oracle_self_consistency`), a held-out `[0, 1]` quality of the oracle itself. A flaky/invalid
//!   oracle earns ~0 reliability and is dropped by the logit pool — a principled weight, not a guess.
//! - **support** — the recurrence observation count, read back **from the vault ledger CF** (the
//!   audit row `oracle_predict` writes), i.e. full-state-verified against the vault source of truth
//!   rather than trusted from a return value.
//! - **trust** — `Provisional` when the prediction's guard is provisional (proxy-grounded), else
//!   `Trusted`, so a provisional oracle makes the whole blend provisional (never silently upgraded).
//!
//! Fail closed: no recurrence for the action, a ledger row that cannot be read back or does not carry
//! the expected `oracle_predict_v1` audit shape, or an out-of-range probability is a hard error.

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorValue, Clock, VaultStore};
use calyx_oracle::{Action, DomainId, OracleError, oracle_predict, oracle_self_consistency};
use serde::{Deserialize, Serialize};

use calyx_assay::TrustTag;

use crate::error::{PolyError, Result};
use crate::forecast::{ComponentKind, ForecastComponent};

/// Schema tag persisted with every oracle forecast component.
pub const ORACLE_FORECAST_SCHEMA_VERSION: &str = "poly.oracle_forecast.v1";
/// Artifact-kind tag.
pub const ORACLE_FORECAST_ARTIFACT_KIND: &str = "poly_oracle_forecast_component";
/// The ledger tag the vault-backed oracle predictor writes; the FSV readback must match it.
pub const ORACLE_PREDICT_LEDGER_TAG: &str = "oracle_predict_v1";

/// The vault-backed forward predictor errored.
pub const ERR_ORACLE_PREDICT: &str = "CALYX_POLY_ORACLE_FORECAST_PREDICT_FAILED";
/// The domain oracle self-consistency (reliability source) could not be measured.
pub const ERR_ORACLE_CONSISTENCY: &str = "CALYX_POLY_ORACLE_FORECAST_CONSISTENCY_FAILED";
/// The prediction's provenance ledger row could not be read back from the vault CF.
pub const ERR_ORACLE_LEDGER_READBACK: &str = "CALYX_POLY_ORACLE_FORECAST_LEDGER_READBACK";
/// The provenance ledger row did not carry the expected `oracle_predict_v1` audit shape.
pub const ERR_ORACLE_LEDGER_SHAPE: &str = "CALYX_POLY_ORACLE_FORECAST_LEDGER_SHAPE";

/// The vault-backed oracle forward prediction turned into a blend component, plus the ledger-verified
/// evidence it was measured on — the durable FSV source of truth on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleForecast {
    /// Schema tag.
    pub schema_version: String,
    /// Artifact-kind tag.
    pub artifact_kind: String,
    /// Domain slug.
    pub domain: String,
    /// Action id the recurrence was queried for.
    pub action_id: String,
    /// The canonical JSON of the outcome that counts as YES for this market.
    pub yes_outcome_label: String,
    /// The canonical JSON of the oracle's predicted modal outcome.
    pub predicted_outcome_label: String,
    /// Whether the predicted modal outcome is the YES outcome.
    pub predicted_is_yes: bool,
    /// The oracle's calibrated confidence in the modal outcome (`[0, 1]`).
    pub oracle_confidence: f64,
    /// The mapped `P(YES)` the component carries.
    pub p_yes: f64,
    /// The domain oracle self-consistency `validity·(1 − flakiness)` used as reliability.
    pub self_consistency: f64,
    /// Recurrence observation count, read back from the vault ledger CF (source of truth).
    pub recurrence_observations: u64,
    /// DPI ceiling the prediction was bounded by.
    pub dpi_ceiling: f64,
    /// Panel↔oracle mutual information the sufficiency bound measured.
    pub i_panel_oracle: f64,
    /// Whether the prediction's guard was provisional (proxy-grounded).
    pub provisional: bool,
    /// Trust of the emitted component.
    pub trust: TrustTag,
    /// Provenance ledger sequence the audit row was read back from.
    pub ledger_seq: u64,
    /// The emitted blend component.
    pub component: ForecastComponent,
}

/// Runs the vault-backed oracle forward predictor for `action`/`domain` and turns it into a blend
/// component. `yes_outcome` is the [`AnchorValue`] that counts as YES for this binary market.
pub fn produce_oracle_forecast_component<C>(
    vault: &AsterVault<C>,
    action: &Action,
    domain: DomainId,
    yes_outcome: &AnchorValue,
    clock: &dyn Clock,
) -> Result<OracleForecast>
where
    C: Clock,
{
    let prediction = oracle_predict(vault, action, domain.clone(), clock)
        .map_err(|err| oracle_error(ERR_ORACLE_PREDICT, &domain, err))?;

    // Principled reliability: the domain's measured oracle self-consistency (held-out quality).
    // Round f32-derived values to 6 decimals so the persisted JSON is a byte-exact FSV round-trip
    // (an f32 cast straight to f64 carries a long mantissa that JSON does not always round-trip;
    // this mirrors `kernel_recall::ratio_for_report`).
    let consistency = oracle_self_consistency(vault, domain.clone(), clock)
        .map_err(|err| oracle_error(ERR_ORACLE_CONSISTENCY, &domain, err))?;
    let reliability = round6(unit_clamp(consistency.ceiling as f64));

    // FSV against the vault CF: read the audit row `oracle_predict` just wrote and recover the real
    // recurrence observation count from the source of truth, not from the return value.
    let recurrence_observations =
        read_ledger_recurrence_observations(vault, prediction.provenance.seq, &domain)?;

    let predicted_is_yes = &prediction.outcome == yes_outcome;
    let oracle_confidence = round6(unit_clamp(prediction.confidence as f64));
    let p_yes = round6(map_p_yes(oracle_confidence, predicted_is_yes));

    let provisional = prediction
        .guard
        .as_ref()
        .is_none_or(|guard| guard.provisional);
    let trust = if provisional {
        TrustTag::Provisional
    } else {
        TrustTag::Trusted
    };

    let n_support = usize::try_from(recurrence_observations).map_err(|_| {
        PolyError::diagnostics(
            ERR_ORACLE_LEDGER_SHAPE,
            format!(
                "domain {} recurrence_observations {recurrence_observations} exceeds usize",
                domain.as_str()
            ),
        )
    })?;

    let detail = format!(
        "vault-backed oracle: modal={} is_yes={} conf={:.6} obs={} self_consistency={:.6} seq={}",
        canonical(&prediction.outcome),
        predicted_is_yes,
        oracle_confidence,
        recurrence_observations,
        reliability,
        prediction.provenance.seq
    );
    let component = ForecastComponent::new(
        ComponentKind::Oracle,
        p_yes,
        reliability,
        n_support,
        trust,
        detail,
    )?;

    Ok(OracleForecast {
        schema_version: ORACLE_FORECAST_SCHEMA_VERSION.to_string(),
        artifact_kind: ORACLE_FORECAST_ARTIFACT_KIND.to_string(),
        domain: domain.as_str().to_string(),
        action_id: action.action_id.clone(),
        yes_outcome_label: canonical(yes_outcome),
        predicted_outcome_label: canonical(&prediction.outcome),
        predicted_is_yes,
        oracle_confidence,
        p_yes,
        self_consistency: reliability,
        recurrence_observations,
        dpi_ceiling: round6(unit_clamp(prediction.bound.dpi_ceiling_unit.get() as f64)),
        i_panel_oracle: round6(prediction.bound.i_panel_oracle.get() as f64),
        provisional,
        trust,
        ledger_seq: prediction.provenance.seq,
        component,
    })
}

/// Maps an oracle modal-outcome confidence to `P(YES)` for a binary market. Monotone in confidence:
/// modal == YES puts `p` in `[0.5, 1]`, modal == not-YES puts it in `[0, 0.5]`; `confidence = 0`
/// (no separation) → `0.5` regardless of side.
pub fn map_p_yes(confidence: f64, predicted_is_yes: bool) -> f64 {
    let c = unit_clamp(confidence);
    let p = if predicted_is_yes {
        0.5 + 0.5 * c
    } else {
        0.5 - 0.5 * c
    };
    p.clamp(0.0, 1.0)
}

/// Reads the recurrence observation count from the vault ledger CF audit row for `seq`.
fn read_ledger_recurrence_observations<C>(
    vault: &AsterVault<C>,
    seq: u64,
    domain: &DomainId,
) -> Result<u64>
where
    C: Clock,
{
    let bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .map_err(|err| {
            PolyError::diagnostics(
                ERR_ORACLE_LEDGER_READBACK,
                format!("domain {} read ledger seq {seq}: {err}", domain.as_str()),
            )
        })?
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_ORACLE_LEDGER_READBACK,
                format!(
                    "domain {} oracle prediction provenance ledger seq {seq} is absent from the \
                     vault CF",
                    domain.as_str()
                ),
            )
        })?;
    let entry = calyx_ledger::decode(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_ORACLE_LEDGER_READBACK,
            format!("domain {} decode ledger seq {seq}: {err}", domain.as_str()),
        )
    })?;
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).map_err(|err| {
        PolyError::diagnostics(
            ERR_ORACLE_LEDGER_SHAPE,
            format!(
                "domain {} ledger seq {seq} payload is not oracle-predict JSON: {err}",
                domain.as_str()
            ),
        )
    })?;
    let tag = payload.get("tag").and_then(|value| value.as_str());
    if tag != Some(ORACLE_PREDICT_LEDGER_TAG) {
        return Err(PolyError::diagnostics(
            ERR_ORACLE_LEDGER_SHAPE,
            format!(
                "domain {} ledger seq {seq} tag {tag:?}, expected {ORACLE_PREDICT_LEDGER_TAG}",
                domain.as_str()
            ),
        ));
    }
    payload
        .get("recurrence_observations")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_ORACLE_LEDGER_SHAPE,
                format!(
                    "domain {} ledger seq {seq} missing integer recurrence_observations",
                    domain.as_str()
                ),
            )
        })
}

/// Persists an oracle forecast component as JSON and returns its path.
pub fn write_oracle_forecast(
    dir: &std::path::Path,
    forecast: &OracleForecast,
) -> Result<std::path::PathBuf> {
    let file_name = format!(
        "oracle_forecast_{}_{}.json",
        sanitize(&forecast.domain),
        sanitize(&forecast.action_id)
    );
    crate::diagnostics_store::write_json(dir, &file_name, forecast)
}

/// Reads a persisted oracle forecast component back from disk.
pub fn read_oracle_forecast(path: &std::path::Path) -> Result<OracleForecast> {
    crate::diagnostics_store::read_json(path)
}

fn canonical(value: &AnchorValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unserializable-anchor>".to_string())
}

/// Rounds an f32-derived f64 to 6 decimals so persisted JSON round-trips byte-exactly.
fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn unit_clamp(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn oracle_error(code: &'static str, domain: &DomainId, err: OracleError) -> PolyError {
    PolyError::diagnostics(
        code,
        format!(
            "domain {} vault-backed oracle failed [{}]: {err}",
            domain.as_str(),
            err.code()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p_yes_mapping_is_monotone_and_centered() {
        // Coin-flip oracle → 0.5 regardless of side.
        assert!((map_p_yes(0.0, true) - 0.5).abs() < 1e-12);
        assert!((map_p_yes(0.0, false) - 0.5).abs() < 1e-12);
        // Confident YES climbs above 0.5; confident not-YES drops below.
        assert!((map_p_yes(1.0, true) - 1.0).abs() < 1e-12);
        assert!((map_p_yes(1.0, false) - 0.0).abs() < 1e-12);
        assert!((map_p_yes(0.5, true) - 0.75).abs() < 1e-12);
        assert!((map_p_yes(0.5, false) - 0.25).abs() < 1e-12);
        // Monotone in confidence on the YES side.
        assert!(map_p_yes(0.2, true) < map_p_yes(0.8, true));
    }

    #[test]
    fn non_finite_confidence_clamps_to_coin_flip() {
        assert!((map_p_yes(f64::NAN, true) - 0.5).abs() < 1e-12);
        assert!((map_p_yes(f64::INFINITY, false) - 0.5).abs() < 1e-12);
    }
}
