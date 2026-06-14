//! Blob layer: chunked payload + manifest.
//!
//! Large payloads are split into fixed-size chunks, each its own CF row, and a
//! single manifest row records the chunk count, total byte count, BLAKE3
//! content hash, and a cold-tier flag. All rows live in the `cf/blob` column
//! family under the `0x05` key-space discriminant.
//!
//! **Durability ordering.** Chunks are committed first (one group-commit WAL
//! flush); the manifest is committed *last* in a second flush. The manifest is
//! the commit point — a crash between the two leaves orphan chunks with no live
//! manifest, so [`BlobLayer::blob_get`] sees no blob rather than partial data.
//! Orphan chunks are reclaimed by the PH58 janitor. This is the content-
//! addressed "write blobs, reference by manifest last" pattern used by Ollama,
//! Docker, restic, and the AT Protocol.
//!
//! **Verification on read.** `blob_get` re-hashes the reassembled payload and
//! fails closed (`CALYX_ASTER_CORRUPT_SHARD`) on any mismatch, so silent
//! corruption is impossible.

use calyx_core::{CalyxError, Clock, Modality, Result, Seq};

use crate::cf::{ColumnFamily, KeyRange};
use crate::collection::{
    CALYX_INVALID_ARGUMENT, Collection, CollectionMode, collection_has_lens,
    ingest_collection_constellation,
};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};

/// Returned when a payload exceeds the hard per-blob ceiling.
pub const CALYX_BLOB_TOO_LARGE: &str = "CALYX_BLOB_TOO_LARGE";

const DISC_BLOB: u8 = 0x05;
const KIND_CHUNK: u8 = 0x00;
const KIND_MANIFEST: u8 = 0x01;
const BLOB_ID_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

/// Fixed chunk size (256 KiB). Immutable once a vault has written its first
/// blob, so reads can address chunks by index without per-blob metadata.
pub const BLOB_CHUNK_SIZE: usize = 262_144;
/// Hard ceiling on a single blob (1 GiB) — fail closed above this.
pub const MAX_BLOB_BYTES: usize = 1 << 30;
/// Legacy `total_bytes (8) | chunk_count (4) | content_hash (32) | cold_tier (1)`.
const MANIFEST_VALUE_BYTES_V1: usize = 8 + 4 + HASH_BYTES + 1;
/// Current manifest appends `created_at_ms (8)` for retention decisions.
const MANIFEST_VALUE_BYTES: usize = MANIFEST_VALUE_BYTES_V1 + 8;

/// 16-byte content-or-caller-assigned blob identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlobId([u8; BLOB_ID_BYTES]);

impl BlobId {
    pub const fn from_bytes(bytes: [u8; BLOB_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != BLOB_ID_BYTES {
            return Err(invalid_argument(format!(
                "blob id must be {BLOB_ID_BYTES} bytes"
            )));
        }
        let mut out = [0_u8; BLOB_ID_BYTES];
        out.copy_from_slice(bytes);
        Ok(Self(out))
    }

    pub fn from_text(value: &str) -> Self {
        let hash = blake3::hash(value.as_bytes());
        let mut out = [0_u8; BLOB_ID_BYTES];
        out.copy_from_slice(&hash.as_bytes()[..BLOB_ID_BYTES]);
        Self(out)
    }

    pub const fn as_bytes(&self) -> &[u8; BLOB_ID_BYTES] {
        &self.0
    }
}

/// Decoded manifest row — the per-blob source of truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobManifest {
    pub total_bytes: u64,
    pub chunk_count: u32,
    pub content_hash: [u8; HASH_BYTES],
    pub cold_tier: bool,
    /// Unix milliseconds from the vault clock. `None` marks a legacy manifest.
    pub created_at_ms: Option<u64>,
}

/// `(blob_id) -> chunked payload + manifest` layer over a `Blob` collection.
pub struct BlobLayer<'a, C: Clock> {
    vault: &'a AsterVault<C>,
}

