use std::collections::BTreeMap;

use calyx_core::{CalyxError, Clock, Result, Seq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cf::{
    CfRouter, ColumnFamily, WEAVE_XTERM_GENERATION_SCHEMA, WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
    weave_xterm_generation_key,
};
use crate::mvcc::is_tombstone_value;
use crate::vault::AsterVault;

use super::key::{GraphKeyspace, graph_corrupt};

const LIFECYCLE_COLLECTION: &str = "__calyx_graph_lifecycle";
pub const GRAPH_GENERATION_STORAGE_SCHEMA: &str = "calyx.graph.generation.v1";
pub const GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL: &str = "storage_schema";
pub const GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL: &str = "storage_collection";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphCollectionGenerationStatus {
    Writing,
    Accepted,
    Failed,
    Tombstoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCollectionGenerationState {
    pub collection: String,
    pub generation: String,
    pub status: GraphCollectionGenerationStatus,
    pub command: String,
    pub reason: Option<String>,
    pub updated_unix_ms: u128,
    #[serde(default)]
    pub detail: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCollectionGenerationReadback {
    pub state: GraphCollectionGenerationState,
    pub key_sha256: String,
    pub value_sha256: String,
    pub value_bytes: usize,
}

pub struct GraphCollectionLifecycle<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    keys: GraphKeyspace,
}

pub struct PhysicalGraphCollectionLifecycle {
    router: CfRouter,
    keys: GraphKeyspace,
}

impl GraphCollectionGenerationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Writing => "writing",
            Self::Accepted => "accepted",
            Self::Failed => "failed",
            Self::Tombstoned => "tombstoned",
        }
    }
}

impl GraphCollectionGenerationState {
    pub fn new(
        collection: impl Into<String>,
        generation: impl Into<String>,
        status: GraphCollectionGenerationStatus,
        command: impl Into<String>,
    ) -> Self {
        Self {
            collection: collection.into(),
            generation: generation.into(),
            status,
            command: command.into(),
            reason: None,
            updated_unix_ms: now_unix_ms(),
            detail: BTreeMap::new(),
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.detail.insert(key.into(), value.into());
        self
    }

    pub fn visible_by_default(&self) -> bool {
        self.status == GraphCollectionGenerationStatus::Accepted
    }
}

/// Derives the physically isolated graph collection for one logical generation.
/// The digest keeps generated collection names bounded and makes the namespace
/// a pure function that readers can independently verify before trusting it.
pub fn graph_generation_storage_collection(collection: &str, generation: &str) -> Result<String> {
    GraphKeyspace::new(collection)?;
    validate_token("generation", generation, 128)?;
    let mut hasher = Sha256::new();
    hasher.update(GRAPH_GENERATION_STORAGE_SCHEMA.as_bytes());
    hasher.update([0xff]);
    hasher.update(collection.as_bytes());
    hasher.update([0xff]);
    hasher.update(generation.as_bytes());
    Ok(format!(
        "__calyx_graph_generation_v1:{:x}",
        hasher.finalize()
    ))
}

pub fn graph_generation_storage_range(
    collection: &str,
    generation: &str,
) -> Result<crate::cf::KeyRange> {
    let storage = graph_generation_storage_collection(collection, generation)?;
    Ok(GraphKeyspace::new(&storage)?.collection_range())
}

impl<'a, C: Clock> GraphCollectionLifecycle<'a, C> {
    pub fn new(vault: &'a AsterVault<C>) -> Result<Self> {
        Ok(Self {
            vault,
            keys: GraphKeyspace::new(LIFECYCLE_COLLECTION)?,
        })
    }

    pub fn put_state(&self, state: &GraphCollectionGenerationState) -> Result<Seq> {
        validate_state(state)?;
        let value = serde_json::to_vec(state).map_err(lifecycle_corrupt)?;
        self.vault
            .write_cf(ColumnFamily::Graph, self.state_key(state)?, value)
    }

    pub(crate) fn put_state_locked(&self, state: &GraphCollectionGenerationState) -> Result<Seq> {
        validate_state(state)?;
        let value = serde_json::to_vec(state).map_err(lifecycle_corrupt)?;
        self.vault
            .commit_rows_locked(&[crate::vault::encode::WriteRow {
                cf: ColumnFamily::Graph,
                key: self.state_key(state)?,
                value,
            }])
    }

