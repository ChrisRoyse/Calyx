//! Per-collection retention policy and TTL sweep support (PH61 T03).

use std::collections::HashMap;

use crate::cf::ColumnFamily;
use crate::erase::{EraseRegistry, EraseResult, EraseScope, erase};
use crate::vault::{AsterVault, VaultContext, encode};
use calyx_core::{CalyxError, Clock, CxId, Result, Ts};
use serde::{Deserialize, Serialize};

/// Metadata key that associates a constellation with a retention collection.
pub const METADATA_COLLECTION: &str = "collection";
/// Metadata key containing the constellation ingest timestamp in Calyx `Ts` units.
pub const METADATA_INGESTED_AT: &str = "ingested_at";

const MILLIS_PER_SEC: u64 = 1_000;

/// Per-collection data minimization policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub collection: String,
    /// Zero means retain indefinitely.
    pub ttl_secs: u64,
    /// Future cold-tier aggregation hook. PH61 logs when it becomes due.
    pub rollup_after_secs: Option<u64>,
}

/// In-memory retention policy registry keyed by collection name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionStore {
    policies: HashMap<String, RetentionPolicy>,
}

impl RetentionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_policy(&mut self, policy: RetentionPolicy) {
        self.policies.insert(policy.collection.clone(), policy);
    }

    pub fn policy_for(&self, collection: &str) -> Option<&RetentionPolicy> {
        self.policies.get(collection)
    }

    pub fn len(&self) -> usize {
        self.policies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn apply_retention(
        &self,
        vault_ctx: &mut VaultContext,
        store: &RetentionStore,
        registry: &EraseRegistry,
        now: Ts,
    ) -> Result<Vec<EraseResult>> {
        apply_retention(self, vault_ctx, store, registry, now)
    }
}

pub fn is_expired(ingested_at: Ts, policy: &RetentionPolicy, now: Ts) -> bool {
    if policy.ttl_secs == 0 {
        return false;
    }
    let ttl = policy.ttl_secs.saturating_mul(MILLIS_PER_SEC);
    now.saturating_sub(ingested_at) > ttl
}

pub fn scan_expired_cxs<C>(
    vault: &AsterVault<C>,
    vault_ctx: &VaultContext,
    store: &RetentionStore,
    now: Ts,
) -> Result<Vec<(CxId, String)>>
where
    C: Clock,
{
    if vault_ctx.vault_id() != vault.vault_id() {
        return Err(CalyxError::vault_access_denied(
            "retention VaultContext belongs to another vault",
        ));
    }
    let snapshot = vault.latest_seq();
    let mut expired = Vec::new();
    for (_key, base) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let cx = encode::decode_constellation_base(&base)?;
        if cx.vault_id != vault_ctx.vault_id() {
            continue;
        }
        let Some(collection) = cx.metadata_value(METADATA_COLLECTION) else {
            continue;
        };
        let Some(policy) = store.policy_for(collection) else {
            continue;
        };
        let ingested_at = parse_ingested_at(cx.cx_id, cx.metadata_value(METADATA_INGESTED_AT))?;
        if is_expired(ingested_at, policy, now) {
            expired.push((cx.cx_id, collection.to_string()));
        } else if is_rollup_due(ingested_at, policy, now) {
            eprintln!("calyx retention rollup not yet implemented for collection {collection}");
        }
    }
    expired.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    Ok(expired)
}

pub fn apply_retention<C>(
    vault: &AsterVault<C>,
    vault_ctx: &mut VaultContext,
    store: &RetentionStore,
    registry: &EraseRegistry,
    now: Ts,
) -> Result<Vec<EraseResult>>
where
    C: Clock,
{
    let expired = scan_expired_cxs(vault, vault_ctx, store, now)?;
    let mut results = Vec::new();
    let mut first_error = None;
    for (cx_id, collection) in expired {
        match erase(vault, EraseScope::Cx(cx_id), vault_ctx, registry) {
            Ok(result) => results.push(result),
            Err(error) if error.code == "CALYX_ERASE_ALREADY_TOMBSTONED" => {
                eprintln!(
                    "calyx retention skipped already tombstoned cx {cx_id} in collection {collection}: {}",
                    error.message
                );
            }
            Err(error) => {
                eprintln!(
                    "calyx retention erase failed for cx {cx_id} in collection {collection}: {error}"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(results)
    }
}

fn parse_ingested_at(cx_id: CxId, value: Option<&str>) -> Result<Ts> {
    let raw = value.ok_or_else(|| retention_metadata_error(cx_id, "missing ingested_at"))?;
    raw.parse::<Ts>()
        .map_err(|_| retention_metadata_error(cx_id, format!("invalid ingested_at `{raw}`")))
}

fn is_rollup_due(ingested_at: Ts, policy: &RetentionPolicy, now: Ts) -> bool {
    policy.rollup_after_secs.is_some_and(|secs| {
        secs != 0 && now.saturating_sub(ingested_at) > secs.saturating_mul(MILLIS_PER_SEC)
    })
}

fn retention_metadata_error(cx_id: CxId, detail: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(format!(
        "retention metadata for cx {cx_id} is invalid: {}",
        detail.into()
    ))
}

#[cfg(test)]
mod tests;
