//! One-pass PH55 cross-model query executor.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{Collection, CollectionMode, SecondaryIndexSpec};
use calyx_aster::index::btree::btree_range_at;
use calyx_aster::layers::document::DocId;
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::relational::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::layers::{DocumentLayer, KvLayer, TimeSeriesLayer};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Result, Seq, SlotId};
use serde_json::Value;

use crate::fusion::rrf::rrf_fuse_restricted;
use crate::fusion::{FusionContext, FusionStrategy};
use crate::index::IndexSearchHit;

use super::ask::ask as ask_query;
use super::{
    AggOp, AggSpec, AskSpec, CrossModelPlan, DocPathFilter, FieldPredicate, PlanStep,
    ProvenancedRow, QueryResult,
};
use support::{
    cx_from_key, default_collection, doc_value_matches, fold_numeric, index_bounds, json_row,
    ledger_ref, numeric_values, parse_record_pk, plain_row, relational_prefix, require_mode,
    row_matches, runtime_index, scan_doc_ids, scoped_u64, shape,
};

const DEFAULT_KV_COLLECTION: &str = "kv";
const DEFAULT_TS_COLLECTION: &str = "timeseries";

struct ExecState {
    rows: Vec<ProvenancedRow>,
    candidates: BTreeSet<CxId>,
    total_scanned: u64,
}

pub fn execute<C>(vault: &AsterVault<C>, plan: CrossModelPlan) -> Result<QueryResult>
where
    C: Clock,
{
    let snapshot = vault.latest_seq();
    execute_at_snapshot(vault, plan, snapshot)
}

fn execute_at_snapshot<C>(
    vault: &AsterVault<C>,
    plan: CrossModelPlan,
    snapshot: Seq,
) -> Result<QueryResult>
where
    C: Clock,
{
    let started = Instant::now();
    let explain = plan.explain.clone();
    let mut state = ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::new(),
        total_scanned: 0,
    };
    for step in plan.steps {
        apply_step(vault, snapshot, &mut state, step)?;
    }
    annotate_provenance(vault, snapshot, &mut state.rows);
    Ok(QueryResult {
        rows: state.rows,
        total_scanned: state.total_scanned,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        explain,
    })
}

fn apply_step<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    step: PlanStep,
) -> Result<()>
where
    C: Clock,
{
    match step {
        PlanStep::RelationalScan {
            collection,
            filter,
            index,
        } => execute_relational(vault, snapshot, state, &collection, &filter, index.as_ref()),
        PlanStep::DocScan {
            collection,
            path_filter,
        } => execute_doc(vault, snapshot, state, &collection, &path_filter),
        PlanStep::KvGet { ns, key } => execute_kv(vault, snapshot, state, &ns, &key),
        PlanStep::TsRangeScan { series, start, end } => {
            execute_ts(vault, snapshot, state, &series, start, end)
        }
        PlanStep::GraphHop {
            from_cx_ids,
            hop_kind,
        } => {
            tracing::info!(hop_kind = %hop_kind, "[INFO] graph hop stubbed");
            execute_graph_stub(state, from_cx_ids)
        }
        PlanStep::VectorFusion {
            lens_ids,
            query_vec,
            limit,
        } => execute_vector_fusion(vault, snapshot, state, lens_ids.len(), &query_vec, limit),
        PlanStep::Aggregate { spec } => execute_aggregate(state, &spec),
        PlanStep::Ask {
            question,
            context_cx_ids,
            top_k,
            oracle,
        } => execute_ask(
            vault,
            snapshot,
            state,
            question,
            context_cx_ids,
            top_k,
            oracle,
        ),
    }
}

fn execute_relational<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    collection: &Collection,
    predicates: &[FieldPredicate],
    index: Option<&SecondaryIndexSpec>,
) -> Result<()>
where
    C: Clock,
{
    require_mode(collection, CollectionMode::Records, "relational")?;
    let keys = if let Some(index) = index {
        indexed_record_keys(vault, snapshot, collection, index, predicates)?
    } else {
        full_scan_record_keys(vault, snapshot, collection, state)?
    };
    let layer = RelationalLayer::new(vault);
    let mut rows = Vec::new();
    for key in keys {
        if let Some(row) = layer.get_record_at(snapshot, collection, &key)? {
            state.total_scanned += 1;
            if row_matches(&row, predicates) {
                rows.push(plain_row(key, row));
            }
        }
    }
    state.rows = rows;
    Ok(())
}

