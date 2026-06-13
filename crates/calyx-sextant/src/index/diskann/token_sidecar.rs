//! Binary sidecars for token DiskANN MaxSim indexes.

use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result};
use memmap2::Mmap;

use crate::error::{CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO, sextant_error};

const DOCS_MAGIC: [u8; 8] = *b"CLXTOKD1";
const TOKEN_DOCS_MAGIC: [u8; 8] = *b"CLXTOKM1";
const TOKEN_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DocSegment {
    pub(super) cx_id: CxId,
    pub(super) start: u32,
    pub(super) len: u32,
}

pub(super) fn write_docs(
    path: &Path,
    token_dim: u32,
    token_count: u32,
    docs: &[DocSegment],
) -> Result<()> {
    let mut out = BufWriter::new(File::create(path).map_err(|e| io("create docs sidecar", e))?);
    out.write_all(&DOCS_MAGIC)
        .map_err(|e| io("write docs magic", e))?;
    out.write_all(&TOKEN_FORMAT_VERSION.to_le_bytes())
        .map_err(|e| io("write docs version", e))?;
    out.write_all(&token_dim.to_le_bytes())
        .map_err(|e| io("write docs token_dim", e))?;
    out.write_all(&(docs.len() as u64).to_le_bytes())
        .map_err(|e| io("write docs count", e))?;
    out.write_all(&(token_count as u64).to_le_bytes())
        .map_err(|e| io("write token count", e))?;
    for doc in docs {
        out.write_all(doc.cx_id.as_bytes())
            .map_err(|e| io("write doc cx", e))?;
        out.write_all(&doc.start.to_le_bytes())
            .map_err(|e| io("write doc start", e))?;
        out.write_all(&doc.len.to_le_bytes())
            .map_err(|e| io("write doc len", e))?;
    }
    sync(out, "docs")
}

pub(super) fn read_docs(path: &Path) -> Result<(u32, usize, Vec<DocSegment>)> {
    let bytes = fs::read(path).map_err(|e| io("read docs sidecar", e))?;
    if bytes.len() < 32 || bytes[0..8] != DOCS_MAGIC {
        return Err(invalid("bad token docs sidecar header"));
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4B"));
    if version != TOKEN_FORMAT_VERSION {
        return Err(invalid(format!("token docs version {version}")));
    }
    let token_dim = u32::from_le_bytes(bytes[12..16].try_into().expect("4B"));
    let doc_count = u64::from_le_bytes(bytes[16..24].try_into().expect("8B")) as usize;
    let token_count = u64::from_le_bytes(bytes[24..32].try_into().expect("8B")) as usize;
    let expected = 32 + doc_count * 24;
    if bytes.len() != expected {
        return Err(invalid(format!(
            "docs sidecar len {} != {expected}",
            bytes.len()
        )));
    }
    let mut docs = Vec::with_capacity(doc_count);
    for chunk in bytes[32..].chunks_exact(24) {
        let mut id = [0_u8; 16];
        id.copy_from_slice(&chunk[..16]);
        docs.push(DocSegment {
            cx_id: CxId::from_bytes(id),
            start: u32::from_le_bytes(chunk[16..20].try_into().expect("4B")),
            len: u32::from_le_bytes(chunk[20..24].try_into().expect("4B")),
        });
    }
    Ok((token_dim, token_count, docs))
}

pub(super) fn write_token_docs(path: &Path, token_docs: &[u32]) -> Result<()> {
    let mut out = BufWriter::new(File::create(path).map_err(|e| io("create token map", e))?);
    out.write_all(&TOKEN_DOCS_MAGIC)
        .map_err(|e| io("write token map magic", e))?;
    out.write_all(&TOKEN_FORMAT_VERSION.to_le_bytes())
        .map_err(|e| io("write token map version", e))?;
    out.write_all(&(token_docs.len() as u64).to_le_bytes())
        .map_err(|e| io("write token map count", e))?;
    for doc in token_docs {
        out.write_all(&doc.to_le_bytes())
            .map_err(|e| io("write token map doc", e))?;
    }
    sync(out, "token map")
}

pub(super) fn read_token_docs(path: &Path, expected_count: usize) -> Result<Vec<u32>> {
    let bytes = fs::read(path).map_err(|e| io("read token map", e))?;
    if bytes.len() < 20 || bytes[0..8] != TOKEN_DOCS_MAGIC {
        return Err(invalid("bad token map sidecar header"));
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4B"));
    let count = u64::from_le_bytes(bytes[12..20].try_into().expect("8B")) as usize;
    if version != TOKEN_FORMAT_VERSION || count != expected_count || bytes.len() != 20 + count * 4 {
        return Err(invalid("token map sidecar metadata mismatch"));
    }
    Ok(bytes[20..]
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("4B")))
        .collect())
}

pub(super) fn write_tokens(path: &Path, rows: &[(CxId, Vec<Vec<f32>>)]) -> Result<()> {
    let mut out = BufWriter::new(File::create(path).map_err(|e| io("create tokens sidecar", e))?);
    for (_, tokens) in rows {
        for token in tokens {
            for value in token {
                out.write_all(&value.to_le_bytes())
                    .map_err(|e| io("write token f32", e))?;
            }
        }
    }
    sync(out, "tokens")
}

pub(super) fn map_tokens(path: &Path, token_count: usize, token_dim: u32) -> Result<Mmap> {
    let file = File::open(path).map_err(|e| io("open tokens sidecar", e))?;
    let expected = token_count * token_dim as usize * 4;
    let actual = file
        .metadata()
        .map_err(|e| io("stat tokens sidecar", e))?
        .len() as usize;
    if actual != expected {
        return Err(sextant_error(
            CALYX_INDEX_IO,
            format!("tokens sidecar is {actual} B, expected {expected} B"),
        ));
    }
    unsafe { Mmap::map(&file).map_err(|e| io("mmap tokens sidecar", e)) }
}

pub(super) fn read_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("4B")))
        .collect()
}

pub(super) fn graph_path(root: &Path) -> PathBuf {
    root.join("graph.cda")
}

pub(super) fn docs_path(root: &Path) -> PathBuf {
    root.join("docs.cdt")
}

pub(super) fn token_docs_path(root: &Path) -> PathBuf {
    root.join("token_docs.u32")
}

pub(super) fn tokens_path(root: &Path) -> PathBuf {
    root.join("tokens.f32")
}

pub(super) fn token_cx(idx: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&(idx as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn sync(out: BufWriter<File>, what: &str) -> Result<()> {
    out.into_inner()
        .map_err(|e| io(format!("flush {what}"), e.into_error()))?
        .sync_all()
        .map_err(|e| io(format!("fsync {what}"), e))
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("token diskann invalid params: {detail}"),
    )
}

fn io(stage: impl std::fmt::Display, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("token diskann {stage}: {error}"))
}
