use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, AnchorValue, CalyxError, Constellation, CxId, Panel, Slot, SlotId, SlotState,
    SlotVector, VaultStore,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use serde::Serialize;

use crate::server::{ToolError, ToolResult};
use crate::tools::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

pub(super) struct VaultContext {
    pub(super) vault: AsterVault,
    pub(super) state: VaultPanelState,
    pub(super) vault_dir: PathBuf,
}

pub(super) fn load_context(vault_name: &str) -> ToolResult<VaultContext> {
    let resolved = resolve_requested_vault(vault_name)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    Ok(VaultContext {
        vault,
        state,
        vault_dir: resolved.path,
    })
}

fn resolve_requested_vault(vault: &str) -> ToolResult<ResolvedVault> {
    let home = home_dir()?;
    resolve_vault_info(&home, vault)
}

fn open_vault(resolved: &ResolvedVault) -> ToolResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

pub(super) fn load_docs(vault: &AsterVault) -> ToolResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
            CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
        })?;
        let cx_id = CxId::from_bytes(bytes);
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

pub(super) fn parse_anchor(raw: &str) -> ToolResult<AnchorKind> {
    Ok(match raw {
        "test_pass" | "test-pass" => AnchorKind::TestPass,
        "thumbs" | "thumbs_up" | "thumbs-up" | "thumbs_down" | "thumbs-down" => AnchorKind::Thumbs,
        "speaker_match" | "speaker-match" => AnchorKind::SpeakerMatch,
        "style_hold" | "style-hold" => AnchorKind::StyleHold,
        "reward" => AnchorKind::Reward,
        "recurrence" => AnchorKind::Recurrence,
        value if value.starts_with("label:") && value.len() > "label:".len() => {
            AnchorKind::Label(value["label:".len()..].to_string())
        }
        other => {
            return Err(ToolError::invalid_params(format!(
                "unknown anchor kind {other}"
            )));
        }
    })
}

pub(super) fn anchor_label(kind: &AnchorKind) -> String {
    match kind {
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

pub(super) fn active_slots(panel: &Panel) -> Vec<&Slot> {
    panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
        .collect()
}

pub(super) fn active_slot_ids(panel: &Panel) -> Vec<SlotId> {
    active_slots(panel)
        .into_iter()
        .map(|slot| slot.slot_id)
        .collect()
}

pub(super) fn has_anchor(cx: &Constellation, kind: &AnchorKind) -> bool {
    cx.anchors
        .iter()
        .any(|anchor| &anchor.kind == kind && anchor_value_truthy(&anchor.value))
}

pub(super) fn has_anchor_kind(cx: &Constellation, kind: &AnchorKind) -> bool {
    cx.anchors.iter().any(|anchor| &anchor.kind == kind)
}

fn anchor_value_truthy(value: &AnchorValue) -> bool {
    match value {
        AnchorValue::Bool(value) => *value,
        AnchorValue::Number(value) => value.is_finite() && *value > 0.0,
        AnchorValue::Enum(value) | AnchorValue::Text(value) => !value.trim().is_empty(),
        AnchorValue::OneHot(values) => !values.is_empty(),
        AnchorValue::Vector(values) => !values.is_empty(),
    }
}

pub(super) fn has_any_anchor(cx: &Constellation, kind: Option<&AnchorKind>) -> bool {
    cx.anchors
        .iter()
        .any(|anchor| kind.is_none_or(|expected| &anchor.kind == expected))
}

pub(super) fn parse_cx_id(value: &str, field: &str) -> ToolResult<CxId> {
    value
        .parse::<CxId>()
        .map_err(|err| ToolError::invalid_params(format!("parse {field} {value}: {err}")))
}

pub(super) fn dense(cx: &Constellation, slot: SlotId) -> Option<&[f32]> {
    cx.slots.get(&slot).and_then(SlotVector::as_dense)
}

pub(super) fn dense_dim(cx: &Constellation, slots: &[SlotId]) -> Option<usize> {
    slots
        .iter()
        .find_map(|slot| dense(cx, *slot).map(|values| values.len()))
}

pub(super) fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let (mut dot, mut l2, mut r2) = (0.0f32, 0.0f32, 0.0f32);
    for (l, r) in left.iter().zip(right) {
        dot += l * r;
        l2 += l * l;
        r2 += r * r;
    }
    (l2 > 0.0 && r2 > 0.0).then(|| dot / (l2.sqrt() * r2.sqrt()))
}

pub(super) fn text_vector(text: &str, dim: usize) -> Vec<f32> {
    let dim = dim.max(1);
    let mut out = Vec::with_capacity(dim);
    for idx in 0..dim {
        let mut hasher = blake3::Hasher::new();
        hasher.update(text.as_bytes());
        hasher.update(&idx.to_le_bytes());
        let bytes = hasher.finalize();
        let raw = u32::from_le_bytes(bytes.as_bytes()[0..4].try_into().expect("hash slice"));
        let unit = raw as f32 / u32::MAX as f32;
        out.push(unit * 2.0 - 1.0);
    }
    normalize(&mut out);
    out
}

pub(super) fn write_json_row<T: Serialize>(
    vault: &AsterVault,
    cf: ColumnFamily,
    key: Vec<u8>,
    value: &T,
) -> ToolResult<Vec<u8>> {
    let bytes = serde_json::to_vec(value)
        .map_err(|err| CalyxError::aster_corrupt_shard(format!("serialize CF row: {err}")))?;
    vault.write_cf(cf, key, bytes.clone())?;
    vault.flush()?;
    Ok(bytes)
}

pub(super) fn read_json_row<T: serde::de::DeserializeOwned>(
    vault: &AsterVault,
    cf: ColumnFamily,
    key: &[u8],
) -> ToolResult<Option<T>> {
    let Some(bytes) = vault.read_cf_at(vault.snapshot(), cf, key)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|err| ToolError::invalid_params(format!("decode CF row: {err}")))
}

pub(super) fn row_exists(vault: &AsterVault, cf: ColumnFamily, key: &[u8]) -> ToolResult<bool> {
    Ok(vault.read_cf_at(vault.snapshot(), cf, key)?.is_some())
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}
