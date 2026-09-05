//! Calyx Registry materialization for Poly's v1 embedder-free signal panel (#39).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_core::{
    CalyxError, Input, Lens, LensId, Modality, Panel, QuantPolicy, Result as CalyxResult, Slot,
    SlotId, SlotKey, SlotShape, SlotState, SlotVector,
};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{DeterminismProof, FrozenLensContract, LensDType, NormPolicy};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::lenses::{SignalLens, default_category_vocab, default_panel};
use crate::model::MarketSnapshot;
use crate::question_bm25_lens::{QUESTION_BM25_DIM, QUESTION_BM25_KEY};
use crate::seed_registry::{SEED_REGISTRY_VERSION, seed_spec_for_lens};
use crate::temporal_lens::{
    E2_RECENCY_KEY, E3_PERIODIC_KEY, E4_POSITIONAL_KEY, is_temporal_lens_key,
};

pub const POLY_PANEL_REGISTRY_SCHEMA_VERSION: &str = "poly.panel_registry.v1";
pub const POLY_PANEL_REGISTRY_ARTIFACT_KIND: &str = "poly_v1_embedder_free_panel_registry";
pub const POLY_PANEL_REGISTRY_FILE: &str = "poly_panel_registry_v1.json";

pub const ERR_PANEL_REGISTRY_INVALID: &str = "CALYX_POLY_PANEL_REGISTRY_INVALID";
pub const ERR_PANEL_REGISTRY_READBACK_MISMATCH: &str =
    "CALYX_POLY_PANEL_REGISTRY_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyPanelRegistrySlot {
    pub slot_id: SlotId,
    pub key: String,
    pub lens_id: LensId,
    pub shape: SlotShape,
    pub contract: FrozenLensContract,
    pub determinism: DeterminismProof,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyPanelRegistrySnapshot {
    pub schema_version: String,
    pub artifact_kind: String,
    pub panel_version: u32,
    pub region_vocab: Vec<String>,
    pub slot_count: usize,
    pub registered_lens_count: usize,
    pub determinism_probe_count: usize,
    pub panel: Panel,
    pub slots: Vec<PolyPanelRegistrySlot>,
}

#[derive(Clone)]
pub struct PolyPanelRegistryMaterialization {
    pub panel: Panel,
    pub registry: calyx_registry::Registry,
    pub snapshot: PolyPanelRegistrySnapshot,
}

pub fn materialize_poly_v1_panel_registry(
    panel_version: u32,
    region_vocab: Vec<String>,
    created_at: u64,
    probe_snapshot: &MarketSnapshot,
) -> Result<PolyPanelRegistryMaterialization> {
    if panel_version == 0 {
        return invalid("panel version must be non-zero");
    }
    let probe = market_snapshot_input(probe_snapshot)?;
    let signal_panel = default_panel(panel_version, region_vocab.clone());
    if signal_panel.lenses.is_empty() {
        return invalid("default Poly panel produced no lenses");
    }

    let mut registry = calyx_registry::Registry::new();
    let mut slots = Vec::with_capacity(signal_panel.lenses.len());
    let mut readback_slots = Vec::with_capacity(signal_panel.lenses.len());
    let mut seen_slots = BTreeSet::new();
    let mut seen_keys = BTreeSet::new();

    for lens in signal_panel.lenses {
        let slot_id = lens.slot();
        let key = lens.key().to_string();
        let shape = lens.shape();
        if !seen_slots.insert(slot_id) {
            return invalid(format!("duplicate Poly panel slot id {slot_id}"));
        }
        if !seen_keys.insert(key.clone()) {
            return invalid(format!("duplicate Poly panel lens key {key}"));
        }

        let contract = poly_lens_contract(panel_version, &region_vocab, &key, shape)?;
        let runtime_lens = PolySignalRegistryLens {
            lens,
            contract: contract.clone(),
        };
        let lens_id =
            registry.register_frozen_with_probe(runtime_lens, contract.clone(), &probe)?;
        let determinism = registry.determinism_proof(lens_id).ok_or_else(|| {
            PolyError::diagnostics(
                ERR_PANEL_REGISTRY_INVALID,
                format!("registry did not record determinism proof for lens {lens_id}"),
            )
        })?;
        if determinism != DeterminismProof::ProbeVerified {
            return invalid(format!(
                "lens {key} registered without deterministic probe proof"
            ));
        }

        slots.push(panel_slot(slot_id, &key, lens_id, shape, panel_version));
        readback_slots.push(PolyPanelRegistrySlot {
            slot_id,
            key,
            lens_id,
            shape,
            contract,
            determinism,
        });
    }

    let panel = Panel {
        version: panel_version,
        slots,
        created_at,
        kernel_ref: None,
        guard_ref: None,
    };
    let snapshot = PolyPanelRegistrySnapshot {
        schema_version: POLY_PANEL_REGISTRY_SCHEMA_VERSION.to_string(),
        artifact_kind: POLY_PANEL_REGISTRY_ARTIFACT_KIND.to_string(),
        panel_version,
        region_vocab,
        slot_count: panel.slots.len(),
        registered_lens_count: registry.lens_snapshots().len(),
        determinism_probe_count: readback_slots.len(),
        panel: panel.clone(),
        slots: readback_slots,
    };
    validate_poly_panel_registry_snapshot(&snapshot)?;
    Ok(PolyPanelRegistryMaterialization {
        panel,
        registry,
        snapshot,
    })
}

