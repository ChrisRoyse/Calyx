use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, PairGain, TrustTag,
    entropy_bits,
};
use calyx_core::{
    AnchorKind, Asymmetry, Input, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey,
    SlotShape, SlotState,
};
use calyx_registry::{
    CapabilityGateDecision, CapabilityGateEvaluation, CapabilityGateThresholds, CommissionRequest,
    ProfileProbe, Registry, commission_lens, evaluate_capability_gate,
    max_panel_pairwise_correlation, profile_slot_with_assay, register_commissioned,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::{FSV_TS, ROWS};
use crate::fsv_io::vault_id;

#[derive(Clone, Copy)]
pub struct Target {
    pub name: &'static str,
    pub model: &'static str,
    pub modality: Modality,
    pub axis: &'static str,
    pub signal_bits: f32,
}

#[derive(Clone)]
pub struct Converted {
    pub target: Target,
    pub lens_id: LensId,
    pub artifact_path: PathBuf,
    pub artifact_blake3: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluatedSlot {
    pub slot_id: SlotId,
    pub evaluation: CapabilityGateEvaluation,
}

pub fn cache_key() -> AssayCacheKey {
    AssayCacheKey::scoped(
        792,
        "embedder-zoo-stage-exit",
        vault_id(),
        AnchorKind::Label("fixture-cross-modal-outcome".to_string()),
    )
}

pub fn convert_register_targets(registry: &mut Registry, artifact_dir: &Path) -> Vec<Converted> {
    targets()
        .into_iter()
        .map(|target| {
            let artifact = commission_lens(
                &CommissionRequest {
                    name: target.name.to_string(),
                    base_model: target.model.to_string(),
                    corpus: corpus_for(target),
                    output_dim: 16,
                    modality: target.modality,
                    axis: Some(target.axis.to_string()),
                },
                &artifact_dir.join(target.axis),
            )
            .unwrap();
            let lens_id = register_commissioned(registry, artifact.clone()).unwrap();
            let bytes = fs::read(&artifact.artifact_path).unwrap();
            Converted {
                target,
                lens_id,
                artifact_path: artifact.artifact_path,
                artifact_blake3: blake3::hash(&bytes).to_hex().to_string(),
            }
        })
        .collect()
}

pub fn target_slots(converted: &[Converted]) -> Vec<Slot> {
    converted
        .iter()
        .enumerate()
        .map(|(index, converted)| slot_for(converted, index as u16))
        .collect()
}

pub fn duplicate_slot(source: &Slot, id: u16, key: &str) -> Slot {
    let mut duplicate = source.clone();
    let slot_id = SlotId::new(id);
    duplicate.slot_id = slot_id;
    duplicate.slot_key = SlotKey::new(slot_id, key);
    duplicate.axis = Some(key.to_string());
    duplicate
}

pub fn assay_store(
    cache_key: &AssayCacheKey,
    slots: &[Slot],
    converted: &[Converted],
    pair_gain: &PairGain,
    labels: &[bool],
) -> AssayStore {
    let mut store = AssayStore::default();
    for (slot, converted) in slots.iter().take(converted.len()).zip(converted) {
        put_signal(&mut store, cache_key, slot, converted.target.signal_bits);
    }
    put_signal(
        &mut store,
        cache_key,
        slots.last().unwrap(),
        converted[0].target.signal_bits,
    );
    store.put(
        cache_key.clone(),
        AssaySubject::Panel,
        MiEstimate::new(
            pair_gain.pair_bits,
            pair_gain.ci_low,
            pair_gain.ci_high,
            pair_gain.n_samples,
            EstimatorKind::LogisticProbe,
            TrustTag::Trusted,
        ),
        "issue792 admitted panel bits over grounded fixture outcome",
        792_300,
    );
    store.put(
        cache_key.clone(),
        AssaySubject::OutcomeEntropy,
        MiEstimate::point(
            entropy_bits(labels),
            labels.len(),
            EstimatorKind::OutcomeEntropy,
            TrustTag::Trusted,
        ),
        "issue792 fixture outcome entropy",
        792_301,
    );
    store
}

pub fn evaluate_all(
    registry: &Registry,
    slots: &[Slot],
    assay: &AssayStore,
    cache_key: &AssayCacheKey,
    duplicate_slot: &Slot,
    thresholds: CapabilityGateThresholds,
) -> Vec<EvaluatedSlot> {
    let mut out = Vec::new();
    for slot in slots.iter().take(slots.len() - 1) {
        let card =
            profile_slot_with_assay(registry, slot, &probes_for(slot.modality), assay, cache_key)
                .unwrap();
        out.push(EvaluatedSlot {
            slot_id: slot.slot_id,
            evaluation: evaluate_capability_gate(card, 0.20, thresholds).unwrap(),
        });
    }
    let baseline = Panel {
        version: 1,
        slots: vec![slots[0].clone()],
        created_at: FSV_TS,
        kernel_ref: None,
        guard_ref: None,
    };
    let duplicate_corr = max_panel_pairwise_correlation(
        registry,
        &baseline,
        duplicate_slot.lens_id,
        None,
        &probes_for(Modality::Text),
    )
    .unwrap();
    let duplicate_card = profile_slot_with_assay(
        registry,
        duplicate_slot,
        &probes_for(Modality::Text),
        assay,
        cache_key,
    )
    .unwrap();
    out.push(EvaluatedSlot {
        slot_id: duplicate_slot.slot_id,
        evaluation: evaluate_capability_gate(duplicate_card, duplicate_corr, thresholds).unwrap(),
    });
    out
}

pub fn conversion_readback(converted: &[Converted]) -> Value {
    json!({
        "converted": converted.iter().map(|entry| json!({
            "name": entry.target.name,
            "model": entry.target.model,
            "modality": format!("{:?}", entry.target.modality).to_ascii_lowercase(),
            "axis": entry.target.axis,
            "lens_id": entry.lens_id,
            "artifact_path": entry.artifact_path,
            "artifact_blake3": entry.artifact_blake3
        })).collect::<Vec<_>>()
    })
}

pub fn modality_counts(converted: &[Converted]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for entry in converted {
        *counts
            .entry(format!("{:?}", entry.target.modality).to_ascii_lowercase())
            .or_default() += 1;
    }
    counts
}

pub fn decision_counts(evals: &[EvaluatedSlot]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::from([("admit", 0), ("park", 0), ("retire", 0)]);
    for eval in evals {
        let key = match eval.evaluation.decision {
            CapabilityGateDecision::Admit => "admit",
            CapabilityGateDecision::Park => "park",
            CapabilityGateDecision::Retire => "retire",
        };
        *counts.get_mut(key).unwrap() += 1;
    }
    counts
}

fn targets() -> Vec<Target> {
    vec![
        target(
            "text_semantic",
            "BAAI/bge-small-en-v1.5",
            Modality::Text,
            0.18,
        ),
        target(
            "domain_text",
            "allenai/scibert_scivocab_uncased",
            Modality::Text,
            0.12,
        ),
        target(
            "image_visual",
            "google/siglip2-base-patch16-224",
            Modality::Image,
            0.16,
        ),
        target(
            "audio_event",
            "laion/clap-htsat-unfused",
            Modality::Audio,
            0.15,
        ),
        target(
            "audio_speech",
            "Xenova/wav2vec2-base-960h",
            Modality::Audio,
            0.01,
        ),
        target(
            "protein_sequence",
            "facebook/esm2_t6_8M_UR50D",
            Modality::Protein,
            0.17,
        ),
        target(
            "dna_sequence",
            "zhihan1996/DNABERT-2-117M",
            Modality::Dna,
            0.11,
        ),
        target(
            "molecule_smiles",
            "seyonec/ChemBERTa-zinc-base-v1",
            Modality::Molecule,
            0.10,
        ),
    ]
}

fn target(axis: &'static str, model: &'static str, modality: Modality, signal_bits: f32) -> Target {
    Target {
        name: axis,
        model,
        modality,
        axis,
        signal_bits,
    }
}

fn slot_for(converted: &Converted, slot_id: u16) -> Slot {
    let slot_id = SlotId::new(slot_id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, converted.target.axis),
        lens_id: converted.lens_id,
        shape: SlotShape::Dense(16),
        modality: converted.target.modality,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::turboquant_default(),
        resource: Default::default(),
        axis: Some(converted.target.axis.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn put_signal(store: &mut AssayStore, cache_key: &AssayCacheKey, slot: &Slot, bits: f32) {
    store.put(
        cache_key.clone(),
        AssaySubject::Lens { slot: slot.slot_id },
        MiEstimate::point(bits, ROWS, EstimatorKind::LogisticProbe, TrustTag::Trusted),
        format!(
            "issue792 grounded capability signal for {}",
            slot.slot_key.key()
        ),
        792_100 + u64::from(slot.slot_id.get()),
    );
}

fn probes_for(modality: Modality) -> Vec<ProfileProbe> {
    (0..4)
        .map(|idx| {
            ProfileProbe::labeled(
                Input::new(
                    modality,
                    format!("issue792-{modality:?}-{idx}").into_bytes(),
                ),
                if idx % 2 == 0 { "positive" } else { "negative" },
            )
        })
        .collect()
}

fn corpus_for(target: Target) -> Vec<Vec<u8>> {
    (0..4)
        .map(|idx| format!("{}:{}:{idx}", target.model, target.axis).into_bytes())
        .collect()
}
