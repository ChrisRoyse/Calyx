use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, DEFAULT_TE_BOOTSTRAP_RESAMPLES,
    DEFAULT_TE_BOOTSTRAP_SEED, DEFAULT_TE_K, DEFAULT_TE_LAGS, DEFAULT_TE_WINDOW, EstimatorKind,
    MiEstimate, PowerCalibration, TEResult, TransferEntropyConfig, TrustTag,
    transfer_entropy_sweep_with_config,
};
use calyx_core::AnchorValue;
use calyx_core::{AnchorKind, Clock, Constellation, SystemClock, content_address};
use calyx_oracle::{Action, Consequence, DomainId, Prediction};
use serde_json::Value;

use crate::learner_origin::model::{OracleForecastRequest, TransferEntropyRequest};

use super::super::{OriginError, ensure_nonempty, storage_error};
use super::oracle_graph::{
    OracleGraphRows, build_oracle_graph_rows, build_oracle_panel,
    build_oracle_source_constellation, prerequisite_edges, transfer_entropy_stream,
    validate_anchor_value,
};
use super::shared::require_nonnegative_bits;
pub(super) struct OracleForecastPlan {
    pub(super) domain: DomainId,
    pub(super) action: Action,
    pub(super) source_cx: Constellation,
    pub(super) graph_rows: OracleGraphRows,
    pub(super) graph_base_count: usize,
    pub(super) recurrence_count: usize,
    pub(super) reverse_answer: AnchorValue,
    pub(super) desired_outcome: Option<AnchorValue>,
    transfer_entropy: TransferEntropyJob,
}

impl OracleForecastPlan {
    pub(super) fn from_request(
        request: &OracleForecastRequest,
        request_id: &str,
        body_hash: &str,
        now: u64,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
    ) -> Result<Self, OriginError> {
        let base_domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-oracle");
        ensure_nonempty("domain", base_domain)?;
        let domain = DomainId::from(base_domain.to_string());
        let panel_bits = require_nonnegative_bits("panelBits", request.panel_bits)?;
        let anchor_entropy_bits =
            require_nonnegative_bits("anchorEntropyBits", request.anchor_entropy_bits)?;
        let panel = build_oracle_panel(request, now)?;
        let action = Action {
            action_id: request.action_id.clone(),
            panel: panel.clone(),
            guard: None,
        };
        validate_anchor_value("reverseAnswer", &request.reverse_answer)?;
        if let Some(desired) = &request.desired_outcome {
            validate_anchor_value("desiredOutcome", desired)?;
        }
        let source_cx =
            build_oracle_source_constellation(vault, request, request_id, &domain, body_hash, now)?;
        let (graph_rows, graph_base_count, recurrence_count) =
            build_oracle_graph_rows(vault, request, request_id, &domain, body_hash, now)?;
        let transfer_entropy =
            TransferEntropyJob::from_request(&request.transfer_entropy, request_id)?;
        Ok(Self {
            domain,
            action,
            source_cx,
            graph_rows,
            graph_base_count,
            recurrence_count,
            reverse_answer: request.reverse_answer.clone(),
            desired_outcome: request.desired_outcome.clone(),
            transfer_entropy: transfer_entropy.with_sufficiency(panel_bits, anchor_entropy_bits),
        })
    }

    pub(super) fn persist_assay_rows(
        &self,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
        now: u64,
    ) -> Result<usize, OriginError> {
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(
            self.action.panel.version,
            self.domain.as_str(),
            vault.vault_id(),
            AnchorKind::Reward,
        );
        store.put(
            key.clone(),
            AssaySubject::Panel,
            MiEstimate::point(
                self.transfer_entropy.panel_bits,
                self.transfer_entropy.sample_count,
                EstimatorKind::PanelSufficiency,
                TrustTag::Trusted,
            )
            .with_power_calibration(
                self.transfer_entropy
                    .power_calibration(self.action.panel.slots.len()),
            ),
            "learner-origin oracle forecast panel sufficiency",
            now,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            MiEstimate::point(
                self.transfer_entropy.anchor_entropy_bits,
                self.transfer_entropy.sample_count,
                EstimatorKind::OutcomeEntropy,
                TrustTag::Trusted,
            ),
            "learner-origin oracle forecast outcome entropy",
            now,
        );
        let per_slot_bits = if self.action.panel.slots.is_empty() {
            0.0
        } else {
            self.transfer_entropy.panel_bits / self.action.panel.slots.len() as f32
        };
        for slot in &self.action.panel.slots {
            store.put(
                key.clone(),
                AssaySubject::Lens { slot: slot.slot_id },
                MiEstimate::point(
                    per_slot_bits,
                    self.transfer_entropy.sample_count,
                    EstimatorKind::Ksg,
                    TrustTag::Trusted,
                ),
                format!("learner-origin oracle forecast slot {}", slot.slot_id.get()),
                now,
            );
        }
        store.persist_to_vault(vault).map_err(storage_error)
    }

