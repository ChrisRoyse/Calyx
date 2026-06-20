use std::collections::BTreeMap;

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EnsembleCard, EnsembleConfig, EnsembleLensInput,
    EstimatorKind, MiEstimate, TrustTag, ensemble_card,
};
use calyx_aster::cf::CfRouter;
use calyx_core::{AnchorKind, SlotId, VaultId};
use serde::Serialize;
use ulid::Ulid;

use crate::assay_bits_validation::calyx_error_detail;
use crate::assay_bits_validation::data::AssayCorpus;
use crate::assay_bits_validation::request::AssayBitsRequest;

use super::request::EnsembleCardRequest;

const PANEL_VERSION: u32 = 799;
const CF_MEMTABLE_CAP: usize = 1_048_576;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EnsembleCardReport {
    pub(crate) dataset: String,
    pub(crate) embedding_model_id: String,
    pub(crate) domain: String,
    pub(crate) target_class: usize,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
    pub(crate) assay_cf_subject_counts: BTreeMap<String, usize>,
    pub(crate) ensemble_card_row_present: bool,
    pub(crate) ensemble_card_payload_readback: bool,
    pub(crate) card: EnsembleCard,
}

pub(crate) fn evaluate(request: &EnsembleCardRequest) -> Result<EnsembleCardReport, String> {
    let corpus_request = AssayBitsRequest {
        corpus_dir: request.corpus_dir.clone(),
        metrics_dir: request.metrics_dir.clone(),
        cf_root: request.cf_root.clone(),
        min_bits: request.min_marginal_bits,
        max_corr: request.max_redundancy,
        target_class: request.target_class,
        domain: request.domain.clone(),
        cost_json: None,
        panel_budget_json: None,
    };
    let corpus = AssayCorpus::load(&corpus_request)?;
    let labels = corpus.anchor_labels(request.target_class);
    let config = EnsembleConfig {
        source: format!(
            "assay ensemble-card dataset={} model={} domain={}",
            corpus.dataset, corpus.embedding_model_id, request.domain
        ),
        min_gate_lenses: request.min_lenses,
        min_marginal_bits: request.min_marginal_bits,
        max_redundancy: request.max_redundancy,
        nmi_bins: 10,
    };
    let lenses = ensemble_lenses(&corpus)?;
    let card = ensemble_card(&lenses, &labels, Some(&corpus.anchor_groups), &config)
        .map_err(calyx_error_detail)?;
    let persistence = persist_and_readback(request, &card)?;
    Ok(EnsembleCardReport {
        dataset: corpus.dataset,
        embedding_model_id: corpus.embedding_model_id,
        domain: request.domain.clone(),
        target_class: request.target_class,
        cf_root: request.cf_root.display().to_string(),
        assay_cf_rows_persisted: persistence.persisted,
        assay_cf_rows_readback: persistence.readback,
        assay_cf_subject_counts: persistence.subject_counts,
        ensemble_card_row_present: persistence.card_row_present,
        ensemble_card_payload_readback: persistence.card_payload_readback,
        card,
    })
}

fn ensemble_lenses(corpus: &AssayCorpus) -> Result<Vec<EnsembleLensInput>, String> {
    corpus
        .lenses
        .iter()
        .enumerate()
        .map(|(idx, lens)| {
            let slot = SlotId::new(
                u16::try_from(idx)
                    .map_err(|_| "CALYX_FSV_ASSAY_INVALID_CORPUS: too many lenses".to_string())?,
            );
            Ok(EnsembleLensInput::new(
                lens.name.clone(),
                slot,
                corpus.lens_vectors[idx].clone(),
            ))
        })
        .collect()
}

struct PersistenceReadback {
    persisted: usize,
    readback: usize,
    subject_counts: BTreeMap<String, usize>,
    card_row_present: bool,
    card_payload_readback: bool,
}

