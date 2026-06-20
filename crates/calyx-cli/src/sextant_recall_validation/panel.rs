use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{
    AnchorKind, ConfidenceInterval, LensCost, LensId, Modality, Panel, Placement, Signal, Slot,
    SlotId, SlotKey, SlotResource, SlotShape, SlotState,
};
use calyx_registry::{
    FastembedQwen3Lens, FrozenLensContract, LensRuntime, LensSpec, OnnxColbertLens, OnnxLens,
    Registry, StaticLookupLens, TeiHttpLens, lens_spec_from_manifest_path,
};
use serde::{Deserialize, Serialize};

use super::request::RecallRequest;

const DEFAULT_LENS_CATALOG: &str = "/var/lib/calyx/lenses/registry.json";
const REAL_PANEL_VERSION: u32 = 727;

#[derive(Clone)]
pub(crate) struct RealPanel {
    pub(crate) panel: Panel,
    pub(crate) registry: Registry,
    pub(crate) slots: Vec<RealPanelSlot>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RealPanelSlot {
    pub(crate) slot: SlotId,
    pub(crate) lens: String,
    pub(crate) lens_id: LensId,
    pub(crate) signal_bits: f32,
    pub(crate) weight: f32,
    pub(crate) placement: Placement,
}

#[derive(Deserialize)]
struct PackedPanel {
    selected: Vec<PackedLens>,
}

#[derive(Clone, Deserialize)]
struct PackedLens {
    lens: String,
    signal_bits: f32,
    usage: PackedUsage,
    placement: Placement,
}

#[derive(Clone, Deserialize)]
struct PackedUsage {
    vram_mb: f32,
    ram_mb: f32,
    ms_per_input: f32,
}

#[derive(Deserialize)]
struct LensCatalog {
    lenses: Vec<CatalogLens>,
}

#[derive(Clone, Deserialize)]
struct CatalogLens {
    name: String,
    manifest: PathBuf,
}

pub(crate) fn load_real_panel(request: &RecallRequest) -> Result<RealPanel, String> {
    let packed_path = request
        .packed_panel_json
        .as_ref()
        .ok_or_else(|| "CALYX_FSV_SEXTANT_PANEL_REQUIRED".to_string())?;
    let packed = read_json::<PackedPanel>(packed_path)?;
    if packed.selected.is_empty() {
        return Err("CALYX_FSV_SEXTANT_EMPTY_PANEL".to_string());
    }
    let mut names = BTreeSet::new();
    for selected in &packed.selected {
        if selected.lens.trim().is_empty() || !names.insert(selected.lens.clone()) {
            return Err(format!(
                "CALYX_FSV_SEXTANT_PANEL_INVALID: duplicate or empty lens `{}`",
                selected.lens
            ));
        }
        if !selected.signal_bits.is_finite() || selected.signal_bits <= 0.0 {
            return Err(format!(
                "CALYX_FSV_SEXTANT_PANEL_INVALID: lens {} has non-positive signal_bits",
                selected.lens
            ));
        }
    }
    let catalog_path = request
        .lens_catalog
        .clone()
        .or_else(|| env::var("CALYX_LENS_CATALOG").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LENS_CATALOG));
    let catalog = read_json::<LensCatalog>(&catalog_path)?;
    let manifests = catalog
        .lenses
        .into_iter()
        .map(|lens| (lens.name, lens.manifest))
        .collect::<BTreeMap<_, _>>();