    pub(super) fn transfer_entropy_readback(
        &self,
        clock: &dyn Clock,
    ) -> Result<TransferEntropyReadback, OriginError> {
        self.transfer_entropy.readback(clock)
    }

    pub(super) fn root_consequence(&self, prediction: &Prediction) -> Consequence {
        Consequence {
            action_or_event: self.action.action_id.clone(),
            domain: self.domain.clone(),
            outcome: prediction.outcome.clone(),
            confidence: prediction.confidence,
            hop: 0,
            provenance: prediction.provenance.clone(),
        }
    }
}

#[derive(Clone)]
struct TransferEntropyJob {
    source_concept_id: String,
    target_concept_id: String,
    source_series: Vec<(u64, f32)>,
    target_series: Vec<(u64, f32)>,
    lags: Vec<usize>,
    config: TransferEntropyConfig,
    panel_bits: f32,
    anchor_entropy_bits: f32,
    sample_count: usize,
}

impl TransferEntropyJob {
    fn from_request(
        request: &TransferEntropyRequest,
        request_id: &str,
    ) -> Result<Self, OriginError> {
        ensure_nonempty(
            "transferEntropy.sourceConceptId",
            &request.source_concept_id,
        )?;
        ensure_nonempty(
            "transferEntropy.targetConceptId",
            &request.target_concept_id,
        )?;
        let source_series =
            transfer_entropy_stream("transferEntropy.sourceSeries", &request.source_series)?;
        let target_series =
            transfer_entropy_stream("transferEntropy.targetSeries", &request.target_series)?;
        let lags = if request.lags.is_empty() {
            DEFAULT_TE_LAGS.to_vec()
        } else {
            request.lags.clone()
        };
        if lags.contains(&0) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TRANSFER_ENTROPY",
                "transferEntropy.lags must be positive",
            ));
        }
        let config = TransferEntropyConfig {
            window_size: request.window_size.unwrap_or(DEFAULT_TE_WINDOW),
            k: request.k.unwrap_or(DEFAULT_TE_K),
            bootstrap_resamples: request
                .bootstrap_resamples
                .unwrap_or(DEFAULT_TE_BOOTSTRAP_RESAMPLES),
            bootstrap_seed: request.bootstrap_seed.unwrap_or_else(|| {
                let digest = content_address([
                    b"oracle-forecast-transfer-entropy".as_slice(),
                    request_id.as_bytes(),
                ]);
                u64::from_be_bytes(digest[..8].try_into().expect("digest slice is u64"))
                    ^ DEFAULT_TE_BOOTSTRAP_SEED
            }),
        };
        if config.window_size == 0 || config.k == 0 || config.bootstrap_resamples == 0 {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TRANSFER_ENTROPY",
                "transferEntropy windowSize, k, and bootstrapResamples must be positive",
            ));
        }
        let sample_count = source_series.len().min(target_series.len()).max(1);
        Ok(Self {
            source_concept_id: request.source_concept_id.clone(),
            target_concept_id: request.target_concept_id.clone(),
            source_series,
            target_series,
            lags,
            config,
            panel_bits: 0.0,
            anchor_entropy_bits: 0.0,
            sample_count,
        })
    }

    fn with_sufficiency(mut self, panel_bits: f32, anchor_entropy_bits: f32) -> Self {
        self.panel_bits = panel_bits;
        self.anchor_entropy_bits = anchor_entropy_bits;
        self
    }

    fn power_calibration(&self, n_features: usize) -> PowerCalibration {
        PowerCalibration::new(1.0, 1.0, 0.50, self.sample_count, n_features.max(1), 0)
            .expect("fixed learner-origin oracle power calibration")
    }

    fn readback(&self, clock: &dyn Clock) -> Result<TransferEntropyReadback, OriginError> {
        let results = transfer_entropy_sweep_with_config(
            &self.source_series,
            &self.target_series,
            &self.lags,
            clock,
            &self.config,
        );
        let max_lag = calyx_assay::max_transfer_entropy_lag(&results);
        let prereq_edges =
            prerequisite_edges(&self.source_concept_id, &self.target_concept_id, &results);
        Ok(TransferEntropyReadback {
            source_concept_id: self.source_concept_id.clone(),
            target_concept_id: self.target_concept_id.clone(),
            results,
            max_lag,
            prereq_edges,
        })
    }
}

pub(super) struct TransferEntropyReadback {
    pub(super) source_concept_id: String,
    pub(super) target_concept_id: String,
    pub(super) results: Vec<TEResult>,
    pub(super) max_lag: Option<usize>,
    pub(super) prereq_edges: Vec<Value>,
}