fn persist_and_readback(
    request: &EnsembleCardRequest,
    card: &EnsembleCard,
) -> Result<PersistenceReadback, String> {
    let key = cache_key(request);
    let mut store = AssayStore::default();
    for (idx, lens) in card.lenses.iter().enumerate() {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: lens.slot },
            estimate(
                lens.solo_bits,
                lens.solo_ci,
                card.n_samples,
                EstimatorKind::LogisticProbe,
            ),
            format!("assay ensemble-card lens={}", lens.name),
            idx as u64,
        );
    }
    for (idx, pair) in card.pairs.iter().enumerate() {
        store.put(
            key.clone(),
            AssaySubject::Pair {
                a: pair.slot_a,
                b: pair.slot_b,
            },
            estimate(
                pair.synergy_gain_bits,
                pair.pair_ci,
                card.n_samples,
                EstimatorKind::PairGain,
            ),
            format!("assay ensemble-card pair={}+{}", pair.a, pair.b),
            1_000 + idx as u64,
        );
    }
    store.put(
        key.clone(),
        AssaySubject::Panel,
        panel_estimate(card, EstimatorKind::LogisticProbe),
        "assay ensemble-card panel",
        2_000,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        MiEstimate::point(
            card.anchor_entropy_bits,
            card.n_samples,
            EstimatorKind::OutcomeEntropy,
            TrustTag::Provisional,
        ),
        "assay ensemble-card anchor entropy",
        2_001,
    );
    let payload = serde_json::to_value(card).map_err(|error| error.to_string())?;
    store.put_with_payload(
        key.clone(),
        AssaySubject::EnsembleCard,
        panel_estimate(card, EstimatorKind::PanelSufficiency),
        "assay ensemble-card payload",
        2_002,
        payload,
    );

    let mut router =
        CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let persisted = store
        .persist_to_aster(&mut router)
        .map_err(calyx_error_detail)?;
    drop(router);
    let reopened = CfRouter::open(&request.cf_root, CF_MEMTABLE_CAP).map_err(calyx_error_detail)?;
    let loaded = AssayStore::load_from_aster(&reopened).map_err(calyx_error_detail)?;
    let row = loaded.get(&key, &AssaySubject::EnsembleCard);
    let card_row_present = row.is_some();
    let card_payload_readback = match row.and_then(|row| row.payload.clone()) {
        Some(payload) => {
            let readback: EnsembleCard = serde_json::from_value(payload)
                .map_err(|error| format!("CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH: {error}"))?;
            if readback != *card {
                return Err(
                    "CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH: EnsembleCard payload changed after Assay CF readback"
                        .to_string(),
                );
            }
            true
        }
        None => {
            return Err(
                "CALYX_FSV_ASSAY_CARD_READBACK_MISSING: EnsembleCard payload absent from Assay CF"
                    .to_string(),
            );
        }
    };
    let rows = loaded.rows();
    Ok(PersistenceReadback {
        persisted,
        readback: rows.len(),
        subject_counts: subject_counts(&rows),
        card_row_present,
        card_payload_readback,
    })
}

fn estimate(bits: f32, ci: [f32; 2], n: usize, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::new(bits, ci[0], ci[1], n, estimator, TrustTag::Provisional)
}

fn panel_estimate(card: &EnsembleCard, estimator: EstimatorKind) -> MiEstimate {
    let mut estimate = estimate(card.panel_bits, card.panel_ci, card.n_samples, estimator)
        .with_bound(card.sufficiency.estimate_bound);
    if let Some(calibration) = card.sufficiency.power_calibration.clone() {
        estimate = estimate.with_power_calibration(calibration);
    }
    estimate
}

fn subject_counts(rows: &[calyx_assay::AssayRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = match &row.subject {
            AssaySubject::Lens { .. } => "lens",
            AssaySubject::Pair { .. } => "pair",
            AssaySubject::Panel => "panel",
            AssaySubject::OutcomeEntropy => "outcome_entropy",
            AssaySubject::EnsembleCard => "ensemble_card",
        };
        *counts.entry(key.to_string()).or_insert(0) += 1;
    }
    counts
}

fn cache_key(request: &EnsembleCardRequest) -> AssayCacheKey {
    AssayCacheKey::scoped(
        PANEL_VERSION,
        request.domain.clone(),
        deterministic_vault_id(&request.domain),
        AnchorKind::Label(format!("target_class_{}", request.target_class)),
    )
}

fn deterministic_vault_id(domain: &str) -> VaultId {
    let digest = blake3::hash(domain.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    VaultId::from_ulid(Ulid::from_bytes(bytes))
}
