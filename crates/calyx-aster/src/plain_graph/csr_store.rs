//! Sharded persisted-CSR storage (#996).
//!
//! A multi-million-edge assoc graph produces a CSR projection far larger than
//! any single CF row may be (the real #877 corpus graph encodes to ~157 MB,
//! observed to fail the router memtable cap loudly). The persisted layout is
//! therefore: the `KIND_CSR` row holds a small JSON manifest (version, counts,
//! segment count, byte total, blake3 of the stream) and the JSON-encoded
//! `PlainGraphCsr` bytes are chunked into ordered `KIND_CSR_SEGMENT` rows.
//! Readers reassemble the stream, verify length and hash, then decode —
//! any missing/torn/stale segment state fails closed as `graph_corrupt`,
//! never as a silently partial graph. Legacy single-row CSRs (written before
//! sharding) are still readable: a row that does not decode as a manifest is
//! decoded directly as `PlainGraphCsr`.

use calyx_core::{Result, Seq};
use serde::{Deserialize, Serialize};

use super::key::{GraphKeyspace, graph_corrupt};
use super::types::PlainGraphCsr;

pub(super) const CSR_MANIFEST_VERSION: u32 = 2;
/// Segment payload cap. Keeps every row within the graph value ceiling and
/// far below the router memtable cap so segment writes never backpressure.
pub(super) const CSR_SEGMENT_MAX_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CsrManifest {
    pub(super) csr_manifest_version: u32,
    pub(super) collection: String,
    pub(super) source_snapshot: Seq,
    pub(super) node_count: usize,
    pub(super) edge_count: usize,
    pub(super) association_edge_count: usize,
    pub(super) segment_count: u32,
    pub(super) total_bytes: usize,
    pub(super) stream_blake3: String,
}

/// Encode a projection into (manifest row bytes, ordered segment payloads).
pub(super) fn encode_csr_segments(
    keys: &GraphKeyspace,
    projection: &PlainGraphCsr,
) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    let stream = serde_json::to_vec(projection)
        .map_err(|error| graph_corrupt(format!("encode CSR projection: {error}")))?;
    let segments: Vec<Vec<u8>> = stream
        .chunks(CSR_SEGMENT_MAX_BYTES)
        .map(<[u8]>::to_vec)
        .collect();
    let segment_count = u32::try_from(segments.len())
        .map_err(|_| graph_corrupt("CSR projection segment count overflows u32"))?;
    let manifest = CsrManifest {
        csr_manifest_version: CSR_MANIFEST_VERSION,
        collection: keys.collection_name(),
        source_snapshot: projection.source_snapshot,
        node_count: projection.nodes.len(),
        edge_count: projection.edges.len(),
        association_edge_count: projection.association_edge_count,
        segment_count,
        total_bytes: stream.len(),
        stream_blake3: blake3::hash(&stream).to_hex().to_string(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| graph_corrupt(format!("encode CSR manifest: {error}")))?;
    Ok((manifest_bytes, segments))
}

/// Reassemble the persisted CSR byte stream through `get`. Returns `None`
/// when no CSR row exists at all.
pub(super) fn load_csr_bytes(
    keys: &GraphKeyspace,
    get: impl Fn(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Option<Vec<u8>>> {
    let Some(row) = get(&keys.csr_key())? else {
        return Ok(None);
    };
    let Some(manifest) = decode_manifest(&row) else {
        // Legacy single-row CSR: the row bytes are the projection itself.
        return Ok(Some(row));
    };
    if manifest.csr_manifest_version != CSR_MANIFEST_VERSION {
        return Err(graph_corrupt(format!(
            "persisted CSR manifest version {} is not supported (expected {CSR_MANIFEST_VERSION})",
            manifest.csr_manifest_version
        )));
    }
    let mut stream = Vec::with_capacity(manifest.total_bytes);
    for ordinal in 0..manifest.segment_count {
        let segment = get(&keys.csr_segment_key(ordinal))?.ok_or_else(|| {
            graph_corrupt(format!(
                "persisted CSR segment {ordinal}/{} is missing for collection={}",
                manifest.segment_count, manifest.collection
            ))
        })?;
        stream.extend_from_slice(&segment);
    }
    if stream.len() != manifest.total_bytes {
        return Err(graph_corrupt(format!(
            "persisted CSR stream is {} bytes but manifest declares {} for collection={}",
            stream.len(),
            manifest.total_bytes,
            manifest.collection
        )));
    }
    let stream_hash = blake3::hash(&stream).to_hex().to_string();
    if stream_hash != manifest.stream_blake3 {
        return Err(graph_corrupt(format!(
            "persisted CSR stream hash {stream_hash} does not match manifest {} for collection={}",
            manifest.stream_blake3, manifest.collection
        )));
    }
    Ok(Some(stream))
}

/// Load and decode the persisted CSR, validating manifest counts.
pub(super) fn load_csr(
    keys: &GraphKeyspace,
    get: impl Fn(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Option<PlainGraphCsr>> {
    let manifest = get(&keys.csr_key())?.and_then(|row| decode_manifest(&row));
    let Some(stream) = load_csr_bytes(keys, get)? else {
        return Ok(None);
    };
    let csr: PlainGraphCsr = serde_json::from_slice(&stream)
        .map_err(|error| graph_corrupt(format!("decode CSR projection: {error}")))?;
    if let Some(manifest) = manifest
        && (csr.nodes.len() != manifest.node_count
            || csr.edges.len() != manifest.edge_count
            || csr.association_edge_count != manifest.association_edge_count)
    {
        return Err(graph_corrupt(format!(
            "persisted CSR decode disagrees with manifest for collection={}: decoded nodes={} edges={} assoc_edges={}, manifest nodes={} edges={} assoc_edges={}",
            manifest.collection,
            csr.nodes.len(),
            csr.edges.len(),
            csr.association_edge_count,
            manifest.node_count,
            manifest.edge_count,
            manifest.association_edge_count
        )));
    }
    Ok(Some(csr))
}

fn decode_manifest(row: &[u8]) -> Option<CsrManifest> {
    // A legacy row decodes as PlainGraphCsr and lacks csr_manifest_version,
    // so manifest decode fails and the caller treats the row as v1 bytes.
    serde_json::from_slice::<CsrManifest>(row).ok()
}
