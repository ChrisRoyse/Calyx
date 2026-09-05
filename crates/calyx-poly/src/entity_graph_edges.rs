//! Shared holder/maker/counterparty entity edges into Graph CF (#70).

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Seq};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::model::{CounterpartyVolume, HolderShare, MakerShare, MarketSnapshot};

pub const ENTITY_GRAPH_SCHEMA_VERSION: &str = "poly.entity_graph_edges.v1";
pub const EDGE_SHARED_ENTITY: &str = "association.shared_entity";

pub const ERR_ENTITY_GRAPH_INVALID_INPUT: &str = "CALYX_POLY_ENTITY_GRAPH_INVALID_INPUT";
pub const ERR_ENTITY_GRAPH_EMPTY: &str = "CALYX_POLY_ENTITY_GRAPH_EMPTY";
pub const ERR_ENTITY_GRAPH_READBACK_MISMATCH: &str = "CALYX_POLY_ENTITY_GRAPH_READBACK_MISMATCH";

const SOURCE_HOLDER: &str = "holder";
const SOURCE_MAKER: &str = "maker";
const SOURCE_COUNTERPARTY: &str = "counterparty";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityMarketEvidence {
    pub cx_id: CxId,
    pub condition_id: String,
    pub holders: Vec<HolderShare>,
    pub makers: Vec<MakerShare>,
    pub counterparties: Vec<CounterpartyVolume>,
}