    pub fn list_states(&self, snapshot: Seq) -> Result<Vec<GraphCollectionGenerationReadback>> {
        let range = self.keys.metadata_range();
        self.vault
            .scan_cf_range_at(snapshot, ColumnFamily::Graph, &range)?
            .into_iter()
            .filter(|(_, value)| !is_tombstone_value(value))
            .map(|(key, value)| decode_readback(&key, &value))
            .collect()
    }

    /// Resolves a canonical XTerm key through the latest accepted graph
    /// generation. Legacy collections with no lifecycle metadata retain their
    /// canonical keyspace; once lifecycle rows exist, absence of an accepted
    /// generation fails closed instead of exposing partial rows.
    pub fn resolve_weave_xterm_key(
        &self,
        snapshot: Seq,
        collection: &str,
        canonical_key: &[u8],
    ) -> Result<Vec<u8>> {
        GraphKeyspace::new(collection)?;
        let collection_rows = self
            .list_states(snapshot)?
            .into_iter()
            .filter(|row| row.state.collection == collection)
            .collect::<Vec<_>>();
        if collection_rows.is_empty() {
            return Ok(canonical_key.to_vec());
        }
        let accepted = collection_rows
            .into_iter()
            .filter(|row| row.state.status == GraphCollectionGenerationStatus::Accepted)
            .max_by(|left, right| {
                (left.state.updated_unix_ms, &left.state.generation)
                    .cmp(&(right.state.updated_unix_ms, &right.state.generation))
            })
            .ok_or_else(|| CalyxError {
                code: "CALYX_GRAPH_COLLECTION_NOT_ACCEPTED",
                message: format!(
                    "graph collection {collection} has lifecycle rows but no accepted generation"
                ),
                remediation: "complete and physically verify a graph/XTerm generation before serving it",
            })?;
        match accepted.state.detail.get(WEAVE_XTERM_STORAGE_SCHEMA_DETAIL) {
            Some(schema) if schema == WEAVE_XTERM_GENERATION_SCHEMA => {
                weave_xterm_generation_key(collection, &accepted.state.generation, canonical_key)
            }
            None if !accepted
                .state
                .detail
                .contains_key(GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL) =>
            {
                // Explicit pre-isolation accepted schema.
                Ok(canonical_key.to_vec())
            }
            Some(schema) => Err(graph_corrupt(format!(
                "accepted graph generation {}/{} names unsupported XTerm schema {schema}",
                collection, accepted.state.generation
            ))),
            None => Err(graph_corrupt(format!(
                "accepted isolated graph generation {}/{} omits XTerm storage schema",
                collection, accepted.state.generation
            ))),
        }
    }