impl<'a, C: Clock> BlobLayer<'a, C> {
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }

    /// Stores `data` under `blob_id`. Chunks are committed first, then the
    /// manifest in a separate commit, so a partial failure never leaves a live
    /// manifest pointing at missing chunks. Returns the manifest commit seq.
    pub fn blob_put(&self, col: &Collection, blob_id: BlobId, data: &[u8]) -> Result<Seq> {
        if collection_has_lens(col) {
            if data.len() > MAX_BLOB_BYTES {
                return Err(blob_too_large(data.len()));
            }
            let len = (data.len() as u64).to_be_bytes();
            let content_hash = blake3::hash(data);
            let parts = [
                ("blob_id", blob_id.as_bytes().as_slice()),
                ("total_bytes", len.as_slice()),
                ("content_hash", content_hash.as_bytes().as_slice()),
                ("payload", data),
            ];
            return ingest_collection_constellation(
                self.vault,
                col,
                "blob",
                &parts,
                Modality::Mixed,
            );
        }
        require_blob_mode(col)?;
        if data.len() > MAX_BLOB_BYTES {
            return Err(blob_too_large(data.len()));
        }
        let chunks: Vec<&[u8]> = if data.is_empty() {
            Vec::new()
        } else {
            data.chunks(BLOB_CHUNK_SIZE).collect()
        };
        let chunk_count = u32::try_from(chunks.len())
            .map_err(|_| invalid_argument("blob chunk count overflowed u32"))?;

        // Phase 1: all chunk rows, durable before the manifest exists.
        if !chunks.is_empty() {
            let chunk_rows = chunks.iter().enumerate().map(|(idx, bytes)| {
                (
                    ColumnFamily::Blob,
                    chunk_key(col, blob_id, idx as u32),
                    bytes.to_vec(),
                )
            });
            self.vault.write_cf_batch(chunk_rows)?;
        }

        // Phase 2: the manifest is the commit point, with the Ledger entry.
        let content_hash = *blake3::hash(data).as_bytes();
        let manifest = BlobManifest {
            total_bytes: data.len() as u64,
            chunk_count,
            content_hash,
            cold_tier: false,
            created_at_ms: Some(self.vault.clock_now()),
        };
        let key = manifest_key(col, blob_id);
        let value = encode_manifest(&manifest);
        let subject = ledger_subject(&key);
        let payload = ledger_payload(col, blob_id, &manifest);
        self.vault.write_cf_batch_with_ledger_entry(
            [(ColumnFamily::Blob, key, value)],
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-blob".to_string()),
        )
    }

    /// Reads the manifest only, without reassembling the payload.
    pub fn blob_manifest(&self, col: &Collection, blob_id: BlobId) -> Result<Option<BlobManifest>> {
        require_blob_mode(col)?;
        self.vault
            .read_cf_at(
                self.vault.latest_seq(),
                ColumnFamily::Blob,
                &manifest_key(col, blob_id),
            )?
            .map(|bytes| decode_manifest(&bytes))
            .transpose()
    }

    /// Reassembles and returns the full payload, or `None` if there is no live
    /// manifest. Fails closed if a chunk is missing or the content hash does
    /// not match.
    pub fn blob_get(&self, col: &Collection, blob_id: BlobId) -> Result<Option<Vec<u8>>> {
        let Some(manifest) = self.blob_manifest(col, blob_id)? else {
            return Ok(None);
        };
        let mut data = Vec::with_capacity(manifest.total_bytes as usize);
        let snapshot = self.vault.latest_seq();
        for idx in 0..manifest.chunk_count {
            let chunk = self
                .vault
                .read_cf_at(snapshot, ColumnFamily::Blob, &chunk_key(col, blob_id, idx))?
                .ok_or_else(|| {
                    corrupt(format!(
                        "blob manifest claims {} chunks but chunk {idx} is missing",
                        manifest.chunk_count
                    ))
                })?;
            data.extend_from_slice(&chunk);
        }
        if data.len() as u64 != manifest.total_bytes {
            return Err(corrupt(format!(
                "blob reassembled to {} bytes but manifest says {}",
                data.len(),
                manifest.total_bytes
            )));
        }
        if blake3::hash(&data).as_bytes() != &manifest.content_hash {
            return Err(corrupt(
                "blob content hash mismatch on read — payload is corrupt",
            ));
        }
        Ok(Some(data))
    }

    /// Tombstones every chunk row and the manifest in one batch. A subsequent
    /// `blob_get` reads back as absent. No-op (returns latest seq) if the blob
    /// does not exist.
    pub fn blob_delete(&self, col: &Collection, blob_id: BlobId) -> Result<Seq> {
        let Some(manifest) = self.blob_manifest(col, blob_id)? else {
            return Ok(self.vault.latest_seq());
        };
        let mut rows = Vec::with_capacity(manifest.chunk_count as usize + 1);
        for idx in 0..manifest.chunk_count {
            rows.push((
                ColumnFamily::Blob,
                chunk_key(col, blob_id, idx),
                tombstone_value(),
            ));
        }
        rows.push((
            ColumnFamily::Blob,
            manifest_key(col, blob_id),
            tombstone_value(),
        ));
        let key = manifest_key(col, blob_id);
        let subject = ledger_subject(&key);
        let payload = ledger_payload(col, blob_id, &manifest);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-blob".to_string()),
        )
    }

    /// Lazy chunk iterator for streaming large blobs without a full in-memory
    /// load. Returns an empty stream if the blob is absent. Manifest-read
    /// errors surface here (we wrap in `Result` rather than swallow them).
    pub fn blob_stream_chunks(
        &self,
        col: &Collection,
        blob_id: BlobId,
    ) -> Result<BlobChunkStream<'_, C>> {
        let chunk_count = self
            .blob_manifest(col, blob_id)?
            .map_or(0, |manifest| manifest.chunk_count);
        Ok(BlobChunkStream {
            vault: self.vault,
            chunk_prefix: chunk_prefix(col, blob_id),
            chunk_count,
            next_idx: 0,
        })
    }
}

