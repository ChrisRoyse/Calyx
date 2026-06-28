use std::collections::BTreeMap;

use calyx_assay::{
    ChangePointReport, MmdConfig, MmdReport, gaussian_mmd_with_config, mmd_change_point,
};
use calyx_aster::cf::{ColumnFamily, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SystemClock,
    content_address,
};
use calyx_ward::{GuardId, GuardPolicy, GuardProfile, NoveltyAction};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::learner_origin::model::{ReactiveAffectRequest, ReactiveMmdRequest};

use super::super::{
    OriginError, ensure_nonempty, hex, insert_optional, sha256_array, storage_error,
};
use super::reactive_support::{mmd_origin_error, reactive_vector, vector_bytes};
use super::shared::require_unit_interval;
use super::{
    REACTIVE_AFFECT_BASELINE_KIND, REACTIVE_AFFECT_DEFAULT_SLOT_ID, REACTIVE_AFFECT_EVIDENCE_KIND,
    REACTIVE_AFFECT_MATCHED_KIND, REACTIVE_AFFECT_MAX_SLOT_ID, REACTIVE_AFFECT_MIN_MMD_WINDOW,
    REACTIVE_AFFECT_PANEL_VERSION,
};

pub(super) type ReactiveRows = Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>;

pub(super) struct ReactiveAffectPlan {
    pub(super) domain: String,
    pub(super) slot: SlotId,
    pub(super) tau: f32,
    pub(super) drift_threshold: f32,
    pub(super) profile: GuardProfile,
    pub(super) matched_cx: Constellation,
    pub(super) baseline_cx: Constellation,
    pub(super) current_cx: Constellation,
    pub(super) recurrence_rows: ReactiveRows,
    pub(super) mmd: ReactiveMmdJob,
}

impl ReactiveAffectPlan {
    pub(super) fn from_request(
        request: &ReactiveAffectRequest,
        request_id: &str,
        body_hash: &str,
        now: u64,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
    ) -> Result<Self, OriginError> {
        let domain = request
            .domain
            .clone()
            .unwrap_or_else(|| "calyxweb-reactive-affect".to_string());
        ensure_nonempty("domain", &domain)?;
        let slot_number = request.slot_id.unwrap_or(REACTIVE_AFFECT_DEFAULT_SLOT_ID);
        if slot_number > REACTIVE_AFFECT_MAX_SLOT_ID {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_SLOT",
                format!("slotId must be <= {REACTIVE_AFFECT_MAX_SLOT_ID} for reactive slots"),
            ));
        }
        let slot = SlotId::new(slot_number);
        let tau = require_unit_interval("tau", request.tau.unwrap_or(calyx_ward::DEFAULT_TAU))?;
        let drift_threshold = require_reactive_drift_threshold(request.drift_threshold)?;
        let matched_vector = reactive_vector("matchedVector", &request.matched_vector)?;
        let baseline_vector = reactive_vector("baselineVector", &request.baseline_vector)?;
        let current_vector = reactive_vector("currentVector", &request.current_vector)?;
        if matched_vector.len() != baseline_vector.len()
            || matched_vector.len() != current_vector.len()
        {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_VECTOR_DIM_MISMATCH",
                "matchedVector, baselineVector, and currentVector must have the same dimension",
            ));
        }
        let current_frequency = request.recurrence.current_occurrences_secs.len().max(1) as u64;
        let matched_cx = build_reactive_constellation(ReactiveConstellationInput {
            vault,
            kind: REACTIVE_AFFECT_MATCHED_KIND,
            role: "matched",
            role_code: 1.0,
            request,
            request_id,
            domain: &domain,
            body_hash,
            now,
            slot,
            vector: matched_vector,
            frequency: request.recurrence.known_pattern_frequency,
        })?;
        let baseline_cx = build_reactive_constellation(ReactiveConstellationInput {
            vault,
            kind: REACTIVE_AFFECT_BASELINE_KIND,
            role: "baseline",
            role_code: 2.0,
            request,
            request_id,
            domain: &domain,
            body_hash,
            now,
            slot,
            vector: baseline_vector,
            frequency: current_frequency,
        })?;
        let current_cx = build_reactive_constellation(ReactiveConstellationInput {
            vault,
            kind: REACTIVE_AFFECT_EVIDENCE_KIND,
            role: "current",
            role_code: 3.0,
            request,
            request_id,
            domain: &domain,
            body_hash,
            now,
            slot,
            vector: current_vector,
            frequency: current_frequency,
        })?;
        let recurrence_rows =
            build_reactive_recurrence_rows(current_cx.cx_id, request, request_id, now)?;
        let profile = build_reactive_guard_profile(request_id, &domain, slot, tau, now);
        let mmd = ReactiveMmdJob::from_request(&request.mmd);
        Ok(Self {
            domain,
            slot,
            tau,
            drift_threshold,
            profile,
            matched_cx,
            baseline_cx,
            current_cx,
            recurrence_rows,
            mmd,
        })
    }
}

