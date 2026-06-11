use calyx_aster::cf::full_content_hash;
use calyx_core::Result;
use serde::{Deserialize, Serialize};

use super::{
    HEAD_KEY_PREFIX, HeadKind, HeadReadback, ONLINE_HEAD_TAG, OnlineHead, STATE_KEY_HASH_SEED,
    invalid_row, validate_head,
};
use crate::ArtifactKey;

#[derive(Serialize, Deserialize)]
struct OnlineHeadRow {
    tag: String,
    head: OnlineHead,
}

pub fn head_key(kind: HeadKind) -> Vec<u8> {
    let mut key = HEAD_KEY_PREFIX.to_vec();
    key.extend_from_slice(kind.key().as_bytes());
    key
}

pub fn encode_online_head(head: &OnlineHead) -> Result<Vec<u8>> {
    validate_head(head)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(
        &OnlineHeadRow {
            tag: ONLINE_HEAD_TAG.to_string(),
            head: head.clone(),
        },
        &mut bytes,
    )
    .map_err(|error| invalid_row(format!("encode anneal_heads row: {error}")))?;
    Ok(bytes)
}

pub fn decode_online_head(bytes: &[u8]) -> Result<OnlineHead> {
    let row: OnlineHeadRow = ciborium::de::from_reader(bytes)
        .map_err(|error| invalid_row(format!("decode anneal_heads row: {error}")))?;
    if row.tag != ONLINE_HEAD_TAG {
        return Err(invalid_row("anneal_heads row has invalid tag"));
    }
    validate_head(&row.head)?;
    Ok(row.head)
}

pub fn decode_head_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Vec<HeadReadback>> {
    let mut decoded = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        decoded.push(HeadReadback {
            key,
            head: decode_online_head(&value)?,
            value,
        });
    }
    decoded.sort_by_key(|row| row.head.kind.key());
    Ok(decoded)
}

pub fn head_state_artifact_key() -> ArtifactKey {
    ArtifactKey::ConfigCache(full_content_hash([STATE_KEY_HASH_SEED]))
}

pub(crate) fn encode_head_rows(heads: &[OnlineHead]) -> Result<Vec<(HeadKind, Vec<u8>)>> {
    heads
        .iter()
        .map(|head| Ok((head.kind, encode_online_head(head)?)))
        .collect()
}

pub(crate) fn heads_hash(heads: Vec<OnlineHead>) -> [u8; 32] {
    let mut parts = Vec::with_capacity(heads.len() + 1);
    parts.push(STATE_KEY_HASH_SEED.to_vec());
    for head in heads {
        parts.push(encode_online_head(&head).unwrap_or_default());
    }
    full_content_hash(parts.iter().map(Vec::as_slice))
}
