use calyx_assay::{
    AssayCacheKey, AssayRow, AssayStore, AssaySubject, DeficitRoutingContext, PanelSufficiency,
    panel_sufficiency_from_estimate_with_context, per_sensor_attribution,
};
use calyx_core::{AnchorKind, Panel, SlotId, VaultId};
use calyx_lodestar::{
    DiscoveryCandidate, DiscoveryChainParams, DiscoveryGateVerdict, LodestarError,
    reachability_prior_gate,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

pub(crate) const DEFAULT_DISCOVERY_ASSAY_DOMAIN: &str = "discovery-chain";
const SOLE_CARRIER_BITS: f32 = 0.10;

pub(crate) struct DiscoverySufficiencyGate {
    panel_version: u32,
    assay_domain: String,
    assay_anchor: AnchorKind,
    panel_bits: f32,
    ci_low: f32,
    ci_high: f32,
    anchor_entropy_bits: f32,
    n_samples: usize,
    power_recovery_ratio: f32,
    report: PanelSufficiency,
}

impl DiscoverySufficiencyGate {
    pub(crate) fn from_store(
        store: &AssayStore,
        panel: &Panel,
        vault_id: VaultId,
        assay_domain: &str,
        assay_anchor: AnchorKind,
    ) -> Result<Self, LodestarError> {
        let key =
            AssayCacheKey::scoped(panel.version, assay_domain, vault_id, assay_anchor.clone());
        let panel_row = required_assay_row(store, &key, &AssaySubject::Panel)?;
        let entropy_row = required_assay_row(store, &key, &AssaySubject::OutcomeEntropy)?;
        let slot_bits = panel
            .slots
            .iter()
            .map(|slot| {
                let subject = AssaySubject::Lens { slot: slot.slot_id };
                let row = required_assay_row(store, &key, &subject)?;
                Ok((slot.slot_id, finite_assay_bits(row, "lens")?))
            })
            .collect::<Result<Vec<(SlotId, f32)>, LodestarError>>()?;
        let anchor_entropy_bits = finite_assay_bits(entropy_row, "outcome entropy")?;
        let attributions = per_sensor_attribution(&slot_bits, SOLE_CARRIER_BITS);
        let report = panel_sufficiency_from_estimate_with_context(
            &panel_row.estimate,
            anchor_entropy_bits,
            &attributions,
            panel_row.estimate.trust,
            DeficitRoutingContext {
                panel_id: format!("discovery-chain:{assay_domain}:panel:{}", panel.version),
                anchor: assay_anchor.clone(),
                computed_at_seq: panel_row.written_at_seq,
                observation_scope: None,
            },
        )
        .map_err(|error| no_sufficiency_assay(error.message))?;
        let calibration = report.power_calibration.as_ref().ok_or_else(|| {
            no_sufficiency_assay("panel sufficiency report lacks power calibration")
        })?;
        Ok(Self {
            panel_version: panel.version,
            assay_domain: assay_domain.to_string(),
            assay_anchor,
            panel_bits: panel_row.estimate.bits,
            ci_low: panel_row.estimate.ci_low,
            ci_high: panel_row.estimate.ci_high,
            anchor_entropy_bits,
            n_samples: panel_row.estimate.n_samples,
            power_recovery_ratio: calibration.recovery_ratio,
            report,
        })
    }

    pub(crate) fn verdict(
        &self,
        candidate: &DiscoveryCandidate,
        params: &DiscoveryChainParams,
    ) -> DiscoveryGateVerdict {
        let prior = reachability_prior_gate(candidate, params);
        if !prior.passed {
            return prior;
        }
        let mut evidence = self.evidence();
        evidence.push(format!("reachability_prior_code={}", prior.code));
        evidence.extend(prior.evidence);
        if !self.report.sufficient {
            return DiscoveryGateVerdict {
                passed: false,
                confidence: 0.0,
                code: "CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY".to_string(),
                reason: "candidate refused because calibrated panel ci_low is below anchor entropy"
                    .to_string(),
                evidence,
            };
        }
        DiscoveryGateVerdict {
            passed: true,
            confidence: self.confidence().min(prior.confidence),
            code: "CALYX_DISCOVERY_SUFFICIENCY_PASS".to_string(),
            reason: "candidate passed calibrated bits-sufficiency gate with reachability prior"
                .to_string(),
            evidence,
        }
    }

    pub(crate) fn summary_json(&self) -> serde_json::Value {
        json!({
            "domain": self.assay_domain,
            "anchor": anchor_kind_label(&self.assay_anchor),
            "panel_version": self.panel_version,
            "panel_bits": self.panel_bits,
            "ci_low": self.ci_low,
            "ci_high": self.ci_high,
            "anchor_entropy_bits": self.anchor_entropy_bits,
            "sufficient": self.report.sufficient,
            "power_calibration": "passed",
            "power_recovery_ratio": self.power_recovery_ratio,
        })
    }

    fn confidence(&self) -> f32 {
        if self.anchor_entropy_bits <= 0.0 {
            return 1.0;
        }
        (self.ci_low / self.anchor_entropy_bits).clamp(0.0, 1.0)
    }

    fn evidence(&self) -> Vec<String> {
        vec![
            format!("assay_domain={}", self.assay_domain),
            format!("assay_anchor={}", anchor_kind_label(&self.assay_anchor)),
            format!("panel_version={}", self.panel_version),
            format!("panel_bits={:.6}", self.panel_bits),
            format!("ci_low={:.6}", self.ci_low),
            format!("ci_high={:.6}", self.ci_high),
            format!("anchor_entropy_bits={:.6}", self.anchor_entropy_bits),
            format!("n_samples={}", self.n_samples),
            "power_calibration=passed".to_string(),
            format!("power_recovery_ratio={:.6}", self.power_recovery_ratio),
        ]
    }
}

fn required_assay_row<'a>(
    store: &'a AssayStore,
    key: &AssayCacheKey,
    subject: &AssaySubject,
) -> Result<&'a AssayRow, LodestarError> {
    store.get(key, subject).ok_or_else(|| {
        no_sufficiency_assay(format!(
            "missing discovery-chain sufficiency assay row for subject {subject:?}"
        ))
    })
}