    let mut registry = Registry::new();
    let mut slots = Vec::new();
    let mut panel_slots = Vec::new();
    for (idx, selected) in packed.selected.iter().enumerate() {
        let manifest = manifests.get(&selected.lens).ok_or_else(|| {
            format!(
                "CALYX_FSV_SEXTANT_PANEL_LENS_MISSING: {} not found in {}",
                selected.lens,
                catalog_path.display()
            )
        })?;
        let spec = lens_spec_from_manifest_path(manifest).map_err(|error| error.to_string())?;
        if spec.modality != Modality::Text {
            return Err(format!(
                "CALYX_FSV_SEXTANT_PANEL_INVALID: {} is {:?}, expected text",
                spec.name, spec.modality
            ));
        }
        let lens_id = register_lens(&mut registry, spec.clone())?;
        let slot_id = SlotId::new(idx as u16);
        let signal = signal(selected.signal_bits);
        panel_slots.push(Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, selected.lens.clone()),
            lens_id,
            shape: spec.output,
            modality: spec.modality,
            asymmetry: spec.asymmetry,
            quant: spec.quant_default,
            resource: SlotResource {
                cost: cost(&selected.usage),
                placement: selected.placement,
            },
            axis: spec.axis.clone().or_else(|| Some(selected.lens.clone())),
            retrieval_only: spec.retrieval_only,
            excluded_from_dedup: spec.excluded_from_dedup,
            bits_about: BTreeMap::from([(AnchorKind::Label("assay_panel".to_string()), signal)]),
            state: SlotState::Active,
            added_at_panel_version: REAL_PANEL_VERSION,
        });
        slots.push(RealPanelSlot {
            slot: slot_id,
            lens: selected.lens.clone(),
            lens_id,
            signal_bits: selected.signal_bits,
            weight: selected.signal_bits,
            placement: selected.placement,
        });
    }
    Ok(RealPanel {
        panel: Panel {
            version: REAL_PANEL_VERSION,
            slots: panel_slots,
            created_at: 0,
            kernel_ref: None,
            guard_ref: None,
        },
        registry,
        slots,
    })
}

fn register_lens(registry: &mut Registry, spec: LensSpec) -> Result<LensId, String> {
    match &spec.runtime {
        LensRuntime::Onnx { .. } => {
            let lens = OnnxLens::from_lens_spec(&spec).map_err(|error| error.to_string())?;
            let contract = lens.contract().clone();
            registry
                .register_frozen_with_spec(lens, contract, spec)
                .map_err(|error| error.to_string())
        }
        LensRuntime::OnnxColbert { .. } => {
            let lens = OnnxColbertLens::from_lens_spec(&spec).map_err(|error| error.to_string())?;
            let contract = lens.contract().clone();
            registry
                .register_frozen_with_spec(lens, contract, spec)
                .map_err(|error| error.to_string())
        }
        LensRuntime::StaticLookup { .. } => {
            let lens =
                StaticLookupLens::from_lens_spec(&spec).map_err(|error| error.to_string())?;
            let contract = lens.contract().clone();
            registry
                .register_frozen_with_spec(lens, contract, spec)
                .map_err(|error| error.to_string())
        }
        LensRuntime::FastembedQwen3 { .. } => {
            let lens =
                FastembedQwen3Lens::from_lens_spec(&spec).map_err(|error| error.to_string())?;
            let contract = lens.contract().clone();
            registry
                .register_frozen_with_spec(lens, contract, spec)
                .map_err(|error| error.to_string())
        }
        LensRuntime::TeiHttp { endpoint } => {
            let dim = dense_dim(spec.output)?;
            let contract = FrozenLensContract::tei_http(&spec.name, endpoint, spec.modality, dim);
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim);
            registry
                .register_frozen_with_spec(lens, contract, spec)
                .map_err(|error| error.to_string())
        }
        other => Err(format!(
            "CALYX_FSV_SEXTANT_PANEL_RUNTIME_UNSUPPORTED: {:?}",
            other
        )),
    }
}

fn dense_dim(shape: SlotShape) -> Result<u32, String> {
    match shape {
        SlotShape::Dense(dim) => Ok(dim),
        other => Err(format!(
            "CALYX_FSV_SEXTANT_PANEL_INVALID: TEI requires dense output, got {:?}",
            other
        )),
    }
}

fn cost(usage: &PackedUsage) -> LensCost {
    LensCost {
        total_ms: usage.ms_per_input,
        ms_per_input: usage.ms_per_input,
        vram_bytes: mb_to_bytes(usage.vram_mb),
        ram_bytes: mb_to_bytes(usage.ram_mb),
        batch_ceiling: u32::MAX,
    }
}

fn mb_to_bytes(value: f32) -> u64 {
    (value.max(0.0) as f64 * 1024.0 * 1024.0).round() as u64
}

fn signal(bits: f32) -> Signal {
    Signal {
        bits,
        ci: ConfidenceInterval {
            low: bits,
            high: bits,
        },
        n: 0,
        estimator: "assay_packed_panel".to_string(),
        ts: 0,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}