fn indexed_record_keys<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    collection: &Collection,
    declared: &SecondaryIndexSpec,
    predicates: &[FieldPredicate],
) -> Result<Vec<RecordKey>>
where
    C: Clock,
{
    let spec = runtime_index(collection, declared, predicates)?;
    let (gte, lte) = index_bounds(&spec, predicates)?;
    btree_range_at(
        vault,
        snapshot,
        collection,
        &spec,
        gte.as_ref(),
        lte.as_ref(),
        0,
    )
}

fn full_scan_record_keys<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    collection: &Collection,
    state: &mut ExecState,
) -> Result<Vec<RecordKey>>
where
    C: Clock,
{
    let prefix = relational_prefix(collection);
    let rows = vault.scan_cf_at(snapshot, ColumnFamily::Relational)?;
    state.total_scanned += rows.len() as u64;
    rows.into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(key, _)| parse_record_pk(&key))
        .collect()
}

fn execute_doc<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    collection: &Collection,
    filter: &DocPathFilter,
) -> Result<()>
where
    C: Clock,
{
    require_mode(collection, CollectionMode::Documents, "document")?;
    let doc_ids = if state.rows.is_empty() {
        scan_doc_ids(vault, snapshot, collection, state)?
    } else {
        state
            .rows
            .iter()
            .filter_map(|row| DocId::from_slice(row.key.as_bytes()).ok())
            .collect()
    };
    let layer = DocumentLayer::new(vault);
    let path = filter.path.iter().map(String::as_str).collect::<Vec<_>>();
    let mut rows = Vec::new();
    for doc_id in doc_ids {
        let subtree = layer.get_subtree_at(snapshot, collection, doc_id, &path)?;
        if doc_value_matches(subtree.as_ref(), filter.value.as_ref()) {
            rows.push(plain_row(
                RecordKey::from_bytes(doc_id.as_bytes().to_vec())?,
                json_row("document", subtree.unwrap_or(Value::Null))?,
            ));
        }
    }
    state.rows = rows;
    Ok(())
}

fn execute_kv<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    ns: &str,
    key: &[u8],
) -> Result<()>
where
    C: Clock,
{
    let (collection_name, ns) = scoped_u64(ns, DEFAULT_KV_COLLECTION);
    let collection = default_collection(&collection_name, CollectionMode::KV);
    if let Some(value) = KvLayer::new(vault).kv_get_at(snapshot, &collection, ns, key)? {
        state.rows.push(plain_row(
            RecordKey::from_bytes(kv_key(&collection, ns, key))?,
            Row::new([("__value", RecordValue::Bytes(value))]),
        ));
    }
    state.total_scanned += 1;
    Ok(())
}

fn execute_ts<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    series: &str,
    start: i64,
    end: i64,
) -> Result<()>
where
    C: Clock,
{
    if start < 0 || end < 0 {
        return Err(shape("time-series range bounds must be non-negative"));
    }
    let (collection_name, series_id) = scoped_u64(series, DEFAULT_TS_COLLECTION);
    let collection = default_collection(&collection_name, CollectionMode::TimeSeries);
    for (ts, value) in TimeSeriesLayer::new(vault).ts_range_at(
        snapshot,
        &collection,
        series_id,
        start as u64,
        end as u64,
    )? {
        state.total_scanned += 1;
        state.rows.push(plain_row(
            RecordKey::from_bytes(calyx_aster::layers::timeseries::point_key(
                &collection,
                series_id,
                ts,
            ))?,
            Row::new([
                ("ts", RecordValue::U64(ts)),
                ("value", RecordValue::F64(value)),
            ]),
        ));
    }
    Ok(())
}

fn execute_graph_stub(state: &mut ExecState, from_cx_ids: Vec<CxId>) -> Result<()> {
    if !from_cx_ids.is_empty() {
        state.candidates = from_cx_ids.into_iter().collect();
    } else if state.candidates.is_empty() {
        state.candidates = state
            .rows
            .iter()
            .filter_map(|row| cx_from_key(&row.key))
            .collect();
    }
    if state.rows.is_empty() {
        state.rows = state
            .candidates
            .iter()
            .map(|cx| ProvenancedRow {
                key: RecordKey::from_bytes(cx.as_bytes().to_vec()).expect("CxId key is non-empty"),
                value: None,
                score: None,
                ledger_ref: None,
            })
            .collect();
    }
    Ok(())
}

