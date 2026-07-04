//! Sharded persisted-CSR storage (#996).
//!
//! A multi-million-edge assoc graph produces a CSR projection far larger than
//! any single CF row may be (the real #877 corpus graph encodes to ~157 MB,
//! observed to fail the router memtable cap loudly). The persisted layout is
//! therefore: the `KIND_CSR` row holds a small JSON manifest (version, counts,
//! segment count, byte total, blake3 of the stream) and the binary-encoded
//! `PlainGraphCsr` stream is chunked into ordered `KIND_CSR_SEGMENT` rows.
//! Readers reassemble the stream, verify length and hash, then decode —
//! any missing/torn/stale segment state fails closed as `graph_corrupt`,
//! never as a silently partial graph. Version 4 stores raw CxId bytes plus a
//! dictionary of edge types; older manifests must be rebuilt instead of being
//! interpreted as current graph evidence.

use std::collections::BTreeMap;

use calyx_core::{CxId, Result, Seq};
use serde::{Deserialize, Serialize};

use super::key::{GraphKeyspace, graph_corrupt, validate_edge_type};
use super::types::{PlainGraphCsr, PlainGraphCsrEdge, validate_plain_graph_csr_weight};

pub(super) const CSR_MANIFEST_VERSION: u32 = 4;
/// Segment payload cap. Keeps every row within the graph value ceiling and
/// far below the router memtable cap so segment writes never backpressure.
pub(super) const CSR_SEGMENT_MAX_BYTES: usize = 1 << 20;
const CSR_BINARY_MAGIC: &[u8; 8] = b"CALYXCSR";
const CSR_BINARY_VERSION: u32 = 1;
const CXID_BYTES: usize = 16;
const EDGE_BYTES: usize = CXID_BYTES + 4 + 4;

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
    let stream = encode_csr_binary(projection)?;
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
        // Single-row projections have no schema manifest; decode will reject
        // old unweighted rows because PlainGraphCsrEdge now requires weight.
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
    let csr = if manifest.is_some() {
        decode_csr_binary(&stream)?
    } else {
        serde_json::from_slice::<PlainGraphCsr>(&stream)
            .map_err(|error| graph_corrupt(format!("decode legacy CSR projection: {error}")))?
    };
    if let Some(manifest) = manifest
        && (csr.collection != manifest.collection
            || csr.source_snapshot != manifest.source_snapshot
            || csr.nodes.len() != manifest.node_count
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

fn encode_csr_binary(projection: &PlainGraphCsr) -> Result<Vec<u8>> {
    let mut type_to_index = BTreeMap::<&str, u32>::new();
    let mut type_table = Vec::<&str>::new();
    for edge in &projection.edges {
        validate_edge_type(&edge.edge_type)?;
        validate_plain_graph_csr_weight(edge.weight)?;
        if !type_to_index.contains_key(edge.edge_type.as_str()) {
            let index = u32::try_from(type_table.len())
                .map_err(|_| graph_corrupt("CSR edge-type table overflows u32"))?;
            type_to_index.insert(edge.edge_type.as_str(), index);
            type_table.push(edge.edge_type.as_str());
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(CSR_BINARY_MAGIC);
    put_u32(&mut out, CSR_BINARY_VERSION);
    put_string(&mut out, &projection.collection)?;
    put_u64(&mut out, projection.source_snapshot);
    put_len(&mut out, projection.nodes.len(), "node count")?;
    put_len(&mut out, projection.offsets.len(), "offset count")?;
    put_len(&mut out, projection.edges.len(), "edge count")?;
    put_len(
        &mut out,
        projection.association_edge_count,
        "association edge count",
    )?;
    put_len_u32(&mut out, type_table.len(), "edge-type table length")?;
    for edge_type in &type_table {
        put_string(&mut out, edge_type)?;
    }
    for node in &projection.nodes {
        out.extend_from_slice(node.as_bytes());
    }
    for offset in &projection.offsets {
        put_len(&mut out, *offset, "CSR offset")?;
    }
    for edge in &projection.edges {
        out.extend_from_slice(edge.dst.as_bytes());
        let index = type_to_index
            .get(edge.edge_type.as_str())
            .ok_or_else(|| graph_corrupt("CSR edge type was absent from dictionary"))?;
        put_u32(&mut out, *index);
        out.extend_from_slice(&edge.weight.to_le_bytes());
    }
    Ok(out)
}

fn decode_csr_binary(bytes: &[u8]) -> Result<PlainGraphCsr> {
    let mut reader = BinaryReader::new(bytes);
    reader.expect_bytes(CSR_BINARY_MAGIC, "CSR binary magic")?;
    let version = reader.read_u32("CSR binary version")?;
    if version != CSR_BINARY_VERSION {
        return Err(graph_corrupt(format!(
            "persisted CSR binary version {version} is not supported (expected {CSR_BINARY_VERSION})"
        )));
    }
    let collection = reader.read_string("collection")?;
    GraphKeyspace::new(&collection)?;
    let source_snapshot = reader.read_u64("source snapshot")?;
    let node_count = reader.read_len("node count")?;
    let offset_count = reader.read_len("offset count")?;
    let edge_count = reader.read_len("edge count")?;
    let association_edge_count = reader.read_len("association edge count")?;
    let type_count = reader.read_u32("edge-type table length")? as usize;
    let mut type_table = Vec::new();
    for _ in 0..type_count {
        let edge_type = reader.read_string("edge type")?;
        validate_edge_type(&edge_type)?;
        type_table.push(edge_type);
    }
    reader.ensure_remaining(node_count, CXID_BYTES, "node array")?;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        nodes.push(reader.read_cxid("node id")?);
    }
    reader.ensure_remaining(offset_count, 8, "offset array")?;
    let mut offsets = Vec::with_capacity(offset_count);
    for _ in 0..offset_count {
        offsets.push(reader.read_len("CSR offset")?);
    }
    reader.ensure_remaining(edge_count, EDGE_BYTES, "edge array")?;
    let mut edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        let dst = reader.read_cxid("edge dst")?;
        let type_index = reader.read_u32("edge type index")? as usize;
        let edge_type = type_table
            .get(type_index)
            .ok_or_else(|| graph_corrupt(format!("CSR edge type index {type_index} is absent")))?
            .clone();
        let weight = validate_plain_graph_csr_weight(reader.read_f32("edge weight")?)?;
        edges.push(PlainGraphCsrEdge {
            dst,
            edge_type,
            weight,
        });
    }
    reader.finish()?;
    Ok(PlainGraphCsr {
        collection,
        source_snapshot,
        nodes,
        offsets,
        edges,
        association_edge_count,
    })
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    put_len_u32(out, value.len(), "string length")?;
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn put_len(out: &mut Vec<u8>, value: usize, field: &str) -> Result<()> {
    let value =
        u64::try_from(value).map_err(|_| graph_corrupt(format!("{field} overflows u64")))?;
    put_u64(out, value);
    Ok(())
}

fn put_len_u32(out: &mut Vec<u8>, value: usize, field: &str) -> Result<()> {
    let value =
        u32::try_from(value).map_err(|_| graph_corrupt(format!("{field} overflows u32")))?;
    put_u32(out, value);
    Ok(())
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct BinaryReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn expect_bytes(&mut self, expected: &[u8], field: &str) -> Result<()> {
        let actual = self.take(expected.len(), field)?;
        if actual == expected {
            Ok(())
        } else {
            Err(graph_corrupt(format!("{field} mismatch")))
        }
    }

    fn read_string(&mut self, field: &str) -> Result<String> {
        let len = self.read_u32(field)? as usize;
        let raw = self.take(len, field)?;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|error| graph_corrupt(format!("{field} is not UTF-8: {error}")))
    }

    fn read_cxid(&mut self, field: &str) -> Result<CxId> {
        let raw = self.take(CXID_BYTES, field)?;
        let mut bytes = [0_u8; CXID_BYTES];
        bytes.copy_from_slice(raw);
        Ok(CxId::from_bytes(bytes))
    }

    fn read_len(&mut self, field: &str) -> Result<usize> {
        usize::try_from(self.read_u64(field)?)
            .map_err(|_| graph_corrupt(format!("{field} overflows usize")))
    }

    fn read_u64(&mut self, field: &str) -> Result<u64> {
        let raw = self.take(8, field)?;
        Ok(u64::from_le_bytes(
            raw.try_into().expect("slice len checked"),
        ))
    }

    fn read_u32(&mut self, field: &str) -> Result<u32> {
        let raw = self.take(4, field)?;
        Ok(u32::from_le_bytes(
            raw.try_into().expect("slice len checked"),
        ))
    }

    fn read_f32(&mut self, field: &str) -> Result<f32> {
        let raw = self.take(4, field)?;
        Ok(f32::from_le_bytes(
            raw.try_into().expect("slice len checked"),
        ))
    }

    fn ensure_remaining(&self, count: usize, width: usize, field: &str) -> Result<()> {
        let required = count
            .checked_mul(width)
            .ok_or_else(|| graph_corrupt(format!("{field} byte count overflows usize")))?;
        if self.remaining() >= required {
            Ok(())
        } else {
            Err(graph_corrupt(format!(
                "{field} needs {required} bytes but only {} remain",
                self.remaining()
            )))
        }
    }

    fn finish(&self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(graph_corrupt(format!(
                "CSR binary stream has {} trailing bytes",
                self.remaining()
            )))
        }
    }

    fn take(&mut self, len: usize, field: &str) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| graph_corrupt(format!("{field} offset overflows usize")))?;
        let raw = self.bytes.get(self.offset..end).ok_or_else(|| {
            graph_corrupt(format!(
                "{field} is truncated at byte {} needing {len} bytes",
                self.offset
            ))
        })?;
        self.offset = end;
        Ok(raw)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}
