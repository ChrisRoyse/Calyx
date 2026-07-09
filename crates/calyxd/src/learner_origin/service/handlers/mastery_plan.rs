use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, PowerCalibration, TrustTag,
};
use calyx_core::{AnchorKind, Constellation, Panel, SystemClock};
use calyx_oracle::{
    CalibrationMeasurement, DomainId, GoodhartDefenseMeasurement, HeldOutSplit,
    MistakeClosureMeasurement, OracleSelfConsistency, SlotSet,
};
use serde_json::{Value, json};

use crate::learner_origin::model::MasteryEstimateRequest;

use super::super::{OriginError, ensure_nonempty, storage_error};
use super::mastery_support::{
    MASTERY_CLIENT_ATTESTED, MASTERY_MEASUREMENT_PROVENANCE, MasteryCalibrationSource,
    MasteryConcept, MasteryGoodhartSource, MasteryKernelSource, MasteryMistakeSource,
    MasteryOracleSource, MasteryRegion, MasteryTrustGate, MasteryTrustSources,
    build_mastery_concepts, build_mastery_constellation, build_mastery_panel,
};
use super::shared::require_nonnegative_bits;
pub(super) struct MasteryPlan {
    request_id: String,
    pub(super) domain: DomainId,
    pub(super) panel: Panel,
    pub(super) cx: Constellation,
    pub(super) clamp: SlotSet,
    pub(super) free: SlotSet,
    pub(super) region: MasteryRegion,
    pub(super) oracle: OracleSelfConsistency,
    trust_gate: MasteryTrustGate,
    pub(super) held_out: HeldOutSplit,
    pub(super) concepts: Vec<MasteryConcept>,
}

impl MasteryPlan {
    pub(super) fn from_request(
        request: &MasteryEstimateRequest,
        request_id: &str,
        body_hash: &str,
        now: u64,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
    ) -> Result<Self, OriginError> {
        let base_domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-mastery");
        ensure_nonempty("domain", base_domain)?;
        let domain = DomainId::from(format!("{base_domain}:{request_id}"));
        let panel_bits = require_nonnegative_bits("panelBits", request.panel_bits)?;
        let anchor_entropy_bits =
            require_nonnegative_bits("anchorEntropyBits", request.anchor_entropy_bits)?;
        let oracle = request.oracle_self_consistency.to_oracle()?;
        let trust_gate = MasteryTrustGate::from_request(&request.trust_gate)?;
        let concepts = build_mastery_concepts(&request.concepts)?;
        let panel = build_mastery_panel(&concepts, now);
        let input_bytes = serde_json::to_vec(&json!({
            "kind": "mastery_evidence",
            "requestId": request_id,
            "learnerId": request.learner_id,
            "domain": domain.to_string(),
            "concepts": concepts.iter().map(MasteryConcept::input_readback).collect::<Vec<_>>(),
            "payloadSha256": body_hash
        }))
        .map_err(|error| OriginError::internal(error.to_string()))?;
        let cx_id = vault.cx_id_for_input(&input_bytes, panel.version);
        let cx = build_mastery_constellation(super::mastery_support::MasteryConstellationInput {
            cx_id,
            vault,
            request,
            request_id,
            domain: &domain,
            concepts: &concepts,
            input_bytes: &input_bytes,
            body_hash,
            now,
        });
        let clamp = concepts
            .iter()
            .filter(|concept| concept.measured)
            .map(|concept| concept.lens_id)
            .collect::<SlotSet>();
        let free = concepts
            .iter()
            .filter(|concept| !concept.measured)
            .map(|concept| concept.lens_id)
            .collect::<SlotSet>();
        let held_out = trust_gate.held_out_split(request_id, cx_id);
        Ok(Self {
            request_id: request_id.to_string(),
            domain,
            panel,
            cx,
            clamp,
            free,
            region: MasteryRegion::new(&concepts),
            oracle,
            trust_gate: trust_gate.with_sufficiency(panel_bits, anchor_entropy_bits),
            held_out,
            concepts,
        })
    }

