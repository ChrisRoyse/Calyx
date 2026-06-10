use calyx_core::{CalyxError, Result};

use crate::rollback::{
    ArtifactKey, ArtifactPtr, ArtifactSnapshot, CALYX_ANNEAL_INVALID_ROLLBACK_STATE, ChangeId,
};

const SNAPSHOT_MAGIC: &[u8; 4] = b"ARS1";
const LIVE_MAGIC: &[u8; 4] = b"ARL1";
pub(crate) const CHANGE_PREFIX: &[u8] = b"change:";
pub(crate) const LIVE_PREFIX: &[u8] = b"live:";

pub fn rollback_snapshot_key(change_id: ChangeId) -> Vec<u8> {
    let mut out = Vec::with_capacity(CHANGE_PREFIX.len() + 8);
    out.extend_from_slice(CHANGE_PREFIX);
    out.extend_from_slice(&change_id.0.to_be_bytes());
    out
}

pub fn rollback_live_key(key: &ArtifactKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(LIVE_PREFIX.len() + 33);
    out.extend_from_slice(LIVE_PREFIX);
    encode_artifact_key(&mut out, key);
    out
}

pub(crate) fn snapshot_row(snapshot: &ArtifactSnapshot) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        rollback_snapshot_key(snapshot.change_id),
        encode_snapshot_value(snapshot)?,
    ))
}

pub(crate) fn encode_live_value(ptr: &ArtifactPtr) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(LIVE_MAGIC);
    encode_artifact_ptr(&mut out, ptr)?;
    Ok(out)
}

pub(crate) fn decode_live_value(bytes: &[u8]) -> Result<ArtifactPtr> {
    let mut dec = Decoder::new(bytes);
    dec.magic(LIVE_MAGIC)?;
    let ptr = decode_artifact_ptr(&mut dec)?;
    dec.finish()?;
    Ok(ptr)
}

pub(crate) fn decode_live_key(bytes: &[u8]) -> Result<ArtifactKey> {
    if !bytes.starts_with(LIVE_PREFIX) {
        return Err(codec_error("rollback live key has invalid prefix"));
    }
    let mut dec = Decoder::new(&bytes[LIVE_PREFIX.len()..]);
    let key = decode_artifact_key(&mut dec)?;
    dec.finish()?;
    Ok(key)
}

pub(crate) fn decode_snapshot_value(bytes: &[u8]) -> Result<ArtifactSnapshot> {
    let mut dec = Decoder::new(bytes);
    dec.magic(SNAPSHOT_MAGIC)?;
    let change_id = ChangeId(dec.u64()?);
    let ts = dec.u64()?;
    let flags = dec.byte()?;
    let key = decode_artifact_key(&mut dec)?;
    let prior_ptr = decode_artifact_ptr(&mut dec)?;
    let candidate_ptr = decode_artifact_ptr(&mut dec)?;
    let description = dec.string()?;
    dec.finish()?;
    Ok(ArtifactSnapshot {
        change_id,
        key,
        prior_ptr,
        candidate_ptr,
        ts,
        description,
        promoted: flags & 0b001 != 0,
        reverted: flags & 0b010 != 0,
        committed: flags & 0b100 != 0,
    })
}

fn encode_snapshot_value(snapshot: &ArtifactSnapshot) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(SNAPSHOT_MAGIC);
    put_u64(&mut out, snapshot.change_id.0);
    put_u64(&mut out, snapshot.ts);
    out.push(flags(snapshot));
    encode_artifact_key(&mut out, &snapshot.key);
    encode_artifact_ptr(&mut out, &snapshot.prior_ptr)?;
    encode_artifact_ptr(&mut out, &snapshot.candidate_ptr)?;
    put_str(&mut out, &snapshot.description)?;
    Ok(out)
}

fn flags(snapshot: &ArtifactSnapshot) -> u8 {
    u8::from(snapshot.promoted)
        | (u8::from(snapshot.reverted) << 1)
        | (u8::from(snapshot.committed) << 2)
}

fn encode_artifact_key(out: &mut Vec<u8>, key: &ArtifactKey) {
    match key {
        ArtifactKey::ConfigCache(hash) => put_hash(out, 0, hash),
        ArtifactKey::HnswGraph(hash) => put_hash(out, 1, hash),
        ArtifactKey::QuantLevel(hash) => put_hash(out, 2, hash),
    }
}

fn decode_artifact_key(dec: &mut Decoder<'_>) -> Result<ArtifactKey> {
    Ok(match dec.byte()? {
        0 => ArtifactKey::ConfigCache(dec.hash()?),
        1 => ArtifactKey::HnswGraph(dec.hash()?),
        2 => ArtifactKey::QuantLevel(dec.hash()?),
        _ => return Err(codec_error("unknown rollback artifact key tag")),
    })
}

fn encode_artifact_ptr(out: &mut Vec<u8>, ptr: &ArtifactPtr) -> Result<()> {
    match ptr {
        ArtifactPtr::ConfigCacheKeyHash(hash) => put_hash(out, 0, hash),
        ArtifactPtr::HnswGraphPath(path) => {
            out.push(1);
            put_str(out, path)?;
        }
        ArtifactPtr::QuantLevelRecordHash(hash) => put_hash(out, 2, hash),
    }
    Ok(())
}

fn decode_artifact_ptr(dec: &mut Decoder<'_>) -> Result<ArtifactPtr> {
    Ok(match dec.byte()? {
        0 => ArtifactPtr::ConfigCacheKeyHash(dec.hash()?),
        1 => ArtifactPtr::HnswGraphPath(dec.string()?),
        2 => ArtifactPtr::QuantLevelRecordHash(dec.hash()?),
        _ => return Err(codec_error("unknown rollback artifact pointer tag")),
    })
}

fn put_hash(out: &mut Vec<u8>, tag: u8, hash: &[u8; 32]) {
    out.push(tag);
    out.extend_from_slice(hash);
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let len = u32::try_from(bytes.len())
        .map_err(|_| codec_error("rollback string is too large to encode"))?;
    put_u32(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}

struct Decoder<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn magic(&mut self, magic: &[u8]) -> Result<()> {
        if self.take(magic.len())? == magic {
            Ok(())
        } else {
            Err(codec_error("rollback row has invalid magic"))
        }
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u64(&mut self) -> Result<u64> {
        let mut value = [0; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(value))
    }

    fn hash(&mut self) -> Result<[u8; 32]> {
        let mut value = [0; 32];
        value.copy_from_slice(self.take(32)?);
        Ok(value)
    }

    fn string(&mut self) -> Result<String> {
        let mut len = [0; 4];
        len.copy_from_slice(self.take(4)?);
        let bytes = self.take(u32::from_be_bytes(len) as usize)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|error| codec_error(format!("rollback row string is not UTF-8: {error}")))
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| codec_error("rollback row cursor overflow"))?;
        if end > self.bytes.len() {
            return Err(codec_error("rollback row ended early"));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn finish(&self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(codec_error("rollback row has trailing bytes"))
        }
    }
}

fn codec_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_INVALID_ROLLBACK_STATE,
        message: message.into(),
        remediation: "repair anneal_rollback CF rows before continuing Anneal",
    }
}
