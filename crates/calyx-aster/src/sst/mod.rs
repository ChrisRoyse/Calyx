//! Immutable SSTable writer and mmap reader.

pub mod arrow;
mod bloom;
pub mod level;

use crate::mmap_col::MmapColumn;
use bloom::BloomFilter;
use calyx_core::{CalyxError, Result};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"CXS1";
const LEGACY_VERSION: u32 = 1;
const VERSION: u32 = 2;
const HEADER_LEN: usize = 32;
const RECORD_HEADER_LEN: usize = 12;
const INDEX_ENTRY_FIXED_LEN: usize = 12;

/// Metadata returned after writing an SSTable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstSummary {
    pub path: PathBuf,
    pub entries: usize,
    pub bytes: u64,
    pub index_offset: u64,
    pub bloom_offset: u64,
}

/// A key/value row read from an SSTable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    key: Vec<u8>,
    offset: u64,
}

/// Memory-mapped SSTable reader.
#[derive(Debug)]
pub struct SstReader {
    column: MmapColumn,
    index: Vec<IndexEntry>,
    bloom: BloomFilter,
}

/// Writes a sorted immutable SSTable. The input iterator must already be ordered.
pub fn write_sst<'a>(
    path: impl AsRef<Path>,
    entries: impl IntoIterator<Item = (&'a [u8], &'a [u8])>,
) -> Result<SstSummary> {
    let path = path.as_ref();
    let entries: Vec<_> = entries
        .into_iter()
        .map(|(key, value)| (key.to_vec(), value.to_vec()))
        .collect();
    ensure_sorted(&entries)?;

    let mut bytes = vec![0u8; HEADER_LEN];
    let mut index = Vec::with_capacity(entries.len());
    for (key, value) in &entries {
        let offset = bytes.len() as u64;
        write_record(&mut bytes, key, value)?;
        index.push(IndexEntry {
            key: key.clone(),
            offset,
        });
    }

    let index_offset = bytes.len() as u64;
    write_index(&mut bytes, &index);
    let bloom_offset = bytes.len() as u64;
    BloomFilter::from_keys(entries.iter().map(|(key, _)| key.as_slice())).encode(&mut bytes);
    let body_crc = section_crc(&bytes[HEADER_LEN..]);
    write_header(
        &mut bytes,
        entries.len() as u32,
        index_offset,
        bloom_offset,
        body_crc,
    );

    let tmp = path.with_extension("sst.tmp");
    {
        let mut file = File::create(&tmp).map_err(|error| storage_error("create SST", error))?;
        file.write_all(&bytes)
            .map_err(|error| storage_error("write SST", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync SST", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("rename SST", error))?;
    sync_parent(path)?;

    Ok(SstSummary {
        path: path.to_path_buf(),
        entries: entries.len(),
        bytes: bytes.len() as u64,
        index_offset,
        bloom_offset,
    })
}

impl SstReader {
    /// Opens an SSTable through mmap.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let column = MmapColumn::open(path.as_ref())?;
        let bytes = column.as_bytes();
        let header = read_header(bytes)?;
        let index = read_index(
            bytes,
            header.entries,
            header.index_offset,
            header.bloom_offset,
        )?;
        let bloom_bytes = bytes
            .get(header.bloom_offset as usize..)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST bloom offset out of bounds"))?;
        let bloom = BloomFilter::decode(bloom_bytes)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("invalid SST bloom filter"))?;
        Ok(Self {
            column,
            index,
            bloom,
        })
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if !self.bloom.may_contain(key) {
            return Ok(None);
        }
        let Ok(position) = self
            .index
            .binary_search_by(|entry| entry.key.as_slice().cmp(key))
        else {
            return Ok(None);
        };
        Ok(Some(
            read_record(self.column.as_bytes(), self.index[position].offset)?.value,
        ))
    }

    pub fn range(&self, start: &[u8], end: &[u8]) -> Result<Vec<SstEntry>> {
        let start_at = self
            .index
            .partition_point(|entry| entry.key.as_slice() < start);
        let mut rows = Vec::new();
        for entry in &self.index[start_at..] {
            if entry.key.as_slice() >= end {
                break;
            }
            rows.push(read_record(self.column.as_bytes(), entry.offset)?);
        }
        Ok(rows)
    }

    pub fn iter(&self) -> Result<Vec<SstEntry>> {
        self.index
            .iter()
            .map(|entry| read_record(self.column.as_bytes(), entry.offset))
            .collect()
    }

    pub fn bloom_may_contain(&self, key: &[u8]) -> bool {
        self.bloom.may_contain(key)
    }
}

#[derive(Debug, Clone, Copy)]
struct Header {
    entries: u32,
    index_offset: u64,
    bloom_offset: u64,
}

fn ensure_sorted(entries: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
    if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(CalyxError::aster_corrupt_shard(
            "SST entries must be strictly sorted by key",
        ));
    }
    Ok(())
}

