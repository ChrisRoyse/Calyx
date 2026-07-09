use calyx_core::{CalyxError, Clock, LensId, Modality, Result, Seq};
use serde_json::json;

use super::{
    Collection, CollectionMode, PanelRef, collection_key, encode_collection, invalid_argument,
};
use crate::cf::ColumnFamily;
use crate::vault::AsterVault;

pub const CALYX_COLLECTION_LENS_DUPLICATE: &str = "CALYX_COLLECTION_LENS_DUPLICATE";
pub const CALYX_LENS_NOT_FOUND: &str = "CALYX_LENS_NOT_FOUND";
pub const CALYX_COLLECTION_LENS_UNMEASURED: &str = "CALYX_COLLECTION_LENS_UNMEASURED";

const BACKFILL_PENDING_PREFIX: &[u8] = b"backfill\0";
const COLLECTION_ID_DOMAIN: &[u8] = b"calyx:collection:metadata:v1";
const LENS_REGISTRY_PREFIX: &[u8] = b"lens\0";
const LENS_REGISTRATION_KIND: &str = "calyx_collection_lens_registration_v1";
const LENS_SLOT_CONTRACT: &str = "measured_slots_required";

pub fn collection_has_lens(collection: &Collection) -> bool {
    collection
        .panel
        .as_ref()
        .is_some_and(|panel| !panel.lenses.is_empty())
}

pub fn add_lens<C>(vault: &AsterVault<C>, collection_name: &str, lens_id: LensId) -> Result<()>
where
    C: Clock,
{
    let key = collection_key(collection_name)?;
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Collections, &key)?
        .ok_or_else(|| collection_not_found(collection_name))?;
    let mut collection = super::decode_collection(&bytes)?;

    if collection.mode == CollectionMode::Constellations || collection_has_lens(&collection) {
        return Err(duplicate_lens(collection_name, lens_id));
    }
    if !lens_registered(vault, lens_id)? {
        return Err(lens_not_found(lens_id));
    }

    collection.mode = CollectionMode::Constellations;
    collection.panel = Some(PanelRef::new(lens_id));
    let updated = encode_collection(&collection)?;
    let marker_key = backfill_pending_key(collection_name)?;
    let marker_value = backfill_marker_value(collection_name, lens_id)?;
    vault.write_cf_batch([
        (ColumnFamily::Collections, key, updated),
        (ColumnFamily::Online, marker_key, marker_value),
    ])?;
    Ok(())
}

pub fn register_lens<C>(vault: &AsterVault<C>, lens_id: LensId) -> Result<()>
where
    C: Clock,
{
    let value = serde_json::to_vec(&json!({
        "kind": LENS_REGISTRATION_KIND,
        "lens_id": lens_id.to_string(),
        "slot_contract": LENS_SLOT_CONTRACT,
        "status": "registered"
    }))
    .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode lens marker: {error}")))?;
    vault.write_cf(ColumnFamily::Online, lens_registry_key(lens_id), value)?;
    Ok(())
}

pub fn collection_id(collection_name: &str) -> Result<u64> {
    let key = collection_key(collection_name)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(COLLECTION_ID_DOMAIN);
    hasher.update(&(key.len() as u64).to_be_bytes());
    hasher.update(&key);
    Ok(u64::from_be_bytes(
        hasher.finalize().as_bytes()[0..8].try_into().unwrap(),
    ))
}

pub fn backfill_pending_key(collection_name: &str) -> Result<Vec<u8>> {
    let id = collection_id(collection_name)?;
    let mut key = Vec::with_capacity(BACKFILL_PENDING_PREFIX.len() + 8);
    key.extend_from_slice(BACKFILL_PENDING_PREFIX);
    key.extend_from_slice(&id.to_be_bytes());
    Ok(key)
}

pub fn lens_registry_key(lens_id: LensId) -> Vec<u8> {
    let mut key = Vec::with_capacity(LENS_REGISTRY_PREFIX.len() + lens_id.as_bytes().len());
    key.extend_from_slice(LENS_REGISTRY_PREFIX);
    key.extend_from_slice(lens_id.as_bytes());
    key
}

pub fn ingest_collection_constellation<C>(
    _vault: &AsterVault<C>,
    collection: &Collection,
    _layer: &str,
    _parts: &[(&str, &[u8])],
    _modality: Modality,
) -> Result<Seq>
where
    C: Clock,
{
    let panel = collection
        .panel
        .as_ref()
        .filter(|panel| !panel.lenses.is_empty())
        .ok_or_else(|| invalid_argument("constellation ingest requires a collection lens"))?;
    Err(lens_unmeasured(collection, panel.lenses.len()))
}

fn lens_registered<C>(vault: &AsterVault<C>, lens_id: LensId) -> Result<bool>
where
    C: Clock,
{
    let Some(row) = vault.read_cf_at(
        vault.latest_seq(),
        ColumnFamily::Online,
        &lens_registry_key(lens_id),
    )?
    else {
        return Ok(false);
    };
    let marker: serde_json::Value = serde_json::from_slice(&row)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode lens marker: {error}")))?;
    let expected_lens = lens_id.to_string();
    Ok(
        marker.get("kind").and_then(serde_json::Value::as_str) == Some(LENS_REGISTRATION_KIND)
            && marker.get("lens_id").and_then(serde_json::Value::as_str)
                == Some(expected_lens.as_str())
            && marker
                .get("slot_contract")
                .and_then(serde_json::Value::as_str)
                == Some(LENS_SLOT_CONTRACT)
            && marker.get("status").and_then(serde_json::Value::as_str) == Some("registered"),
    )
}

fn backfill_marker_value(collection_name: &str, lens_id: LensId) -> Result<Vec<u8>> {
    serde_json::to_vec(&json!({
        "kind": "backfill_pending",
        "collection": collection_name,
        "collection_id": format!("{:016x}", collection_id(collection_name)?),
        "lens_id": lens_id.to_string(),
        "status": "pending"
    }))
    .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode backfill marker: {error}")))
}

fn collection_not_found(name: &str) -> CalyxError {
    collection_error(
        super::CALYX_COLLECTION_NOT_FOUND,
        format!("collection `{name}` was not found"),
        "create the collection before adding a lens",
    )
}

fn duplicate_lens(name: &str, lens_id: LensId) -> CalyxError {
    collection_error(
        CALYX_COLLECTION_LENS_DUPLICATE,
        format!("collection `{name}` is already upgraded with lens `{lens_id}`"),
        "read the existing collection panel before adding another lens",
    )
}

fn lens_not_found(lens_id: LensId) -> CalyxError {
    collection_error(
        CALYX_LENS_NOT_FOUND,
        format!("lens `{lens_id}` is not registered"),
        "register the lens before adding it to a collection",
    )
}

fn lens_unmeasured(collection: &Collection, lens_count: usize) -> CalyxError {
    collection_error(
        CALYX_COLLECTION_LENS_UNMEASURED,
        format!(
            "collection `{}` has {lens_count} registered lens(es), but no measured slot vectors were provided",
            collection.name
        ),
        "ingest through a measured lens pipeline before writing constellation collection rows",
    )
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