/// Lazy per-chunk iterator returned by [`BlobLayer::blob_stream_chunks`].
pub struct BlobChunkStream<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    chunk_prefix: Vec<u8>,
    chunk_count: u32,
    next_idx: u32,
}

impl<C: Clock> Iterator for BlobChunkStream<'_, C> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_idx >= self.chunk_count {
            return None;
        }
        let idx = self.next_idx;
        self.next_idx += 1;
        let mut key = self.chunk_prefix.clone();
        key.extend_from_slice(&idx.to_be_bytes());
        match self
            .vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::Blob, &key)
        {
            Ok(Some(bytes)) => Some(Ok(bytes)),
            Ok(None) => Some(Err(corrupt(format!(
                "blob manifest claims {} chunks but chunk {idx} is missing",
                self.chunk_count
            )))),
            Err(error) => Some(Err(error)),
        }
    }
}

/// Stable per-collection id scoping blob rows. Distinct hash domain from the
/// other layers so cross-mode collisions are impossible.
pub fn collection_id(col: &Collection) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx:blob:collection:v1");
    hasher.update(&col.tenant.0.to_be_bytes());
    hasher.update(&(col.name.len() as u16).to_be_bytes());
    hasher.update(col.name.as_bytes());
    u64::from_be_bytes(hasher.finalize().as_bytes()[0..8].try_into().unwrap())
}

/// `0x05 | 0x00 | cid | blob_id | chunk_idx`.
pub fn chunk_key(col: &Collection, blob_id: BlobId, idx: u32) -> Vec<u8> {
    let mut key = chunk_prefix(col, blob_id);
    key.extend_from_slice(&idx.to_be_bytes());
    key
}

/// `0x05 | 0x01 | cid | blob_id`.
pub fn manifest_key(col: &Collection, blob_id: BlobId) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + 8 + BLOB_ID_BYTES);
    key.push(DISC_BLOB);
    key.push(KIND_MANIFEST);
    key.extend_from_slice(&collection_id(col).to_be_bytes());
    key.extend_from_slice(blob_id.as_bytes());
    key
}