    pub(super) fn persist_assay_rows(
        &self,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
        now: u64,
    ) -> Result<usize, OriginError> {
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(
            self.panel.version,
            self.domain.as_str(),
            vault.vault_id(),
            AnchorKind::Reward,
        );
        store.put_with_payload(
            key.clone(),
            AssaySubject::Panel,
            MiEstimate::point(
                self.trust_gate.panel_bits,
                self.trust_gate.sample_count,
                EstimatorKind::PanelSufficiency,
                TrustTag::Provisional,
            )
            .with_power_calibration(self.trust_gate.power_calibration(self.concepts.len())),
            "learner-origin mastery panel sufficiency",
            now,
            self.assay_payload("panel_sufficiency"),
        );
        store.put_with_payload(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            MiEstimate::point(
                self.trust_gate.anchor_entropy_bits,
                self.trust_gate.sample_count,
                EstimatorKind::OutcomeEntropy,
                TrustTag::Provisional,
            ),
            "learner-origin mastery outcome entropy",
            now,
            self.assay_payload("outcome_entropy"),
        );
        let per_slot_bits = if self.concepts.is_empty() {
            0.0
        } else {
            self.trust_gate.panel_bits / self.concepts.len() as f32
        };
        for concept in &self.concepts {
            store.put_with_payload(
                key.clone(),
                AssaySubject::Lens {
                    slot: concept.slot_id,
                },
                MiEstimate::point(
                    per_slot_bits,
                    self.trust_gate.sample_count,
                    EstimatorKind::Ksg,
                    TrustTag::Provisional,
                ),
                format!("learner-origin mastery lens {}", concept.concept_id),
                now,
                self.assay_payload("lens"),
            );
        }
        store.persist_to_vault(vault).map_err(storage_error)
    }

    fn assay_payload(&self, subject: &str) -> Value {
        json!({
            "clientAttested": MASTERY_CLIENT_ATTESTED,
            "measurementProvenance": MASTERY_MEASUREMENT_PROVENANCE,
            "requestId": self.request_id.as_str(),
            "domain": self.domain.to_string(),
            "subject": subject
        })
    }

    pub(super) fn trust_sources(&self) -> MasteryTrustSources {
        MasteryTrustSources {
            oracle: MasteryOracleSource(self.oracle.clone()),
            kernel: MasteryKernelSource {
                ratio: self.trust_gate.kernel_recall_ratio,
            },
            calibration: MasteryCalibrationSource(CalibrationMeasurement {
                stored_profile_far_readback: self.trust_gate.calibration_error,
            }),
            goodhart: MasteryGoodhartSource(GoodhartDefenseMeasurement {
                pass_rate: self.trust_gate.goodhart_pass_rate,
                held_out_count: self.held_out.held_out_count(),
                report_passed: self.trust_gate.goodhart_passed,
                violation_count: self.trust_gate.goodhart_violations,
            }),
            mistakes: MasteryMistakeSource(MistakeClosureMeasurement {
                recurring_mistakes: self.trust_gate.recurring_mistakes,
                replayed_mistakes: self.trust_gate.replayed_mistakes,
            }),
        }
    }

    pub(super) fn slot_readbacks(&self, completion: &calyx_oracle::CompletionResult) -> Vec<Value> {
        completion
            .filled_cx
            .iter()
            .filter_map(|slot| {
                self.concepts
                    .iter()
                    .find(|concept| concept.lens_id == slot.lens_id)
                    .map(|concept| {
                        json!({
                            "conceptId": concept.concept_id,
                            "measured": concept.measured,
                            "tag": slot.tag,
                            "mastery": slot.vector.first().copied().unwrap_or(0.0),
                            "lensId": slot.lens_id,
                            "slotId": concept.slot_id
                        })
                    })
            })
            .collect()
    }
}

impl MasteryTrustGate {
    fn power_calibration(&self, n_features: usize) -> PowerCalibration {
        PowerCalibration::new(1.0, 1.0, 0.50, self.sample_count, n_features.max(1), 0)
            .expect("fixed learner-origin mastery power calibration")
    }
}
