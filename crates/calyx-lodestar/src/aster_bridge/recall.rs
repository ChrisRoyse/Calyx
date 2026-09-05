use std::collections::BTreeMap;

use calyx_core::{CalyxError, CxId, SlotId};

use crate::TypedPanelVectors;
use crate::kernel_index::{PANEL_RRF_K, PanelVectors};
use crate::recall_test::{InMemoryAnnIndex, InMemoryCorpus, RecallQuery, RecallTestParams};
use crate::summarize::SummarizeRecall;

use super::{AsterAssocMetadata, AsterAssocNodeProps, bridge_error, to_calyx};

/// Recall inputs derived from an Aster graph, resolved to exactly one contract.
pub enum AsterRecallInputs {
    Legacy {
        embeddings: BTreeMap<CxId, Vec<f32>>,
        corpus: InMemoryCorpus,
        full_index: InMemoryAnnIndex,
        params: RecallTestParams,
    },
    Panel {
        slots: Vec<SlotId>,
        full: BTreeMap<CxId, PanelVectors>,
        corpus_name: String,
        params: RecallTestParams,
    },
    TypedPanel {
        slots: Vec<SlotId>,
        full: BTreeMap<CxId, TypedPanelVectors>,
        corpus_name: String,
        params: RecallTestParams,
    },
}

impl AsterRecallInputs {
    pub fn measurement(&self) -> SummarizeRecall<'_> {
        match self {
            Self::Legacy {
                embeddings,
                corpus,
                full_index,
                params,
            } => SummarizeRecall::Legacy {
                embeddings,
                full_index,
                corpus,
                params: params.clone(),
            },
            Self::Panel {
                slots,
                full,
                corpus_name,
                params,
            } => SummarizeRecall::Panel {
                slots: slots.clone(),
                full,
                corpus_name: corpus_name.clone(),
                params: params.clone(),
            },
            Self::TypedPanel {
                slots,
                full,
                corpus_name,
                params,
            } => SummarizeRecall::TypedPanel {
                slots: slots.clone(),
                full,
                corpus_name: corpus_name.clone(),
                params: params.clone(),
            },
        }
    }
}

/// Resolve the graph's sealed representation without selecting or dropping a lane.
pub(super) fn build_recall_inputs(
    metadata: &AsterAssocMetadata,
    nodes: Vec<(CxId, AsterAssocNodeProps)>,
    corpus_name: String,
    params: RecallTestParams,
) -> Result<Option<AsterRecallInputs>, CalyxError> {
    if nodes.is_empty() {
        return Ok(None);
    }
    let slot_count = metadata.embedding_slots.len();
    if slot_count >= 2 {
        if metadata.embedding_slot.is_some() {
            return Err(bridge_error(
                "CALYX_ASSOC_BRIDGE_AMBIGUOUS_CONTRACT",
                format!(
                    "graph seals both a legacy embedding_slot and {slot_count} panel embedding_slots; the recall contract is ambiguous"
                ),
            ));
        }
        return match metadata.fusion.as_deref() {
            Some("typed_block_rrf") if metadata.rrf_k == Some(PANEL_RRF_K) => {
                typed_panel_inputs(metadata, nodes, corpus_name, params)
            }
            Some("rrf") if metadata.rrf_k == Some(PANEL_RRF_K) => {
                dense_panel_inputs(metadata, nodes, corpus_name, params)
            }
            _ => Err(bridge_error(
                "CALYX_ASSOC_BRIDGE_UNSUPPORTED_FUSION",
                format!(
                    "panel graph must seal fusion=typed_block_rrf or rrf with rrf_k={PANEL_RRF_K}; metadata carries fusion={:?} rrf_k={:?}",
                    metadata.fusion, metadata.rrf_k
                ),
            )),
        };
    }
    if slot_count == 1 {
        return Err(bridge_error(
            "CALYX_ASSOC_BRIDGE_MALFORMED_PANEL",
            "panel-native graph must seal at least two content lanes but embedding_slots has 1; reweave the graph contract".to_string(),
        ));
    }
    legacy_inputs(nodes, corpus_name, params)
}

fn typed_panel_inputs(
    metadata: &AsterAssocMetadata,
    nodes: Vec<(CxId, AsterAssocNodeProps)>,
    corpus_name: String,
    params: RecallTestParams,
) -> Result<Option<AsterRecallInputs>, CalyxError> {
    let mut full = BTreeMap::new();
    for (cx_id, props) in nodes {
        if props.embedding.is_some() || !props.embeddings.is_empty() {
            return Err(mixed_lanes(cx_id, "typed-block"));
        }
        full.insert(cx_id, props.typed_embeddings);
    }
    Ok(Some(AsterRecallInputs::TypedPanel {
        slots: metadata.embedding_slots.clone(),
        full,
        corpus_name,
        params,
    }))
}

fn dense_panel_inputs(
    metadata: &AsterAssocMetadata,
    nodes: Vec<(CxId, AsterAssocNodeProps)>,
    corpus_name: String,
    params: RecallTestParams,
) -> Result<Option<AsterRecallInputs>, CalyxError> {
    let mut full = BTreeMap::new();
    for (cx_id, props) in nodes {
        if props.embedding.is_some() || !props.typed_embeddings.is_empty() {
            return Err(mixed_lanes(cx_id, "dense-panel"));
        }
        full.insert(cx_id, props.embeddings);
    }
    Ok(Some(AsterRecallInputs::Panel {
        slots: metadata.embedding_slots.clone(),
        full,
        corpus_name,
        params,
    }))
}

fn legacy_inputs(
    nodes: Vec<(CxId, AsterAssocNodeProps)>,
    corpus_name: String,
    params: RecallTestParams,
) -> Result<Option<AsterRecallInputs>, CalyxError> {
    let mut embeddings = BTreeMap::new();
    let mut rows = Vec::new();
    for (cx_id, props) in nodes {
        if !props.embeddings.is_empty() || !props.typed_embeddings.is_empty() {
            return Err(mixed_lanes(cx_id, "legacy"));
        }
        let vector = props.embedding.ok_or_else(|| {
            bridge_error(
                "CALYX_SUMMARIZE_RECALL_MISSING_EMBEDDING",
                format!("node {cx_id} has no embedding in Aster graph props"),
            )
        })?;
        embeddings.insert(cx_id, vector.clone());
        rows.push(RecallQuery { cx_id, vector });
    }
    let full_index = InMemoryAnnIndex::new(rows.clone()).map_err(to_calyx)?;
    Ok(Some(AsterRecallInputs::Legacy {
        embeddings,
        corpus: InMemoryCorpus::new(corpus_name, rows),
        full_index,
        params,
    }))
}

fn mixed_lanes(cx_id: CxId, expected: &str) -> CalyxError {
    bridge_error(
        "CALYX_ASSOC_BRIDGE_MIXED_LANES",
        format!(
            "{expected} node {cx_id} carries another vector representation; reweave the graph under one sealed contract"
        ),
    )
}
