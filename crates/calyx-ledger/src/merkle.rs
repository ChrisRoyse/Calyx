//! Merkle roots and signed export bundles for Ledger ranges.

use std::collections::BTreeMap;
use std::ops::Range;

use calyx_core::{CalyxError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::append::LedgerCfStore;
use crate::codec::decode;

const HASH_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;

pub const MERKLE_EMPTY_ROOT: [u8; HASH_BYTES] = [0; HASH_BYTES];
pub const MERKLE_SIGNING_DOMAIN: &[u8] = b"calyx-ledger-root-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleExportBundle {
    pub range_start: u64,
    pub range_end: u64,
    pub root: [u8; HASH_BYTES],
    #[serde(with = "signature_serde")]
    pub signature: Option<[u8; SIGNATURE_BYTES]>,
    pub signer_pubkey: Option<[u8; HASH_BYTES]>,
}

impl MerkleExportBundle {
    pub fn unsigned(range: Range<u64>, root: [u8; HASH_BYTES]) -> Self {
        Self {
            range_start: range.start,
            range_end: range.end,
            root,
            signature: None,
            signer_pubkey: None,
        }
    }

    pub fn signed(
        range: Range<u64>,
        root: [u8; HASH_BYTES],
        signing_key: &[u8; HASH_BYTES],
    ) -> Self {
        let key = SigningKey::from_bytes(signing_key);
        let signature = sign_root(range.clone(), &root, signing_key);
        Self {
            range_start: range.start,
            range_end: range.end,
            root,
            signature: Some(signature),
            signer_pubkey: Some(key.verifying_key().to_bytes()),
        }
    }
}

pub fn leaf_hash(entry_hash: &[u8; HASH_BYTES]) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"leaf");
    hasher.update(entry_hash);
    *hasher.finalize().as_bytes()
}

pub fn combine_hash(left: &[u8; HASH_BYTES], right: &[u8; HASH_BYTES]) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"node");
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

pub fn merkle_root_of_hashes(entry_hashes: &[[u8; HASH_BYTES]]) -> [u8; HASH_BYTES] {
    if entry_hashes.is_empty() {
        return MERKLE_EMPTY_ROOT;
    }

    let mut level: Vec<_> = entry_hashes.iter().map(leaf_hash).collect();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            let last = *level.last().expect("non-empty level");
            level.push(last);
        }
        level = level
            .chunks_exact(2)
            .map(|pair| combine_hash(&pair[0], &pair[1]))
            .collect();
    }
    level[0]
}

pub fn merkle_root(store: &dyn LedgerCfStore, range: Range<u64>) -> Result<[u8; HASH_BYTES]> {
    if range.start > range.end {
        return Err(CalyxError::ledger_corrupt(format!(
            "invalid ledger range {}..{}",
            range.start, range.end
        )));
    }
    if range.start == range.end {
        return Ok(MERKLE_EMPTY_ROOT);
    }

    let mut rows = BTreeMap::new();
    for row in store.scan()? {
        if range.contains(&row.seq) && rows.insert(row.seq, row.bytes).is_some() {
            return Err(CalyxError::ledger_corrupt(format!(
                "duplicate ledger seq {}",
                row.seq
            )));
        }
    }

    let mut hashes = Vec::with_capacity(range_len(&range)?);
    for seq in range.clone() {
        let bytes = rows.get(&seq).ok_or_else(|| {
            CalyxError::ledger_corrupt(format!("missing ledger row for seq {seq}"))
        })?;
        let entry = decode(bytes)?;
        if entry.seq != seq {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger row seq {seq} encodes seq {}",
                entry.seq
            )));
        }
        hashes.push(entry.entry_hash);
    }
    Ok(merkle_root_of_hashes(&hashes))
}

pub fn sign_root(
    range: Range<u64>,
    root: &[u8; HASH_BYTES],
    signing_key: &[u8; HASH_BYTES],
) -> [u8; SIGNATURE_BYTES] {
    let key = SigningKey::from_bytes(signing_key);
    let signature: Signature = key.sign(&signing_message(range, root));
    signature.to_bytes()
}

pub fn verify_signature(bundle: &MerkleExportBundle) -> bool {
    let (Some(signature), Some(pubkey)) = (&bundle.signature, &bundle.signer_pubkey) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    let signature = Signature::from_bytes(signature);
    key.verify(
        &signing_message(bundle.range_start..bundle.range_end, &bundle.root),
        &signature,
    )
    .is_ok()
}

fn signing_message(range: Range<u64>, root: &[u8; HASH_BYTES]) -> Vec<u8> {
    let mut message = Vec::with_capacity(MERKLE_SIGNING_DOMAIN.len() + 16 + HASH_BYTES);
    message.extend_from_slice(MERKLE_SIGNING_DOMAIN);
    message.extend_from_slice(&range.start.to_be_bytes());
    message.extend_from_slice(&range.end.to_be_bytes());
    message.extend_from_slice(root);
    message
}

fn range_len(range: &Range<u64>) -> Result<usize> {
    let len = range.end.checked_sub(range.start).ok_or_else(|| {
        CalyxError::ledger_corrupt(format!(
            "invalid ledger range {}..{}",
            range.start, range.end
        ))
    })?;
    usize::try_from(len).map_err(|_| {
        CalyxError::ledger_corrupt(format!(
            "ledger range {}..{} is too large for this host",
            range.start, range.end
        ))
    })
}

mod signature_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::SIGNATURE_BYTES;

    pub fn serialize<S>(
        value: &Option<[u8; SIGNATURE_BYTES]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(bytes) => serializer.serialize_some(bytes.as_slice()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<[u8; SIGNATURE_BYTES]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<Vec<u8>>::deserialize(deserializer)?;
        let Some(bytes) = value else {
            return Ok(None);
        };
        if bytes.len() != SIGNATURE_BYTES {
            return Err(serde::de::Error::custom(format!(
                "signature has {} bytes, expected {SIGNATURE_BYTES}",
                bytes.len()
            )));
        }
        let mut signature = [0; SIGNATURE_BYTES];
        signature.copy_from_slice(&bytes);
        Ok(Some(signature))
    }
}
