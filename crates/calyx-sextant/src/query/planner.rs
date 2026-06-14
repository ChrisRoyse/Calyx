//! Cross-model universal query planner for PH55.

use std::collections::BTreeSet;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{SecondaryIndexKind, SecondaryIndexSpec};
use calyx_aster::layers::{document, relational};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, Result};

use crate::error::{CALYX_PLANNER_COST_CAP, sextant_error};

use super::{
    CrossModelPlan, DocPathFilter, ExplainOutput, ExplainStep, PlanStep, RelationalFilter,
    UniversalQuery,
};

pub const DEFAULT_COST_CAP_MS: u32 = 30_000;
const MIN_FULL_SCAN_COST_MS: f32 = 50.0;
const FULL_SCAN_COST_PER_1K_ROWS_MS: f32 = 50.0;
const INDEX_SCAN_COST_MS: f32 = 5.0;
const KV_GET_COST_MS: f32 = 0.1;
const TS_COST_PER_1K_POINTS_MS: f32 = 1.0;
const GRAPH_HOP_COST_MS: f32 = 10.0;
const VECTOR_LENS_COST_MS: f32 = 5.0;
const ASK_COST_MS: f32 = 200.0;
const DOC_SCAN_MIN_COST_MS: f32 = 25.0;
const AGGREGATE_COST_MS: f32 = 1.0;

#[derive(Clone, Debug)]
struct PlannedStep {
    step: PlanStep,
    cost_ms: f32,
    chosen_index: Option<SecondaryIndexSpec>,
}

pub fn plan<C>(vault: &AsterVault<C>, query: &UniversalQuery) -> Result<CrossModelPlan>
where
    C: Clock,
{
    let mut planned = Vec::new();

    if let Some(relational) = &query.relational {
        planned.push(plan_relational(vault, relational)?);
    }
    if let Some(kv) = &query.kv {
        planned.push(PlannedStep {
            step: PlanStep::KvGet {
                ns: kv.ns.clone(),
                key: kv.key.clone(),
            },
            cost_ms: KV_GET_COST_MS,
            chosen_index: None,
        });
    }
    if let Some(document) = &query.document {
        let docs = document
            .estimated_docs
            .unwrap_or(count_document_rows(vault, &document.collection)?);
        let cost_ms = if docs == 0 {
            0.0
        } else {
            DOC_SCAN_MIN_COST_MS.max(docs as f32 * 0.5)
        };
        planned.push(PlannedStep {
            step: PlanStep::DocScan {
                collection: document.collection.clone(),
                path_filter: DocPathFilter {
                    path: document.path.clone(),
                    value: document.value.clone(),
                },
            },
            cost_ms,
            chosen_index: None,
        });
    }
    if let Some(timeseries) = &query.timeseries {
        let points = timeseries.estimated_points.unwrap_or(1);
        planned.push(PlannedStep {
            step: PlanStep::TsRangeScan {
                series: timeseries.series.clone(),
                start: timeseries.start,
                end: timeseries.end,
            },
            cost_ms: points_to_cost(points),
            chosen_index: None,
        });
    }
    if let Some(graph) = &query.graph_hop {
        planned.push(PlannedStep {
            step: PlanStep::GraphHop {
                from_cx_ids: graph.from_cx_ids.clone(),
                hop_kind: graph.hop_kind.clone(),
            },
            cost_ms: GRAPH_HOP_COST_MS * f32::from(graph.max_hops.max(1)),
            chosen_index: None,
        });
    }
    if let Some(vector) = &query.vector {
        planned.push(PlannedStep {
            step: PlanStep::VectorFusion {
                lens_ids: vector.lens_ids.clone(),
                query_vec: vector.query_vec.clone(),
                limit: vector.limit,
            },
            cost_ms: VECTOR_LENS_COST_MS * vector.lens_ids.len().max(1) as f32,
            chosen_index: None,
        });
    }
    if let Some(aggregate) = &query.aggregate {
        planned.push(PlannedStep {
            step: PlanStep::Aggregate {
                spec: aggregate.clone(),
            },
            cost_ms: AGGREGATE_COST_MS,
            chosen_index: None,
        });
    }
    if let Some(ask) = &query.ask {
        planned.push(PlannedStep {
            step: PlanStep::Ask {
                question: ask.question.clone(),
                context_cx_ids: ask.context_cx_ids.clone(),
                top_k: ask.top_k,
                oracle: ask.oracle,
            },
            cost_ms: ASK_COST_MS,
            chosen_index: None,
        });
    }

    let estimated_cost_ms = if planned.is_empty() {
        0.0
    } else {
        planned.iter().map(|step| step.cost_ms).sum::<f32>()
    };
    enforce_cost_cap(query, estimated_cost_ms)?;

    let explain = query
        .explain
        .then(|| explain_for(&planned, estimated_cost_ms));
    Ok(CrossModelPlan {
        steps: planned.into_iter().map(|step| step.step).collect(),
        estimated_cost_ms,
        explain,
    })
}