fn chunk_prefix(col: &Collection, blob_id: BlobId) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + 8 + BLOB_ID_BYTES + 4);
    key.push(DISC_BLOB);
    key.push(KIND_CHUNK);
    key.extend_from_slice(&collection_id(col).to_be_bytes());
    key.extend_from_slice(blob_id.as_bytes());
    key
}

fn encode_manifest(manifest: &BlobManifest) -> Vec<u8> {
    let mut out = Vec::with_capacity(MANIFEST_VALUE_BYTES);
    out.extend_from_slice(&manifest.total_bytes.to_be_bytes());
    out.extend_from_slice(&manifest.chunk_count.to_be_bytes());
    out.extend_from_slice(&manifest.content_hash);
    out.push(u8::from(manifest.cold_tier));
    out.extend_from_slice(&manifest.created_at_ms.unwrap_or(0).to_be_bytes());
    out
}

fn decode_manifest(bytes: &[u8]) -> Result<BlobManifest> {
    if !matches!(bytes.len(), MANIFEST_VALUE_BYTES_V1 | MANIFEST_VALUE_BYTES) {
        return Err(corrupt(format!(
            "blob manifest must be {MANIFEST_VALUE_BYTES_V1} or {MANIFEST_VALUE_BYTES} bytes, got {}",
            bytes.len()
        )));
    }
    let total_bytes = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    let chunk_count = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let mut content_hash = [0_u8; HASH_BYTES];
    content_hash.copy_from_slice(&bytes[12..44]);
    let cold_tier = match bytes[44] {
        0 => false,
        1 => true,
        other => {
            return Err(corrupt(format!(
                "blob manifest cold_tier byte {other} is not 0/1"
            )));
        }
    };
    let created_at_ms = if bytes.len() == MANIFEST_VALUE_BYTES {
        Some(u64::from_be_bytes(bytes[45..53].try_into().unwrap()))
    } else {
        None
    };
    Ok(BlobManifest {
        total_bytes,
        chunk_count,
        content_hash,
        cold_tier,
        created_at_ms,
    })
}

/// Half-open range covering every row (chunks + manifest) of one blob. Used by
/// callers that need to scan a blob's physical footprint.
pub fn blob_row_range(col: &Collection, blob_id: BlobId) -> KeyRange {
    crate::cf::prefix_range(&{
        let mut prefix = Vec::with_capacity(1 + 8 + BLOB_ID_BYTES);
        prefix.push(DISC_BLOB);
        // Spans both KIND_CHUNK (0x00) and KIND_MANIFEST (0x01) for this blob.
        prefix.push(KIND_CHUNK);
        prefix.extend_from_slice(&collection_id(col).to_be_bytes());
        prefix.extend_from_slice(blob_id.as_bytes());
        prefix
    })
}

fn require_blob_mode(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::Blob {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "blob layer requires a Blob collection, got {:?}",
            col.mode
        )))
    }
}

fn ledger_subject(manifest_key: &[u8]) -> SubjectId {
    SubjectId::Query(blake3::hash(manifest_key).as_bytes().to_vec())
}

fn ledger_payload(col: &Collection, blob_id: BlobId, manifest: &BlobManifest) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("collection_id", format!("{:016x}", collection_id(col)))
        .insert_str("blob_id", hex_bytes(blob_id.as_bytes()))
        .insert_str("total_bytes", manifest.total_bytes.to_string())
        .insert_str("chunk_count", manifest.chunk_count.to_string())
        .insert_str("content_hash", hex_bytes(&manifest.content_hash));
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn blob_too_large(len: usize) -> CalyxError {
    CalyxError {
        code: CALYX_BLOB_TOO_LARGE,
        message: format!("blob of {len} bytes exceeds the {MAX_BLOB_BYTES}-byte ceiling"),
        remediation: "split the payload or raise MAX_BLOB_BYTES",
    }
}

fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "fix the blob input",
    }
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

#[cfg(test)]
mod tests;