pub fn measure_registered_poly_panel(
    registry: &calyx_registry::Registry,
    panel: &Panel,
    snapshot: &MarketSnapshot,
) -> Result<BTreeMap<SlotId, SlotVector>> {
    let input = market_snapshot_input(snapshot)?;
    let mut measured = BTreeMap::new();
    for slot in &panel.slots {
        measured.insert(slot.slot_id, registry.measure(slot.lens_id, &input)?);
    }
    Ok(measured)
}

pub fn write_poly_panel_registry_snapshot(
    dir: &Path,
    snapshot: &PolyPanelRegistrySnapshot,
) -> Result<PathBuf> {
    validate_poly_panel_registry_snapshot(snapshot)?;
    write_json(dir, POLY_PANEL_REGISTRY_FILE, snapshot)
}

pub fn read_poly_panel_registry_snapshot(path: &Path) -> Result<PolyPanelRegistrySnapshot> {
    let snapshot = read_json(path)?;
    validate_poly_panel_registry_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn validate_poly_panel_registry_snapshot(snapshot: &PolyPanelRegistrySnapshot) -> Result<()> {
    if snapshot.schema_version != POLY_PANEL_REGISTRY_SCHEMA_VERSION
        || snapshot.artifact_kind != POLY_PANEL_REGISTRY_ARTIFACT_KIND
        || snapshot.panel_version == 0
    {
        return invalid("unexpected panel registry schema, artifact kind, or panel version");
    }
    if snapshot.slot_count != snapshot.panel.slots.len()
        || snapshot.slot_count != snapshot.slots.len()
        || snapshot.registered_lens_count != snapshot.slot_count
        || snapshot.determinism_probe_count != snapshot.slot_count
    {
        return invalid("panel registry counts do not match physical slots");
    }
    let mut seen_slots = BTreeSet::new();
    let mut seen_keys = BTreeSet::new();
    for (panel_slot, registry_slot) in snapshot.panel.slots.iter().zip(&snapshot.slots) {
        if !seen_slots.insert(registry_slot.slot_id) {
            return invalid(format!(
                "duplicate registry slot id {} in persisted panel",
                registry_slot.slot_id
            ));
        }
        if !seen_keys.insert(registry_slot.key.clone()) {
            return invalid(format!(
                "duplicate registry slot key {} in persisted panel",
                registry_slot.key
            ));
        }
        if panel_slot.slot_id != registry_slot.slot_id
            || panel_slot.slot_key.key() != registry_slot.key
            || panel_slot.lens_id != registry_slot.lens_id
            || panel_slot.shape != registry_slot.shape
        {
            return invalid(format!(
                "panel slot {} does not match registry slot {}",
                panel_slot.slot_id, registry_slot.key
            ));
        }
        if registry_slot.contract.lens_id() != registry_slot.lens_id {
            return invalid(format!(
                "slot {} lens id does not match frozen contract",
                registry_slot.key
            ));
        }
        if registry_slot.determinism != DeterminismProof::ProbeVerified {
            return invalid(format!(
                "slot {} lacks deterministic probe proof",
                registry_slot.key
            ));
        }
    }
    Ok(())
}

fn market_snapshot_input(snapshot: &MarketSnapshot) -> Result<Input> {
    Ok(Input::new(
        Modality::Structured,
        snapshot.canonical_input_bytes()?,
    ))
}

fn panel_slot(
    slot_id: SlotId,
    key: &str,
    lens_id: LensId,
    shape: SlotShape,
    panel_version: u32,
) -> Slot {
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, key.to_string()),
        lens_id,
        shape,
        modality: Modality::Structured,
        asymmetry: calyx_core::Asymmetry::None,
        quant: QuantPolicy::turboquant_default(),
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only: is_temporal_lens_key(key),
        excluded_from_dedup: is_temporal_lens_key(key),
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: panel_version,
    }
}