fn plan_relational<C>(vault: &AsterVault<C>, filter: &RelationalFilter) -> Result<PlannedStep>
where
    C: Clock,
{
    let index = choose_index(filter);
    let cost_ms = if index.is_some() {
        INDEX_SCAN_COST_MS
    } else {
        let rows = filter
            .estimated_rows
            .unwrap_or(count_relational_rows(vault, filter)?);
        full_scan_cost(rows)
    };
    Ok(PlannedStep {
        step: PlanStep::RelationalScan {
            collection: filter.collection.clone(),
            filter: filter.predicates.clone(),
            index: index.clone(),
        },
        cost_ms,
        chosen_index: index,
    })
}

fn choose_index(filter: &RelationalFilter) -> Option<SecondaryIndexSpec> {
    let predicate_fields = filter
        .predicates
        .iter()
        .map(|predicate| predicate.field.as_str())
        .collect::<BTreeSet<_>>();
    filter
        .collection
        .indexes
        .iter()
        .find(|index| {
            index.kind == SecondaryIndexKind::Btree
                && index
                    .fields
                    .iter()
                    .any(|field| predicate_fields.contains(field.as_str()))
        })
        .cloned()
}

fn count_relational_rows<C>(vault: &AsterVault<C>, filter: &RelationalFilter) -> Result<u64>
where
    C: Clock,
{
    let mut prefix = Vec::with_capacity(9);
    prefix.push(0x01);
    prefix.extend_from_slice(&relational::collection_id(&filter.collection).to_be_bytes());
    Ok(vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Relational)?
        .into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count() as u64)
}

fn count_document_rows<C>(
    vault: &AsterVault<C>,
    collection: &calyx_aster::collection::Collection,
) -> Result<u64>
where
    C: Clock,
{
    let mut prefix = Vec::with_capacity(9);
    prefix.push(0x02);
    prefix.extend_from_slice(&document::collection_id(collection).to_be_bytes());
    Ok(vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Document)?
        .into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count() as u64)
}

fn full_scan_cost(rows: u64) -> f32 {
    if rows == 0 {
        0.0
    } else {
        MIN_FULL_SCAN_COST_MS.max(rows as f32 / 1_000.0 * FULL_SCAN_COST_PER_1K_ROWS_MS)
    }
}

fn points_to_cost(points: u64) -> f32 {
    if points == 0 {
        0.0
    } else {
        TS_COST_PER_1K_POINTS_MS.max(points as f32 / 1_000.0 * TS_COST_PER_1K_POINTS_MS)
    }
}

fn enforce_cost_cap(query: &UniversalQuery, estimated_cost_ms: f32) -> Result<()> {
    let cap = query
        .cost_cap_ms
        .or_else(|| {
            query
                .relational
                .as_ref()
                .and_then(|filter| filter.collection.txn_policy.cost_cap_ms)
        })
        .or_else(|| {
            query
                .document
                .as_ref()
                .and_then(|filter| filter.collection.txn_policy.cost_cap_ms)
        })
        .unwrap_or(DEFAULT_COST_CAP_MS);
    if estimated_cost_ms > cap as f32 {
        return Err(sextant_error(
            CALYX_PLANNER_COST_CAP,
            format!("cross-model plan estimated {estimated_cost_ms:.1} ms exceeds cap {cap} ms"),
        ));
    }
    Ok(())
}

fn explain_for(planned: &[PlannedStep], total_cost_ms: f32) -> ExplainOutput {
    ExplainOutput {
        steps: planned
            .iter()
            .enumerate()
            .map(|(ordinal, planned)| ExplainStep {
                ordinal,
                kind: planned.step.kind(),
                estimated_cost_ms: planned.cost_ms,
                chosen_index: planned.chosen_index.clone(),
            })
            .collect(),
        total_cost_ms,
    }
}

#[cfg(test)]
mod fsv_tests;
#[cfg(test)]
mod tests;
