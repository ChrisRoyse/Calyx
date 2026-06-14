//! Collection descriptors for PH53 collections-as-any-model.

mod enhancement;
mod policy;
mod schema;

use bincode::config;
use calyx_core::{CalyxError, Clock, Result};
use serde::{Deserialize, Serialize};

use crate::cf::ColumnFamily;
use crate::vault::AsterVault;

pub use enhancement::{
    CALYX_COLLECTION_LENS_DUPLICATE, CALYX_LENS_NOT_FOUND, add_lens, backfill_pending_key,
    collection_has_lens, collection_id, ingest_collection_constellation, lens_registry_key,
    register_lens,
};
pub use policy::{
    DEFAULT_TEMPORAL_BOOST_WEIGHTS, DedupAction, DedupPolicy, IsolationLevel, PanelRef,
    RetentionPolicy, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
};
pub use schema::{FieldDef, FieldType, Schema};

pub const CALYX_COLLECTION_ALREADY_EXISTS: &str = "CALYX_COLLECTION_ALREADY_EXISTS";
pub const CALYX_COLLECTION_NOT_FOUND: &str = "CALYX_COLLECTION_NOT_FOUND";
pub const CALYX_INVALID_ARGUMENT: &str = "CALYX_INVALID_ARGUMENT";
pub const COLLECTION_KEY_PREFIX: &[u8] = b"coll\0";
pub(crate) const MAX_NAME_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionMode {
    Records,
    Documents,
    KV,
    TimeSeries,
    Blob,
    Constellations,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub mode: CollectionMode,
    pub schema: Option<Schema>,
    pub panel: Option<PanelRef>,
    pub indexes: Vec<SecondaryIndexSpec>,
    pub dedup: DedupPolicy,
    pub temporal: TemporalPolicy,
    pub retention: RetentionPolicy,
    pub txn_policy: TxnPolicy,
    pub tenant: TenantId,
}

impl Collection {
    pub fn validate(&self) -> Result<()> {
        schema::validate_name("collection name", &self.name)?;
        if let Some(schema) = &self.schema {
            schema.validate()?;
        }
        match (&self.mode, &self.panel) {
            (CollectionMode::Constellations, None) => {
                return Err(invalid_argument(
                    "Constellations mode requires a non-empty panel reference",
                ));
            }
            (CollectionMode::Constellations, Some(panel)) => panel.validate()?,
            (_, Some(_)) => {
                return Err(invalid_argument(
                    "plain collection modes must not carry a panel reference",
                ));
            }
            (_, None) => {}
        }
        for index in &self.indexes {
            index.validate()?;
        }
        self.dedup.validate()?;
        self.temporal.validate()?;
        self.retention.validate()?;
        self.txn_policy.validate()
    }
}

pub fn create_collection<C>(vault: &AsterVault<C>, collection: Collection) -> Result<()>
where
    C: Clock,
{
    collection.validate()?;
    let key = collection_key(&collection.name)?;
    if vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Collections, &key)?
        .is_some()
    {
        return Err(collection_exists(&collection.name));
    }
    let value = encode_collection(&collection)?;
    vault.write_cf(ColumnFamily::Collections, key, value)?;
    Ok(())
}

pub fn get_collection<C>(vault: &AsterVault<C>, name: &str) -> Result<Collection>
where
    C: Clock,
{
    let key = collection_key(name)?;
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Collections, &key)?
        .ok_or_else(|| collection_not_found(name))?;
    decode_collection(&bytes)
}

pub fn collection_key(name: &str) -> Result<Vec<u8>> {
    schema::validate_name("collection name", name)?;
    let mut key = Vec::with_capacity(COLLECTION_KEY_PREFIX.len() + name.len());
    key.extend_from_slice(COLLECTION_KEY_PREFIX);
    key.extend_from_slice(name.as_bytes());
    Ok(key)
}

pub fn encode_collection(collection: &Collection) -> Result<Vec<u8>> {
    collection.validate()?;
    bincode::serde::encode_to_vec(collection, config::standard())
        .map_err(|error| corrupt_collection_row(format!("encode collection: {error}")))
}

pub fn decode_collection(bytes: &[u8]) -> Result<Collection> {
    let (collection, read): (Collection, usize) =
        bincode::serde::decode_from_slice(bytes, config::standard())
            .map_err(|error| corrupt_collection_row(format!("decode collection: {error}")))?;
    if read != bytes.len() {
        return Err(corrupt_collection_row(format!(
            "collection row has {} trailing bytes",
            bytes.len() - read
        )));
    }
    collection.validate()?;
    Ok(collection)
}

pub(crate) fn invalid_argument(message: impl Into<String>) -> CalyxError {
    collection_error(
        CALYX_INVALID_ARGUMENT,
        message,
        "correct the collection descriptor",
    )
}

fn collection_exists(name: &str) -> CalyxError {
    collection_error(
        CALYX_COLLECTION_ALREADY_EXISTS,
        format!("collection `{name}` already exists"),
        "choose a new collection name or read the existing descriptor",
    )
}

fn collection_not_found(name: &str) -> CalyxError {
    collection_error(
        CALYX_COLLECTION_NOT_FOUND,
        format!("collection `{name}` was not found"),
        "create the collection before reading it",
    )
}

fn corrupt_collection_row(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

fn collection_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

#[cfg(test)]
mod enhancement_tests;
#[cfg(test)]
mod tests;