fn poly_lens_contract(
    panel_version: u32,
    region_vocab: &[String],
    key: &str,
    shape: SlotShape,
) -> Result<FrozenLensContract> {
    let descriptor = lens_descriptor(key, shape, region_vocab)?;
    let version = panel_version.to_be_bytes();
    let weights = sha256_digest(&[
        b"poly-v1-embedder-free-panel",
        version.as_slice(),
        key.as_bytes(),
        descriptor.as_bytes(),
    ]);
    let corpus = sha256_digest(&[
        b"poly-v1-panel-axis",
        key.as_bytes(),
        axis_descriptor(key, region_vocab).as_bytes(),
    ]);
    Ok(FrozenLensContract::new(
        key,
        weights,
        corpus,
        shape,
        Modality::Structured,
        LensDType::F32,
        NormPolicy::Finite,
    ))
}

fn lens_descriptor(key: &str, shape: SlotShape, region_vocab: &[String]) -> Result<String> {
    if let Ok(spec) = seed_spec_for_lens(key) {
        return Ok(format!(
            "{SEED_REGISTRY_VERSION}:slot={}:seed={}:dim={}:sigma={}:field={}:transform={}",
            spec.slot,
            spec.seed_hex(),
            spec.dim,
            spec.sigma,
            spec.source_field,
            spec.transform
        ));
    }
    Ok(match key {
        "logvol_ple" => {
            "quantile_ple:field=volume_24h:transform=signed_log:edges=0,2,4,6,8,10,12,14"
                .to_string()
        }
        "liquidity_ple" => {
            "quantile_ple:field=liquidity:transform=signed_log:edges=0,2,4,6,8,10,12".to_string()
        }
        "category_oh" => format!(
            "one_hot:category:vocab={}",
            default_category_vocab().join("|")
        ),
        "region_oh" => format!("one_hot:region:vocab={}", region_vocab.join("|")),
        "holders_membership" => "sparse_holders:dim=1048576:hash=blake3:weight=share".to_string(),
        E2_RECENCY_KEY => "calyx_registry:E2_recency:exponential_half_life_secs=86400:reference=per_snapshot_temporal_reference_ts:retrieval_only:excluded_from_dedup".to_string(),
        E3_PERIODIC_KEY => "calyx_registry:E3_periodic:use_now_reference=temporal_reference_ts:features=hour_score,day_of_week_score:retrieval_only:excluded_from_dedup".to_string(),
        E4_POSITIONAL_KEY => "calyx_registry:E4_positional:source=sequence_position,total:direction=both:retrieval_only:excluded_from_dedup".to_string(),
        QUESTION_BM25_KEY => format!(
            "sparse_question_bm25:dim={QUESTION_BM25_DIM}:source=question,tags:tokenizer=calyx_sextant_lowercase_punct:hash=blake3:tf=log_l2"
        ),
        "book_shape" => "dense_book_shape:levels=5:normalized_l2:features=best_bid,best_ask,spread,imbalance,cumulative_bid_depths,cumulative_ask_depths".to_string(),
        "toxicity" => "dense_vpin_toxicity:source=onchain_fills:volume_buckets=3:normalized_l2:features=vpin,signed_imbalance,log_total_volume,largest_fill_share,bucket_coverage".to_string(),
        other => {
            return invalid(format!(
                "unknown default Poly panel lens key {other} with shape {shape:?}"
            ));
        }
    })
}

fn axis_descriptor(key: &str, region_vocab: &[String]) -> String {
    match key {
        "region_oh" => region_vocab.join("|"),
        "category_oh" => default_category_vocab().join("|"),
        _ => "data-oblivious-axis".to_string(),
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PANEL_REGISTRY_INVALID,
        message.into(),
    ))
}

trait SeedHex {
    fn seed_hex(&self) -> String;
}

impl SeedHex for crate::seed_registry::FrozenRffSeedSpec {
    fn seed_hex(&self) -> String {
        format!("0x{:016X}", self.seed)
    }
}

struct PolySignalRegistryLens {
    lens: Box<dyn SignalLens>,
    contract: FrozenLensContract,
}

impl Lens for PolySignalRegistryLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        self.contract.shape()
    }

    fn modality(&self) -> Modality {
        Modality::Structured
    }

    fn measure(&self, input: &Input) -> CalyxResult<SlotVector> {
        if input.modality != Modality::Structured {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "Poly signal lens {} accepts structured input, got {:?}",
                self.contract.name(),
                input.modality
            )));
        }
        let snapshot: MarketSnapshot = serde_json::from_slice(&input.bytes).map_err(|err| {
            CalyxError::lens_unreachable(format!(
                "Poly signal lens {} could not decode MarketSnapshot input: {err}",
                self.contract.name()
            ))
        })?;
        Ok(self.lens.measure(&snapshot))
    }
}
