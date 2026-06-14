use std::collections::BTreeMap;

use calyx_core::{
    AbsentReason, CalyxError, Clock, Constellation, CxFlags, InputRef, LedgerRef, LensId, Modality,
    Result, Seq, SlotId, SlotVector, VaultStore,
};
use serde_json::json;

use super::{
    Collection, CollectionMode, PanelRef, collection_key, encode_collection, invalid_argument,
};
use crate::cf::ColumnFamily;
use crate::vault::AsterVault;

pub const CALYX_COLLECTION_LENS_DUPLICATE: &str = "CALYX_COLLECTION_LENS_DUPLICATE";
pub const CALYX_LENS_NOT_FOUND: &str = "CALYX_LENS_NOT_FOUND";

const BACKFILL_PENDING_PREFIX: &[u8] = b"backfill\0";
const COLLECTION_ID_DOMAIN: &[u8] = b"calyx:collection:metadata:v1";
const COLLECTION_INPUT_DOMAIN: &[u8] = b"calyx:collection-constellation-input:v1";
const LENS_REGISTRY_PREFIX: &[u8] = b"lens\0";

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
        "kind": "ph53_lens_registry_stub",
        "lens_id": lens_id.to_string(),
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
    vault: &AsterVault<C>,
    collection: &Collection,
    layer: &str,
    parts: &[(&str, &[u8])],
    modality: Modality,
) -> Result<Seq>
where
    C: Clock,
{
    let panel = collection
        .panel
        .as_ref()
        .filter(|panel| !panel.lenses.is_empty())
        .ok_or_else(|| invalid_argument("constellation ingest requires a collection lens"))?;
    let input = constellation_input(collection, layer, parts);
    let mut slots = BTreeMap::new();
    for (index, _) in panel.lenses.iter().enumerate() {
        let slot = u16::try_from(index).map_err(|_| {
            invalid_argument("collection panel has more lenses than addressable slot ids")
        })?;
        slots.insert(
            SlotId::new(slot),
            SlotVector::Absent {
                reason: AbsentReason::Deferred,
            },
        );
    }
    let constellation = Constellation {
        cx_id: vault.cx_id_for_input(&input, panel.panel_version),
        vault_id: vault.vault_id(),
        panel_version: panel.panel_version,
        created_at: vault.clock_now(),
        input_ref: InputRef {
            hash: *blake3::hash(&input).as_bytes(),
            pointer: None,
            redacted: false,
        },
        modality,
        slots,
        scalars: BTreeMap::new(),
        metadata: constellation_metadata(collection, layer, panel),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    };
    vault.put(constellation)?;
    Ok(vault.latest_seq())
}

fn lens_registered<C>(vault: &AsterVault<C>, lens_id: LensId) -> Result<bool>
where
    C: Clock,
{
    vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Online,
            &lens_registry_key(lens_id),
        )
        .map(|row| row.is_some())
}

fn constellation_input(collection: &Collection, layer: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut input = Vec::new();
    append_part(&mut input, "domain", COLLECTION_INPUT_DOMAIN);
    append_part(&mut input, "collection", collection.name.as_bytes());
    append_part(&mut input, "layer", layer.as_bytes());
    for (label, value) in parts {
        append_part(&mut input, label, value);
    }
    input
}

fn append_part(out: &mut Vec<u8>, label: &str, value: &[u8]) {
    out.extend_from_slice(&(label.len() as u64).to_be_bytes());
    out.extend_from_slice(label.as_bytes());
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}

fn constellation_metadata(
    collection: &Collection,
    layer: &str,
    panel: &PanelRef,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("collection".to_string(), collection.name.clone());
    metadata.insert("collection_layer".to_string(), layer.to_string());
    metadata.insert("panel_version".to_string(), panel.panel_version.to_string());
    metadata.insert(
        "lenses".to_string(),
        panel
            .lenses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    metadata
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
