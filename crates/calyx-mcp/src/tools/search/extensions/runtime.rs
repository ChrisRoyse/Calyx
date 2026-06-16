use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use calyx_sextant::{
    HnswIndex, InvertedIndex, MaxSimIndex, SearchEngine, SkillParams, SkillTree, SlotIndexMap,
};

use crate::server::{ToolError, ToolResult};

use super::super::engine;

pub(super) struct NavRuntime {
    pub(super) vault: AsterVault,
    pub(super) state: VaultPanelState,
    pub(super) docs: BTreeMap<CxId, Constellation>,
    pub(super) engine: SearchEngine,
}

pub(super) fn load_runtime(vault: &str) -> ToolResult<NavRuntime> {
    let resolved = engine::resolve_requested_vault(vault)?;
    let vault = engine::open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let docs = engine::load_docs(&vault)?;
    let engine = build_search_engine(&docs, vault.latest_seq())?;
    Ok(NavRuntime {
        vault,
        state,
        docs,
        engine,
    })
}

pub(super) fn skill_tree(search: &SearchEngine) -> ToolResult<SkillTree> {
    Ok(calyx_sextant::skills(
        search,
        &SkillParams {
            min_cluster_size: 2,
            min_samples: 1,
            max_constellations: 2048,
            slots: None,
            allow_single_cluster: true,
        },
    )?)
}

pub(super) fn query_vector_for_skill(
    runtime: &NavRuntime,
    text: &str,
) -> ToolResult<Option<(SlotId, SlotVector)>> {
    let slots: BTreeSet<_> = runtime.engine.indexes.slots().into_iter().collect();
    for (slot, vector) in engine::measure_query_vectors(&runtime.state, text)? {
        if slots.contains(&slot)
            && !matches!(vector, SlotVector::Sparse { .. })
            && runtime.docs.values().any(|cx| {
                cx.slots
                    .get(&slot)
                    .is_some_and(|stored| engine::same_index_shape(&vector, stored))
            })
        {
            return Ok(Some((slot, vector)));
        }
    }
    Ok(None)
}

pub(super) fn parse_cx_id(value: &str) -> ToolResult<CxId> {
    value
        .parse::<CxId>()
        .map_err(|err| ToolError::invalid_params(format!("parse cx_id {value}: {err}")))
}

pub(super) fn ensure_doc_exists(
    docs: &BTreeMap<CxId, Constellation>,
    cx_id: CxId,
) -> ToolResult<()> {
    if docs.contains_key(&cx_id) {
        Ok(())
    } else {
        Err(
            CalyxError::vault_access_denied(format!("cx_id {cx_id} does not exist in vault"))
                .into(),
        )
    }
}

pub(super) fn score01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn build_search_engine(docs: &BTreeMap<CxId, Constellation>, seq: u64) -> ToolResult<SearchEngine> {
    let indexes = SlotIndexMap::new();
    let samples = first_vectors(docs);
    for (slot, vector) in &samples {
        match vector {
            SlotVector::Dense { dim, .. } => {
                indexes.register(HnswIndex::new(*slot, *dim, engine::HNSW_SEED))?
            }
            SlotVector::Sparse { .. } => indexes.register(InvertedIndex::new(*slot))?,
            SlotVector::Multi { token_dim, .. } => {
                indexes.register(MaxSimIndex::new(*slot, *token_dim))?
            }
            SlotVector::Absent { .. } => {}
        }
    }
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            if samples
                .get(slot)
                .is_some_and(|sample| engine::same_index_shape(sample, vector))
            {
                indexes.insert(*slot, cx.cx_id, vector.clone(), cx.provenance.seq)?;
            }
        }
    }
    for slot in indexes.registered_slots() {
        indexes.set_base_seq(slot, seq)?;
    }
    let mut search = SearchEngine::new(indexes);
    for cx in docs.values() {
        search.put_constellation(cx.clone());
    }
    search.set_assoc_graph(association_graph(docs)?);
    Ok(search)
}

fn first_vectors(docs: &BTreeMap<CxId, Constellation>) -> BTreeMap<SlotId, SlotVector> {
    let mut out = BTreeMap::new();
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            if engine::indexable(vector) {
                out.entry(*slot).or_insert_with(|| vector.clone());
            }
        }
    }
    out
}

fn association_graph(docs: &BTreeMap<CxId, Constellation>) -> ToolResult<calyx_paths::AssocGraph> {
    let mut builder = calyx_paths::AssocGraph::builder();
    let mut ordered = docs
        .values()
        .map(|cx| (cx.created_at, cx.cx_id))
        .collect::<Vec<_>>();
    ordered.sort();
    for (_, cx_id) in &ordered {
        builder
            .add_node(*cx_id, 1.0)
            .map_err(|err| CalyxError::stale_derived(format!("build association node: {err}")))?;
    }
    for pair in ordered.windows(2) {
        let left = docs.get(&pair[0].1).expect("ordered id in docs");
        let right = docs.get(&pair[1].1).expect("ordered id in docs");
        builder
            .add_edge(left.cx_id, right.cx_id, association_weight(left, right))
            .map_err(|err| CalyxError::stale_derived(format!("build association edge: {err}")))?;
    }
    Ok(builder.build())
}

fn association_weight(left: &Constellation, right: &Constellation) -> f32 {
    let mut sum = 0.0_f32;
    let mut n = 0_usize;
    for (slot, vector) in &left.slots {
        if let (Some(a), Some(b)) = (
            vector.as_dense(),
            right.slots.get(slot).and_then(SlotVector::as_dense),
        ) && let Some(cos) = calyx_core::dense_cosine(a, b)
        {
            sum += cos.clamp(0.0, 1.0);
            n += 1;
        }
    }
    if n == 0 {
        1.0
    } else {
        (sum / n as f32).clamp(0.05, 1.0)
    }
}