    fn state_key(&self, state: &GraphCollectionGenerationState) -> Result<Vec<u8>> {
        self.keys
            .metadata_key(&state_row_name(&state.collection, &state.generation))
    }
}

/// Publishes a fully verified failed generation only if the vault tip and its
/// lifecycle row are byte-semantically unchanged from the verifier's read.
///
/// The comparison and accepted-state commit share Aster's durable commit lock,
/// closing the check/write race that a CLI-level `latest_seq` comparison would
/// otherwise leave. Physical Graph/XTerm verification deliberately happens
/// before this short critical section.
pub fn accept_verified_graph_generation<C: Clock>(
    vault: &AsterVault<C>,
    expected_latest_seq: Seq,
    expected_failed: &GraphCollectionGenerationState,
    accepted: &GraphCollectionGenerationState,
) -> Result<Seq> {
    if expected_failed.status != GraphCollectionGenerationStatus::Failed
        || accepted.status != GraphCollectionGenerationStatus::Accepted
        || accepted.collection != expected_failed.collection
        || accepted.generation != expected_failed.generation
    {
        return Err(CalyxError {
            code: "CALYX_GRAPH_GENERATION_ACCEPT_INVALID",
            message: "verified generation publication requires one failed row and a matching accepted row"
                .to_string(),
            remediation: "verify the exact failed generation and publish only an accepted state for the same collection/generation",
        });
    }
    validate_state(expected_failed)?;
    validate_state(accepted)?;

    vault.with_durable_commit_lock(|| {
        let observed_latest_seq = vault.latest_seq();
        if observed_latest_seq != expected_latest_seq {
            return Err(CalyxError::stale_derived(format!(
                "verified graph generation publication expected vault sequence {expected_latest_seq}, observed {observed_latest_seq}"
            )));
        }
        let lifecycle = GraphCollectionLifecycle::new(vault)?;
        let observed = lifecycle
            .list_states(observed_latest_seq)?
            .into_iter()
            .find(|row| {
                row.state.collection == expected_failed.collection
                    && row.state.generation == expected_failed.generation
            })
            .ok_or_else(|| {
                CalyxError::stale_derived(format!(
                    "verified graph generation lifecycle row disappeared for {}/{}",
                    expected_failed.collection, expected_failed.generation
                ))
            })?;
        if &observed.state != expected_failed {
            return Err(CalyxError::stale_derived(format!(
                "verified graph generation lifecycle row changed for {}/{}",
                expected_failed.collection, expected_failed.generation
            )));
        }
        let seq = lifecycle.put_state_locked(accepted)?;
        vault.flush_locked()?;
        Ok(seq)
    })
}

impl PhysicalGraphCollectionLifecycle {
    pub fn open_latest(vault_dir: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            router: CfRouter::open_selected_cfs(vault_dir, 0, [ColumnFamily::Graph])?,
            keys: GraphKeyspace::new(LIFECYCLE_COLLECTION)?,
        })
    }

    pub fn put_state_physical(&mut self, state: &GraphCollectionGenerationState) -> Result<()> {
        validate_state(state)?;
        let key = self
            .keys
            .metadata_key(&state_row_name(&state.collection, &state.generation))?;
        if self.router.get(ColumnFamily::Graph, &key)?.is_some() {
            return Err(CalyxError {
                code: "CALYX_GRAPH_LIFECYCLE_COMMIT_REQUIRED",
                message: format!(
                    "physical lifecycle write cannot update committed generation {}/{}",
                    state.collection, state.generation
                ),
                remediation: "write lifecycle transitions through GraphCollectionLifecycle so WAL, MVCC, and physical readback share one commit sequence",
            });
        }
        let value = serde_json::to_vec(state).map_err(lifecycle_corrupt)?;
        self.router.put(ColumnFamily::Graph, &key, &value)?;
        self.router.flush_cf(ColumnFamily::Graph)?;
        Ok(())
    }

    pub fn list_states(&self) -> Result<Vec<GraphCollectionGenerationReadback>> {
        let range = self.keys.metadata_range();
        let end = range
            .end
            .as_deref()
            .ok_or_else(|| graph_corrupt("graph lifecycle range is unexpectedly unbounded"))?;
        self.router
            .range(ColumnFamily::Graph, &range.start, end)?
            .into_iter()
            .filter(|entry| !is_tombstone_value(&entry.value))
            .map(|entry| decode_readback(&entry.key, &entry.value))
            .collect()
    }
}

fn validate_state(state: &GraphCollectionGenerationState) -> Result<()> {
    GraphKeyspace::new(&state.collection)?;
    validate_token("generation", &state.generation, 128)?;
    validate_token("command", &state.command, 128)?;
    if let Some(reason) = &state.reason {
        validate_token("reason", reason, 1024)?;
    }
    for (key, value) in &state.detail {
        validate_token("detail key", key, 128)?;
        validate_token("detail value", value, 1024)?;
    }
    Ok(())
}

fn validate_token(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > max_bytes || bytes.iter().any(|byte| *byte < 0x20) {
        return Err(CalyxError {
            code: "CALYX_GRAPH_COLLECTION_LIFECYCLE_INVALID",
            message: format!("{name} must be printable and 1..={max_bytes} bytes"),
            remediation: "write a valid graph collection lifecycle state row",
        });
    }
    Ok(())
}

fn decode_readback(key: &[u8], value: &[u8]) -> Result<GraphCollectionGenerationReadback> {
    let state = serde_json::from_slice::<GraphCollectionGenerationState>(value)
        .map_err(lifecycle_corrupt)?;
    validate_state(&state)?;
    Ok(GraphCollectionGenerationReadback {
        state,
        key_sha256: sha256_hex(key),
        value_sha256: sha256_hex(value),
        value_bytes: value.len(),
    })
}

fn lifecycle_corrupt(error: impl std::fmt::Display) -> CalyxError {
    CalyxError {
        code: "CALYX_GRAPH_COLLECTION_LIFECYCLE_CORRUPT",
        message: format!("graph collection lifecycle row is corrupt: {error}"),
        remediation: "repair or tombstone the graph collection generation state before using it",
    }
}

fn state_row_name(collection: &str, generation: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(collection.as_bytes());
    hasher.update([0xff]);
    hasher.update(generation.as_bytes());
    format!("generation:{:x}", hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
