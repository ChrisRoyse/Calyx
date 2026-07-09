use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{
    AbsentReason, Asymmetry, Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotShape, SlotState, SlotVector, SystemClock,
    content_address,
};
use calyx_lodestar::{LodestarError, RecallReport};
use calyx_oracle::{
    AnnealConfig, CalibrationMeasurement, CalibrationSource, CompletionRegion, DomainId,
    GoodhartDefenseMeasurement, GoodhartDefenseSource, HeldOutSplit, KernelRecallSource,
    MistakeClosureMeasurement, MistakeClosureSource, OracleConsistencySource, OracleError,
    OracleSelfConsistency,
};
use serde_json::{Value, json};

use crate::learner_origin::model::{
    MasteryConceptRequest, MasteryEstimateRequest, MasteryTrustGateRequest,
};

use super::super::{OriginError, ensure_nonempty, sha256_array};
use super::shared::require_unit_interval;

pub(super) const MASTERY_CLIENT_ATTESTED: bool = true;
pub(super) const MASTERY_MEASUREMENT_PROVENANCE: &str = "client_attested";
pub(super) const MASTERY_CERTIFICATION_BLOCKED_REASON: &str = "client_attested_metrics";

#[derive(Clone)]
pub(super) struct MasteryConcept {
    pub(super) concept_id: String,
    pub(super) slot_id: SlotId,
    pub(super) lens_id: LensId,
    pub(super) measured: bool,
    pub(super) mastery: f32,
    pub(super) trusted_mastery: f32,
}

impl MasteryConcept {
    pub(super) fn input_readback(&self) -> Value {
        json!({
            "conceptId": self.concept_id,
            "measured": self.measured,
            "mastery": self.mastery,
            "trustedMastery": self.trusted_mastery
        })
    }
}

#[derive(Clone)]
pub(super) struct MasteryTrustGate {
    pub(super) panel_bits: f32,
    pub(super) anchor_entropy_bits: f32,
    pub(super) sample_count: usize,
    pub(super) held_out_count: usize,
    pub(super) kernel_recall_ratio: f32,
    pub(super) calibration_error: f32,
    pub(super) goodhart_pass_rate: f32,
    pub(super) goodhart_passed: bool,
    pub(super) goodhart_violations: usize,
    pub(super) recurring_mistakes: usize,
    pub(super) replayed_mistakes: usize,
}

impl MasteryTrustGate {
    pub(super) fn from_request(request: &MasteryTrustGateRequest) -> Result<Self, OriginError> {
        let held_out_count = request.held_out_count;
        Ok(Self {
            panel_bits: 0.0,
            anchor_entropy_bits: 0.0,
            sample_count: held_out_count.max(1),
            held_out_count,
            kernel_recall_ratio: require_unit_interval(
                "trustGate.kernelRecallRatio",
                request.kernel_recall_ratio,
            )?,
            calibration_error: require_unit_interval(
                "trustGate.calibrationError",
                request.calibration_error,
            )?,
            goodhart_pass_rate: require_unit_interval(
                "trustGate.goodhartPassRate",
                request.goodhart_pass_rate,
            )?,
            goodhart_passed: request
                .goodhart_passed
                .unwrap_or(request.goodhart_pass_rate >= calyx_oracle::GOODHART_THRESHOLD),
            goodhart_violations: request.goodhart_violations.unwrap_or(0),
            recurring_mistakes: request.recurring_mistakes,
            replayed_mistakes: request
                .replayed_mistakes
                .unwrap_or(request.recurring_mistakes),
        })
    }

    pub(super) fn with_sufficiency(mut self, panel_bits: f32, anchor_entropy_bits: f32) -> Self {
        self.panel_bits = panel_bits;
        self.anchor_entropy_bits = anchor_entropy_bits;
        self
    }

    pub(super) fn held_out_split(&self, request_id: &str, source_cx: CxId) -> HeldOutSplit {
        let held_out_ids = (0..self.held_out_count)
            .map(|index| {
                CxId::from_bytes(content_address([
                    b"mastery-held-out".as_slice(),
                    request_id.as_bytes(),
                    &index.to_be_bytes(),
                ]))
            })
            .collect();
        HeldOutSplit::new(
            format!("mastery-estimate:{request_id}"),
            vec![source_cx],
            held_out_ids,
        )
    }
}

