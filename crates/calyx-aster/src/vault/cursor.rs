use calyx_core::{CalyxError, Result};
use std::str;

pub(super) struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub(super) fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("encoded cursor offset overflow"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("encoded bytes truncated"))?;
        self.offset = end;
        Ok(slice)
    }

    pub(super) fn bytes_prefixed(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        self.bytes(len)
    }

    pub(super) fn string(&mut self) -> Result<String> {
        let bytes = self.bytes_prefixed()?;
        str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|error| CalyxError::aster_corrupt_shard(format!("utf8 decode: {error}")))
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| CalyxError::aster_corrupt_shard("encoded array width mismatch"))
    }

    pub(super) fn u8(&mut self) -> Result<u8> {
        Ok(*self
            .bytes(1)?
            .first()
            .ok_or_else(|| CalyxError::aster_corrupt_shard("missing u8"))?)
    }

    pub(super) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(super) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}