struct ReactiveConstellationInput<'a> {
    vault: &'a calyx_aster::vault::AsterVault<SystemClock>,
    kind: &'static str,
    role: &'static str,
    role_code: f64,
    request: &'a ReactiveAffectRequest,
    request_id: &'a str,
    domain: &'a str,
    body_hash: &'a str,
    now: u64,
    slot: SlotId,
    vector: Vec<f32>,
    frequency: u64,
}

fn build_reactive_constellation(
    input: ReactiveConstellationInput<'_>,
) -> Result<Constellation, OriginError> {
    let vector_hash = sha256_array(&vector_bytes(&input.vector));
    let input_bytes = serde_json::to_vec(&json!({
        "kind": input.kind,
        "role": input.role,
        "requestId": input.request_id,
        "learnerId": input.request.learner_id,
        "domain": input.domain,
        "conceptId": input.request.concept_id,
        "slotId": input.slot.get(),
        "vectorHash": hex(&vector_hash),
        "payloadSha256": input.body_hash
    }))
    .map_err(|error| OriginError::internal(error.to_string()))?;
    let cx_id = input
        .vault
        .cx_id_for_input(&input_bytes, REACTIVE_AFFECT_PANEL_VERSION);
    let mut metadata = BTreeMap::from([
        ("origin_kind".to_string(), input.kind.to_string()),
        ("origin_version".to_string(), "1".to_string()),
        ("request_id".to_string(), input.request_id.to_string()),
        ("learner_id".to_string(), input.request.learner_id.clone()),
        ("domain".to_string(), input.domain.to_string()),
        ("concept_id".to_string(), input.request.concept_id.clone()),
        ("role".to_string(), input.role.to_string()),
        ("slot_id".to_string(), input.slot.get().to_string()),
        ("payload_sha256".to_string(), input.body_hash.to_string()),
        ("vector_sha256".to_string(), hex(&vector_hash)),
    ]);
    insert_optional(
        &mut metadata,
        "idempotency_key",
        input.request.idempotency_key.as_deref(),
    );
    insert_optional(
        &mut metadata,
        "session_id",
        input.request.session_id.as_deref(),
    );
    insert_optional(
        &mut metadata,
        "privacy_class",
        input.request.privacy_class.as_deref(),
    );
    Ok(Constellation {
        cx_id,
        vault_id: input.vault.vault_id(),
        panel_version: REACTIVE_AFFECT_PANEL_VERSION,
        created_at: input.now,
        input_ref: InputRef {
            hash: sha256_array(&input_bytes),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Structured,
        slots: BTreeMap::from([(
            input.slot,
            SlotVector::Dense {
                dim: input.vector.len() as u32,
                data: input.vector,
            },
        )]),
        scalars: BTreeMap::from([
            (FREQUENCY_SCALAR.to_string(), input.frequency as f64),
            ("reactive.role_code".to_string(), input.role_code),
        ]),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            redacted_input: true,
            ungrounded: true,
            ..CxFlags::default()
        },
    })
}