fn write_record(out: &mut Vec<u8>, key: &[u8], value: &[u8]) -> Result<()> {
    let key_len = u32::try_from(key.len())
        .map_err(|_| CalyxError::disk_pressure("SST key length exceeds u32"))?;
    let value_len = u32::try_from(value.len())
        .map_err(|_| CalyxError::disk_pressure("SST value length exceeds u32"))?;
    let crc = record_crc(key, value);
    out.extend_from_slice(&key_len.to_le_bytes());
    out.extend_from_slice(&value_len.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(value);
    Ok(())
}

fn read_record(bytes: &[u8], offset: u64) -> Result<SstEntry> {
    let offset = offset as usize;
    let header = bytes
        .get(offset..offset + RECORD_HEADER_LEN)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST record header out of bounds"))?;
    let key_len = u32::from_le_bytes(header[0..4].try_into().expect("key len")) as usize;
    let value_len = u32::from_le_bytes(header[4..8].try_into().expect("value len")) as usize;
    let expected_crc = u32::from_le_bytes(header[8..12].try_into().expect("record crc"));
    let key_start = offset + RECORD_HEADER_LEN;
    let value_start = key_start + key_len;
    let value_end = value_start + value_len;
    let key = bytes
        .get(key_start..value_start)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST key out of bounds"))?;
    let value = bytes
        .get(value_start..value_end)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST value out of bounds"))?;
    let actual_crc = record_crc(key, value);
    if actual_crc != expected_crc {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST record crc mismatch at {offset}: expected {expected_crc:08x}, got {actual_crc:08x}"
        )));
    }
    Ok(SstEntry {
        key: key.to_vec(),
        value: value.to_vec(),
    })
}

fn write_index(out: &mut Vec<u8>, index: &[IndexEntry]) {
    for entry in index {
        out.extend_from_slice(&(entry.key.len() as u32).to_le_bytes());
        out.extend_from_slice(&entry.offset.to_le_bytes());
        out.extend_from_slice(&entry.key);
    }
}

fn read_index(
    bytes: &[u8],
    entries: u32,
    index_offset: u64,
    bloom_offset: u64,
) -> Result<Vec<IndexEntry>> {
    let mut offset = index_offset as usize;
    let end = bloom_offset as usize;
    let mut index = Vec::with_capacity(entries as usize);
    for _ in 0..entries {
        let fixed = bytes
            .get(offset..offset + INDEX_ENTRY_FIXED_LEN)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST index entry out of bounds"))?;
        let key_len = u32::from_le_bytes(fixed[0..4].try_into().expect("index key len")) as usize;
        let record_offset = u64::from_le_bytes(fixed[4..12].try_into().expect("record offset"));
        offset += INDEX_ENTRY_FIXED_LEN;
        let key = bytes
            .get(offset..offset + key_len)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST index key out of bounds"))?
            .to_vec();
        offset += key_len;
        index.push(IndexEntry {
            key,
            offset: record_offset,
        });
    }
    if offset != end {
        return Err(CalyxError::aster_corrupt_shard("SST index length mismatch"));
    }
    Ok(index)
}

fn write_header(
    bytes: &mut [u8],
    entries: u32,
    index_offset: u64,
    bloom_offset: u64,
    body_crc: u32,
) {
    bytes[0..4].copy_from_slice(MAGIC);
    bytes[4..8].copy_from_slice(&VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&entries.to_le_bytes());
    bytes[12..20].copy_from_slice(&index_offset.to_le_bytes());
    bytes[20..28].copy_from_slice(&bloom_offset.to_le_bytes());
    bytes[28..32].copy_from_slice(&body_crc.to_le_bytes());
}

fn read_header(bytes: &[u8]) -> Result<Header> {
    let header = bytes
        .get(0..HEADER_LEN)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST header missing"))?;
    if &header[0..4] != MAGIC {
        return Err(CalyxError::aster_corrupt_shard("SST magic mismatch"));
    }
    let version = u32::from_le_bytes(header[4..8].try_into().expect("version"));
    if version != VERSION && version != LEGACY_VERSION {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "unsupported SST version {version}"
        )));
    }
    let entries = u32::from_le_bytes(header[8..12].try_into().expect("entries"));
    let index_offset = u64::from_le_bytes(header[12..20].try_into().expect("index offset"));
    let bloom_offset = u64::from_le_bytes(header[20..28].try_into().expect("bloom offset"));
    let len = bytes.len() as u64;
    if index_offset < HEADER_LEN as u64
        || index_offset > len
        || bloom_offset < index_offset
        || bloom_offset > len
    {
        return Err(CalyxError::aster_corrupt_shard(
            "SST header offsets out of bounds",
        ));
    }
    if version >= VERSION {
        let expected_crc = u32::from_le_bytes(header[28..32].try_into().expect("body crc"));
        let actual_crc = section_crc(
            bytes
                .get(HEADER_LEN..)
                .ok_or_else(|| CalyxError::aster_corrupt_shard("SST body missing"))?,
        );
        if actual_crc != expected_crc {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST body crc mismatch: expected {expected_crc:08x}, got {actual_crc:08x}"
            )));
        }
    }
    Ok(Header {
        entries,
        index_offset,
        bloom_offset,
    })
}