fn finite_assay_bits(row: &AssayRow, label: &str) -> Result<f32, LodestarError> {
    let bits = row.estimate.bits;
    if bits.is_finite() && bits >= 0.0 {
        Ok(bits)
    } else {
        Err(no_sufficiency_assay(format!(
            "discovery-chain {label} assay bits must be finite and non-negative"
        )))
    }
}

fn no_sufficiency_assay(detail: impl Into<String>) -> LodestarError {
    LodestarError::DiscoveryNoSufficiencyAssay {
        detail: detail.into(),
    }
}

pub(crate) fn parse_nonempty(raw: &str, flag: &str) -> CliResult<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(CliError::usage(format!("{flag} must not be empty")));
    }
    Ok(value.to_string())
}

pub(crate) fn parse_anchor_kind(raw: &str) -> CliResult<AnchorKind> {
    let value = raw.trim();
    if let Some(label) = value.strip_prefix("label:") {
        return Ok(AnchorKind::Label(parse_nonempty(
            label,
            "--assay-anchor label",
        )?));
    }
    Ok(match value {
        "test_pass" | "test-pass" => AnchorKind::TestPass,
        "tie_formed" | "tie-formed" => AnchorKind::TieFormed,
        "thumbs" => AnchorKind::Thumbs,
        "reward" => AnchorKind::Reward,
        "speaker_match" | "speaker-match" => AnchorKind::SpeakerMatch,
        "style_hold" | "style-hold" => AnchorKind::StyleHold,
        "recurrence" => AnchorKind::Recurrence,
        other => {
            return Err(CliError::usage(format!(
                "unknown --assay-anchor {other}; use reward, test_pass, tie_formed, thumbs, label:<name>, speaker_match, style_hold, or recurrence"
            )));
        }
    })
}

pub(crate) fn anchor_kind_label(anchor: &AnchorKind) -> String {
    match anchor {
        AnchorKind::TestPass => "test_pass".to_string(),
        AnchorKind::TieFormed => "tie_formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker_match".to_string(),
        AnchorKind::StyleHold => "style_hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}