fn build_reactive_recurrence_rows(
    current_cx: CxId,
    request: &ReactiveAffectRequest,
    request_id: &str,
    now: u64,
) -> Result<ReactiveRows, OriginError> {
    let timestamps = if request.recurrence.current_occurrences_secs.is_empty() {
        vec![now / 1000]
    } else {
        request.recurrence.current_occurrences_secs.clone()
    };
    let mut rows = Vec::with_capacity(timestamps.len());
    for (index, timestamp) in timestamps.iter().enumerate() {
        let t_k = i64::try_from(*timestamp).map_err(|_| {
            OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TIMESTAMP",
                "recurrence timestamp is outside i64 epoch seconds range",
            )
        })?;
        let occurrence = Occurrence {
            id: OccurrenceId(index as u64),
            t_k: EpochSecs(t_k),
            context: OccurrenceContext::new(
                format!("reactive-affect:{request_id}:{index}").into_bytes(),
            )
            .map_err(storage_error)?,
        };
        rows.push((
            ColumnFamily::Recurrence,
            recurrence_key(current_cx, occurrence.id.0),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                .map_err(storage_error)?,
        ));
    }
    Ok(rows)
}

fn build_reactive_guard_profile(
    request_id: &str,
    domain: &str,
    slot: SlotId,
    tau: f32,
    now: u64,
) -> GuardProfile {
    let mut uuid_bytes = content_address([
        b"reactive-affect-guard".as_slice(),
        request_id.as_bytes(),
        domain.as_bytes(),
        &slot.get().to_be_bytes(),
    ]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    GuardProfile {
        guard_id: GuardId::new(Uuid::from_bytes(uuid_bytes)),
        panel_version: REACTIVE_AFFECT_PANEL_VERSION as u64,
        domain: domain.to_string(),
        tau: BTreeMap::from([(slot, tau)]),
        required_slots: vec![slot],
        policy: GuardPolicy::AllRequired,
        calibration: Some(calyx_ward::CalibrationMeta::new(
            sha256_array(format!("reactive-affect-calibration:{request_id}:{domain}").as_bytes()),
            "learner-origin-reactive-affect",
            0.0,
            0.0,
            0.99,
            &calyx_core::FixedClock::new(now),
        )),
        novelty_action: NoveltyAction::NewRegion,
    }
}

#[derive(Clone)]
pub(super) struct ReactiveMmdJob {
    baseline: Vec<Vec<f64>>,
    recent: Vec<Vec<f64>>,
    stream: Vec<Vec<f64>>,
    min_window: usize,
    config: MmdConfig,
}

impl ReactiveMmdJob {
    fn from_request(request: &ReactiveMmdRequest) -> Self {
        let mut config = MmdConfig::default();
        if let Some(bandwidth) = request.bandwidth {
            config.bandwidth = Some(bandwidth);
        }
        if let Some(permutations) = request.permutations {
            config.permutations = permutations;
        }
        if let Some(seed) = request.seed {
            config.seed = seed;
        }
        if let Some(alpha) = request.alpha {
            config.alpha = alpha;
        }
        let stream = if request.change_point_stream.is_empty() {
            request
                .baseline_samples
                .iter()
                .chain(request.recent_samples.iter())
                .cloned()
                .collect()
        } else {
            request.change_point_stream.clone()
        };
        let min_window = request.min_window.unwrap_or_else(|| {
            request
                .baseline_samples
                .len()
                .min(request.recent_samples.len())
                .max(REACTIVE_AFFECT_MIN_MMD_WINDOW)
        });
        Self {
            baseline: request.baseline_samples.clone(),
            recent: request.recent_samples.clone(),
            stream,
            min_window,
            config,
        }
    }

    pub(super) fn readback(&self) -> Result<ReactiveMmdReadback, OriginError> {
        let drift = gaussian_mmd_with_config(&self.baseline, &self.recent, &self.config)
            .map_err(mmd_origin_error)?;
        let change_point = mmd_change_point(&self.stream, self.min_window, &self.config)
            .map_err(mmd_origin_error)?;
        Ok(ReactiveMmdReadback {
            drift,
            change_point,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReactiveMmdReadback {
    pub(super) drift: MmdReport,
    pub(super) change_point: ChangePointReport,
}

fn require_reactive_drift_threshold(value: f32) -> Result<f32, OriginError> {
    if value.is_finite() && (0.0..=2.0).contains(&value) {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            "driftThreshold must be finite and within [0, 2]",
        ))
    }
}