fn section_crc(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

fn record_crc(key: &[u8], value: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(key);
    hasher.update(value);
    hasher.finalize()
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("SST path has no parent"))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| storage_error("fsync SST directory", error))
}

#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<()> {
    path.parent()
        .ok_or_else(|| CalyxError::disk_pressure("SST path has no parent"))?;
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::Memtable;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn flushed_memtable_reads_back_byte_exact_and_ordered() {
        let dir = test_dir("sst-roundtrip");
        let path = dir.join("000001.sst");
        let mut table = Memtable::new(128);
        table.put(b"k03", b"three").expect("put k03");
        table.put(b"k01", b"one").expect("put k01");
        table.put(b"k02", b"two").expect("put k02");

        let summary = table.flush_to_sst(&path).expect("flush sst");
        let disk_bytes = fs::read(&path).expect("read sst bytes");
        assert_eq!(summary.bytes, disk_bytes.len() as u64);
        assert_eq!(&disk_bytes[0..4], b"CXS1");

        let reader = SstReader::open(&path).expect("open sst");
        assert_eq!(reader.get(b"k02").expect("get k02"), Some(b"two".to_vec()));
        let rows = reader.range(b"k01", b"k04").expect("range");
        let keys: Vec<_> = rows.into_iter().map(|row| row.key).collect();
        assert_eq!(keys, [b"k01".to_vec(), b"k02".to_vec(), b"k03".to_vec()]);
        assert!(reader.bloom_may_contain(b"k01"));
        assert!(reader.bloom_may_contain(b"k02"));
        assert!(reader.bloom_may_contain(b"k03"));
        cleanup(dir);
    }

    #[test]
    fn corrupt_record_crc_fails_closed() {
        let dir = test_dir("sst-corrupt");
        let path = dir.join("000001.sst");
        let mut table = Memtable::new(128);
        table.put(b"k01", b"one").expect("put k01");
        table.flush_to_sst(&path).expect("flush sst");

        let mut bytes = fs::read(&path).expect("read sst");
        bytes[HEADER_LEN + RECORD_HEADER_LEN] ^= 0xff;
        fs::write(&path, bytes).expect("write corrupt sst");
        let error = SstReader::open(&path).expect_err("crc mismatch");

        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(dir);
    }

    #[test]
    fn invalid_header_offsets_fail_closed() {
        let dir = test_dir("sst-bad-header");
        let path = dir.join("000001.sst");
        let mut table = Memtable::new(128);
        table.put(b"k01", b"one").expect("put k01");
        table.flush_to_sst(&path).expect("flush sst");

        let mut bytes = fs::read(&path).expect("read sst");
        bytes[20..28].copy_from_slice(&u64::MAX.to_le_bytes());
        fs::write(&path, bytes).expect("write bad header");
        let error = SstReader::open(&path).expect_err("bad header rejected");

        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(dir);
    }

    #[test]
    fn corrupt_index_section_fails_closed_on_open() {
        let dir = test_dir("sst-corrupt-index");
        let path = dir.join("000001.sst");
        let summary = write_sst(
            &path,
            [
                (b"k01".as_slice(), b"one".as_slice()),
                (b"k02".as_slice(), b"two".as_slice()),
            ],
        )
        .expect("write sst");

        let mut bytes = fs::read(&path).expect("read sst");
        bytes[summary.index_offset as usize + INDEX_ENTRY_FIXED_LEN] ^= 0x01;
        fs::write(&path, bytes).expect("write corrupt index");
        let error = SstReader::open(&path).expect_err("index crc rejected");

        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(dir);
    }

    #[test]
    fn corrupt_bloom_section_fails_closed_on_open() {
        let dir = test_dir("sst-corrupt-bloom");
        let path = dir.join("000001.sst");
        let summary =
            write_sst(&path, [(b"k01".as_slice(), b"one".as_slice())]).expect("write sst");

        let mut bytes = fs::read(&path).expect("read sst");
        bytes[summary.bloom_offset as usize] ^= 0x01;
        fs::write(&path, bytes).expect("write corrupt bloom");
        let error = SstReader::open(&path).expect_err("bloom crc rejected");

        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).expect("cleanup test dir");
    }
}