pub(super) struct MasteryTrustSources {
    pub(super) oracle: MasteryOracleSource,
    pub(super) kernel: MasteryKernelSource,
    pub(super) calibration: MasteryCalibrationSource,
    pub(super) goodhart: MasteryGoodhartSource,
    pub(super) mistakes: MasteryMistakeSource,
}

#[derive(Clone)]
pub(super) struct MasteryOracleSource(pub(super) OracleSelfConsistency);

impl OracleConsistencySource for MasteryOracleSource {
    fn oracle_self_consistency(
        &self,
        _domain: DomainId,
        _clock: &dyn Clock,
    ) -> Result<OracleSelfConsistency, OracleError> {
        Ok(self.0.clone())
    }
}

pub(super) struct MasteryKernelSource {
    pub(super) ratio: f32,
}

impl KernelRecallSource for MasteryKernelSource {
    fn kernel_recall_report(
        &self,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, LodestarError> {
        Ok(RecallReport {
            kernel_only: self.ratio,
            full: 1.0,
            ratio: self.ratio,
            approx_factor: 1.0,
            tau_star_estimate: held_out.held_out_count(),
            tau_star_exact: false,
            recall_test_params: None,
            corpus_name: Some("client-attested:calyxweb-learner-mastery".to_string()),
            n_queries_tested: held_out.held_out_count(),
            held_out: held_out.held_out_ids.clone(),
            warning: Some(MASTERY_CERTIFICATION_BLOCKED_REASON.to_string()),
        })
    }
}

pub(super) struct MasteryCalibrationSource(pub(super) CalibrationMeasurement);

impl CalibrationSource for MasteryCalibrationSource {
    fn calibration_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

pub(super) struct MasteryGoodhartSource(pub(super) GoodhartDefenseMeasurement);

impl GoodhartDefenseSource for MasteryGoodhartSource {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

pub(super) struct MasteryMistakeSource(pub(super) MistakeClosureMeasurement);

impl MistakeClosureSource for MasteryMistakeSource {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
pub(super) struct MasteryRegion {
    members: BTreeMap<LensId, Vec<Vec<f32>>>,
}

impl MasteryRegion {
    pub(super) fn new(concepts: &[MasteryConcept]) -> Self {
        let members = concepts
            .iter()
            .map(|concept| (concept.lens_id, vec![vec![concept.trusted_mastery]]))
            .collect();
        Self { members }
    }
}

impl CompletionRegion for MasteryRegion {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        Ok(self.members.get(&lens_id).cloned().unwrap_or_default())
    }
}

pub(super) struct MasteryAnneal;

impl AnnealConfig for MasteryAnneal {
    fn energy_beta(&self, _domain: &DomainId) -> Option<f32> {
        Some(1.0)
    }
}

impl crate::learner_origin::model::OracleSelfConsistencyRequest {
    pub(super) fn to_oracle(&self) -> Result<OracleSelfConsistency, OriginError> {
        let flakiness = require_unit_interval("oracleSelfConsistency.flakiness", self.flakiness)?;
        let validity = require_unit_interval("oracleSelfConsistency.validity", self.validity)?;
        Ok(OracleSelfConsistency::with_provenance(
            flakiness,
            validity,
            self.provisional,
            None,
        ))
    }
}

pub(super) fn build_mastery_concepts(
    inputs: &[MasteryConceptRequest],
) -> Result<Vec<MasteryConcept>, OriginError> {
    if inputs.is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            "concepts must contain at least one measured and one un-probed concept",
        ));
    }
    if inputs.len() > 256 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_TOO_MANY_CONCEPTS",
            "mastery estimate accepts at most 256 concepts",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut measured_count = 0_usize;
    let mut free_count = 0_usize;
    let mut out = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        ensure_nonempty("conceptId", &input.concept_id)?;
        if !seen.insert(input.concept_id.as_str()) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_DUPLICATE_CONCEPT",
                format!("duplicate conceptId {}", input.concept_id),
            ));
        }
        let measured = input.mastery.is_some();
        let mastery = match input.mastery {
            Some(value) => {
                measured_count += 1;
                require_unit_interval("concept.mastery", value)?
            }
            None => {
                free_count += 1;
                0.0
            }
        };
        let trusted_mastery = match input.trusted_mastery {
            Some(value) => require_unit_interval("concept.trustedMastery", value)?,
            None if measured => mastery,
            None => {
                return Err(OriginError::bad_request(
                    "CALYX_ORIGIN_FIELD_REQUIRED",
                    format!(
                        "un-probed concept {} requires trustedMastery",
                        input.concept_id
                    ),
                ));
            }
        };
        let slot_id = SlotId::new((index + 1) as u16);
        out.push(MasteryConcept {
            concept_id: input.concept_id.clone(),
            slot_id,
            lens_id: LensId::from_bytes(content_address([
                b"calyxweb-mastery-lens".as_slice(),
                input.concept_id.as_bytes(),
                &(index as u64).to_be_bytes(),
            ])),
            measured,
            mastery,
            trusted_mastery,
        });
    }
    if measured_count == 0 || free_count == 0 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            "mastery imputation requires at least one measured concept and one un-probed concept",
        ));
    }
    Ok(out)
}