impl EntityMarketEvidence {
    pub fn from_snapshot(cx_id: CxId, snapshot: &MarketSnapshot) -> Self {
        Self {
            cx_id,
            condition_id: snapshot.condition_id.clone(),
            holders: snapshot.holders.clone(),
            makers: snapshot.makers.clone(),
            counterparties: snapshot.counterparty_volumes.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SharedEntity {
    pub address: String,
    pub sources: Vec<String>,
    pub overlap_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub relation_key: String,
    pub shared_entity_count: usize,
    pub shared_entities: Vec<SharedEntity>,
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphAbsence {
    pub code: String,
    pub relation: String,
    pub relation_key: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphEdgeSet {
    pub schema_version: String,
    pub input_count: usize,
    pub edge_count: usize,
    pub absent: Vec<EntityGraphAbsence>,
    pub edges: Vec<EntityGraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphEdgeValue {
    pub schema_version: String,
    pub edge_type: String,
    pub relation_key: String,
    pub shared_entity_count: usize,
    pub shared_entities: Vec<SharedEntity>,
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphReadback {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: EntityGraphEdgeValue,
    pub value_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityGraphRun {
    pub schema_version: String,
    pub collection: String,
    pub domain: String,
    pub snapshot_seq: Seq,
    pub graph_cf_row_count: usize,
    pub computed: EntityGraphEdgeSet,
    pub readback_edges: Vec<EntityGraphReadback>,
}

pub fn compute_entity_graph_edges(records: &[EntityMarketEvidence]) -> Result<EntityGraphEdgeSet> {
    validate_records(records)?;
    let prepared = records
        .iter()
        .map(PreparedMarket::from_record)
        .collect::<Result<Vec<_>>>()?;
    let mut edges = Vec::new();
    let mut absent = Vec::new();
    for i in 0..prepared.len() {
        for j in (i + 1)..prepared.len() {
            compute_pair(&prepared[i], &prepared[j], &mut edges, &mut absent);
        }
    }
    Ok(EntityGraphEdgeSet {
        schema_version: ENTITY_GRAPH_SCHEMA_VERSION.to_string(),
        input_count: records.len(),
        edge_count: edges.len(),
        absent,
        edges,
    })
}

pub fn persist_entity_graph_edges<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    domain: &str,
    records: &[EntityMarketEvidence],
) -> Result<EntityGraphRun> {
    let computed = compute_entity_graph_edges(records)?;
    if computed.edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ENTITY_GRAPH_EMPTY,
            "entity graph materialization requires at least one shared-entity edge",
        ));
    }
    let graph = PlainGraph::new(vault, collection)?;
    for record in records {
        graph.put_node(record.cx_id, &node_value(domain, record)?)?;
    }
    for edge in &computed.edges {
        graph.put_edge(edge.src, &edge.edge_type, edge.dst, &edge_bytes(edge)?)?;
    }
    let snapshot_seq = vault.latest_seq();
    let mut readback_edges = Vec::new();
    for edge in &computed.edges {
        let bytes = graph
            .get_edge(snapshot_seq, edge.src, &edge.edge_type, edge.dst)?
            .ok_or_else(|| readback_error(format!("missing entity edge {}", edge_id(edge))))?;
        let expected = edge_bytes(edge)?;
        if bytes != expected {
            return Err(readback_error(format!(
                "entity edge {} bytes mismatch expected_blake3={} actual_blake3={}",
                edge_id(edge),
                blake3::hash(&expected).to_hex(),
                blake3::hash(&bytes).to_hex()
            )));
        }
        let value: EntityGraphEdgeValue =
            serde_json::from_slice(&bytes).map_err(|err| readback_error(err.to_string()))?;
        readback_edges.push(EntityGraphReadback {
            src: edge.src,
            dst: edge.dst,
            edge_type: edge.edge_type.clone(),
            value,
            value_blake3: blake3::hash(&bytes).to_hex().to_string(),
        });
    }
    Ok(EntityGraphRun {
        schema_version: ENTITY_GRAPH_SCHEMA_VERSION.to_string(),
        collection: collection.to_string(),
        domain: domain.to_string(),
        snapshot_seq,
        graph_cf_row_count: vault.scan_cf_at(snapshot_seq, ColumnFamily::Graph)?.len(),
        computed,
        readback_edges,
    })
}

fn validate_records(records: &[EntityMarketEvidence]) -> Result<()> {
    if records.len() < 2 {
        return Err(invalid(
            "entity graph requires at least two market evidence records",
        ));
    }
    let mut seen = BTreeSet::new();
    for record in records {
        if !seen.insert(record.cx_id) {
            return Err(invalid(format!("duplicate market cx_id {}", record.cx_id)));
        }
        if record.condition_id.trim().is_empty() {
            return Err(invalid(format!(
                "market {} has an empty condition_id",
                record.cx_id
            )));
        }
    }
    Ok(())
}

fn compute_pair(
    left: &PreparedMarket,
    right: &PreparedMarket,
    edges: &mut Vec<EntityGraphEdge>,
    absent: &mut Vec<EntityGraphAbsence>,
) {
    let relation_key = format!("{}|{}", left.condition_id, right.condition_id);
    let mut shared = Vec::new();
    for (address, left_entity) in &left.entities {
        let Some(right_entity) = right.entities.get(address) else {
            continue;
        };
        let mut sources = left_entity.sources.clone();
        sources.extend(right_entity.sources.iter().cloned());
        shared.push(SharedEntity {
            address: address.clone(),
            sources: sources.into_iter().collect(),
            overlap_score: canonical_score(left_entity.score.min(right_entity.score)),
        });
    }
    if shared.is_empty() {
        absent.push(EntityGraphAbsence {
            code: "no_shared_entities".to_string(),
            relation: "shared_entity".to_string(),
            relation_key,
            reason: "market pair has no normalized holder/maker/counterparty overlap".to_string(),
        });
        return;
    }
    shared.sort_by(|a, b| {
        b.overlap_score
            .total_cmp(&a.overlap_score)
            .then_with(|| a.address.cmp(&b.address))
    });
    let weight = canonical_score(
        shared
            .iter()
            .map(|entity| entity.overlap_score)
            .sum::<f64>()
            .clamp(0.0, 1.0),
    );
    push_bidir(edges, left.cx_id, right.cx_id, relation_key, shared, weight);
}

fn push_bidir(
    edges: &mut Vec<EntityGraphEdge>,
    left: CxId,
    right: CxId,
    relation_key: String,
    shared_entities: Vec<SharedEntity>,
    weight: f64,
) {
    push_edge(
        edges,
        left,
        right,
        relation_key.clone(),
        shared_entities.clone(),
        weight,
    );
    push_edge(edges, right, left, relation_key, shared_entities, weight);
}

fn push_edge(
    edges: &mut Vec<EntityGraphEdge>,
    src: CxId,
    dst: CxId,
    relation_key: String,
    shared_entities: Vec<SharedEntity>,
    weight: f64,
) {
    edges.push(EntityGraphEdge {
        src,
        dst,
        edge_type: EDGE_SHARED_ENTITY.to_string(),
        relation_key,
        shared_entity_count: shared_entities.len(),
        shared_entities,
        weight,
    });
}

#[derive(Clone)]
struct EntityScore {
    score: f64,
    sources: BTreeSet<String>,
}

struct PreparedMarket {
    cx_id: CxId,
    condition_id: String,
    entities: BTreeMap<String, EntityScore>,
}

impl PreparedMarket {
    fn from_record(record: &EntityMarketEvidence) -> Result<Self> {
        let mut by_source = BTreeMap::<&'static str, BTreeMap<String, f64>>::new();
        for row in &record.holders {
            add_entity(
                &mut by_source,
                SOURCE_HOLDER,
                &row.wallet,
                row.amount,
                record.cx_id,
            )?;
        }
        for row in &record.makers {
            add_entity(
                &mut by_source,
                SOURCE_MAKER,
                &row.maker,
                row.size,
                record.cx_id,
            )?;
        }
        for row in &record.counterparties {
            add_entity(
                &mut by_source,
                SOURCE_COUNTERPARTY,
                &row.counterparty,
                row.volume,
                record.cx_id,
            )?;
        }
        let mut entities = BTreeMap::new();
        for (source, rows) in by_source {
            let total: f64 = rows.values().sum();
            if total <= 0.0 {
                continue;
            }
            for (address, amount) in rows {
                let score = canonical_score(amount / total);
                let entity = entities.entry(address).or_insert_with(|| EntityScore {
                    score: 0.0,
                    sources: BTreeSet::new(),
                });
                entity.score = canonical_score(entity.score + score);
                entity.sources.insert(source.to_string());
            }
        }
        Ok(Self {
            cx_id: record.cx_id,
            condition_id: record.condition_id.clone(),
            entities,
        })
    }
}

fn add_entity(
    by_source: &mut BTreeMap<&'static str, BTreeMap<String, f64>>,
    source: &'static str,
    raw_address: &str,
    amount: f64,
    cx_id: CxId,
) -> Result<()> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(invalid(format!(
            "{source} entity amount for market {cx_id} must be finite and non-negative"
        )));
    }
    if amount == 0.0 {
        return Ok(());
    }
    let address = normalize_address(raw_address);
    if address.is_empty() {
        return Err(invalid(format!(
            "{source} entity address for market {cx_id} is empty"
        )));
    }
    *by_source
        .entry(source)
        .or_default()
        .entry(address)
        .or_insert(0.0) += amount;
    Ok(())
}

fn edge_bytes(edge: &EntityGraphEdge) -> Result<Vec<u8>> {
    serde_json::to_vec(&edge_value(edge)).map_err(|err| {
        PolyError::diagnostics(
            ERR_ENTITY_GRAPH_READBACK_MISMATCH,
            format!("encode entity graph edge: {err}"),
        )
    })
}

fn edge_value(edge: &EntityGraphEdge) -> EntityGraphEdgeValue {
    EntityGraphEdgeValue {
        schema_version: ENTITY_GRAPH_SCHEMA_VERSION.to_string(),
        edge_type: edge.edge_type.clone(),
        relation_key: edge.relation_key.clone(),
        shared_entity_count: edge.shared_entity_count,
        shared_entities: edge.shared_entities.clone(),
        weight: edge.weight,
    }
}

fn node_value(domain: &str, record: &EntityMarketEvidence) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": ENTITY_GRAPH_SCHEMA_VERSION,
        "domain": domain,
        "cx_id": record.cx_id,
        "condition_id": record.condition_id,
        "holder_rows": record.holders.len(),
        "maker_rows": record.makers.len(),
        "counterparty_rows": record.counterparties.len(),
    }))
    .map_err(|err| readback_error(err.to_string()))
}

fn normalize_address(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn canonical_score(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn edge_id(edge: &EntityGraphEdge) -> String {
    format!("{} -{}-> {}", edge.src, edge.edge_type, edge.dst)
}

fn invalid(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_ENTITY_GRAPH_INVALID_INPUT, message)
}

fn readback_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_ENTITY_GRAPH_READBACK_MISMATCH, message)
}
