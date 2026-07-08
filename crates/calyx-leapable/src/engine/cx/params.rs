use std::collections::BTreeMap;

use calyx_core::{Anchor, Modality, SlotVector, Ts};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct CxPutParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    #[serde(flatten)]
    pub(super) item: CxPutItem,
}

#[derive(Deserialize)]
pub(super) struct CxPutBatchParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) items: Vec<CxPutItem>,
}

#[derive(Clone, Deserialize)]
pub(super) struct CxPutItem {
    pub(super) panel_version: u32,
    pub(super) modality: Modality,
    pub(super) input: CxInput,
    #[serde(default)]
    pub(super) slots: Vec<CxSlotParam>,
    #[serde(default)]
    pub(super) scalars: BTreeMap<String, f64>,
    #[serde(default)]
    pub(super) metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) anchors: Vec<Anchor>,
    #[serde(default)]
    pub(super) ts: Option<Ts>,
}

#[derive(Clone, Deserialize)]
pub(super) struct CxInput {
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) bytes: Option<Vec<u8>>,
    #[serde(default)]
    pub(super) hex: Option<String>,
    #[serde(default)]
    pub(super) pointer: Option<String>,
    #[serde(default)]
    pub(super) redacted: bool,
}

#[derive(Clone, Deserialize)]
pub(super) struct CxSlotParam {
    pub(super) slot_id: u16,
    pub(super) vector: SlotVector,
}

#[derive(Deserialize)]
pub(super) struct CxGetParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) cx_id: String,
    #[serde(default)]
    pub(super) snapshot: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct CxScanParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    #[serde(default)]
    pub(super) snapshot: Option<u64>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct CxAnchorParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) cx_id: String,
    pub(super) anchor: Anchor,
}

#[derive(Deserialize)]
pub(super) struct CxDeleteParams {
    pub(super) vault_ref: String,
    pub(super) ts: Ts,
    pub(super) cx_id: String,
}