pub(super) fn build_mastery_panel(concepts: &[MasteryConcept], now: u64) -> Panel {
    Panel {
        version: 1247,
        slots: concepts
            .iter()
            .map(|concept| Slot {
                slot_id: concept.slot_id,
                slot_key: concept
                    .slot_id
                    .with_key(format!("mastery-{}", concept.slot_id.get())),
                lens_id: concept.lens_id,
                shape: SlotShape::Dense(1),
                modality: Modality::Structured,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: Default::default(),
                axis: Some(format!("mastery:{}", concept.concept_id)),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about: BTreeMap::new(),
                state: SlotState::Active,
                added_at_panel_version: 1247,
            })
            .collect(),
        created_at: now,
        kernel_ref: None,
        guard_ref: None,
    }
}

pub(super) struct MasteryConstellationInput<'a> {
    pub(super) vault: &'a calyx_aster::vault::AsterVault<SystemClock>,
    pub(super) cx_id: CxId,
    pub(super) request: &'a MasteryEstimateRequest,
    pub(super) request_id: &'a str,
    pub(super) domain: &'a DomainId,
    pub(super) concepts: &'a [MasteryConcept],
    pub(super) input_bytes: &'a [u8],
    pub(super) body_hash: &'a str,
    pub(super) now: u64,
}

pub(super) fn build_mastery_constellation(input: MasteryConstellationInput<'_>) -> Constellation {
    let slots = input
        .concepts
        .iter()
        .map(|concept| {
            let vector = if concept.measured {
                SlotVector::Dense {
                    dim: 1,
                    data: vec![concept.mastery],
                }
            } else {
                SlotVector::Absent {
                    reason: AbsentReason::Deferred,
                }
            };
            (concept.slot_id, vector)
        })
        .collect();
    let metadata = BTreeMap::from([
        ("origin_kind".to_string(), "mastery_evidence".to_string()),
        ("origin_version".to_string(), "1".to_string()),
        ("payload_sha256".to_string(), input.body_hash.to_string()),
        ("request_id".to_string(), input.request_id.to_string()),
        ("learner_id".to_string(), input.request.learner_id.clone()),
        ("domain".to_string(), input.domain.to_string()),
        (
            "client_attested".to_string(),
            MASTERY_CLIENT_ATTESTED.to_string(),
        ),
        (
            "measurement_provenance".to_string(),
            MASTERY_MEASUREMENT_PROVENANCE.to_string(),
        ),
        (
            "concept_count".to_string(),
            input.concepts.len().to_string(),
        ),
    ]);
    Constellation {
        cx_id: input.cx_id,
        vault_id: input.vault.vault_id(),
        panel_version: 1247,
        created_at: input.now,
        input_ref: InputRef {
            hash: sha256_array(input.input_bytes),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Structured,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            redacted_input: true,
            ..CxFlags::default()
        },
    }
}