fn execute_vector_fusion<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    lens_count: usize,
    query_vec: &[f32],
    limit: usize,
) -> Result<()>
where
    C: Clock,
{
    if limit == 0 || query_vec.iter().any(|value| !value.is_finite()) {
        return Err(shape(
            "vector fusion requires a positive limit and finite query vector",
        ));
    }
    if state.candidates.is_empty() {
        state.rows.clear();
        return Ok(());
    }
    let mut per_slot = BTreeMap::new();
    for idx in 0..lens_count.max(1).min(u16::MAX as usize) {
        per_slot.insert(
            SlotId::new(idx as u16),
            state
                .candidates
                .iter()
                .enumerate()
                .map(|(rank, cx_id)| IndexSearchHit {
                    cx_id: *cx_id,
                    score: 1.0 / (rank + 1) as f32,
                    rank: rank + 1,
                })
                .collect::<Vec<_>>(),
        );
    }
    let hits = rrf_fuse_restricted(
        &per_slot,
        &FusionContext {
            k: limit,
            explain: false,
            strategy: FusionStrategy::Rrf,
            weights: BTreeMap::new(),
            stage1_slots: Vec::new(),
        },
        &state.candidates,
    );
    let hit_rows = hits
        .into_iter()
        .map(|hit| ProvenancedRow {
            key: RecordKey::from_bytes(hit.cx_id.as_bytes().to_vec()).expect("CxId key"),
            value: None,
            score: Some(hit.score),
            ledger_ref: ledger_ref(vault, snapshot, hit.cx_id),
        })
        .collect();
    merge_vector_hits(state, hit_rows);
    Ok(())
}

fn merge_vector_hits(state: &mut ExecState, hit_rows: Vec<ProvenancedRow>) {
    if state.rows.is_empty() {
        state.rows = hit_rows;
        return;
    }
    let mut hits = hit_rows
        .into_iter()
        .map(|row| (row.key.as_bytes().to_vec(), row))
        .collect::<BTreeMap<_, _>>();
    let mut merged = Vec::with_capacity(state.rows.len() + hits.len());
    for row in state.rows.drain(..) {
        if cx_from_key(&row.key).is_some() {
            if let Some(hit) = hits.remove(row.key.as_bytes()) {
                merged.push(hit);
            }
        } else {
            merged.push(row);
        }
    }
    merged.extend(hits.into_values());
    state.rows = merged;
}

fn execute_ask<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    question: String,
    mut context_cx_ids: Vec<CxId>,
    top_k: usize,
    oracle: bool,
) -> Result<()>
where
    C: Clock,
{
    if context_cx_ids.is_empty() {
        context_cx_ids.extend(state.candidates.iter().copied());
    }
    if context_cx_ids.is_empty() {
        context_cx_ids.extend(state.rows.iter().filter_map(|row| cx_from_key(&row.key)));
    }
    let result = ask_query(
        vault,
        &AskSpec {
            question,
            context_cx_ids,
            top_k,
            oracle,
        },
        snapshot,
    )?;
    state.candidates.extend(
        result
            .grounding
            .iter()
            .filter_map(|row| cx_from_key(&row.key)),
    );
    state.total_scanned += result.grounding.len() as u64;
    state.rows.extend(result.grounding);
    Ok(())
}

fn annotate_provenance<C>(vault: &AsterVault<C>, snapshot: Seq, rows: &mut [ProvenancedRow])
where
    C: Clock,
{
    for row in rows.iter_mut().filter(|row| row.ledger_ref.is_none()) {
        let Some(cx_id) = cx_from_key(&row.key) else {
            continue;
        };
        row.ledger_ref = ledger_ref(vault, snapshot, cx_id);
    }
}

fn execute_aggregate(state: &mut ExecState, spec: &AggSpec) -> Result<()> {
    let value = match spec.op {
        AggOp::Count => RecordValue::U64(state.rows.len() as u64),
        AggOp::Sum => RecordValue::F64(numeric_values(&state.rows, spec).iter().sum()),
        AggOp::Min => RecordValue::F64(fold_numeric(&state.rows, spec, f64::min)?),
        AggOp::Max => RecordValue::F64(fold_numeric(&state.rows, spec, f64::max)?),
        AggOp::Avg => {
            let values = numeric_values(&state.rows, spec);
            RecordValue::F64(values.iter().sum::<f64>() / values.len().max(1) as f64)
        }
    };
    state.rows = vec![plain_row(
        RecordKey::from_bytes(format!("aggregate:{:?}", spec.op).into_bytes())?,
        Row::new([("value", value)]),
    )];
    Ok(())
}

#[cfg(test)]
mod fsv_tests;
#[cfg(test)]
mod tests;
