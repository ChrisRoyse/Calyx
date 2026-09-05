//! Immutable SSTable writer and mmap reader.

pub mod arrow;
mod bloom;
#[path = "io.rs"]
mod io_helpers;
pub mod level;
mod page;
mod point_read;
mod reader_cache;

pub use reader_cache::{invalidate_reader, shared_reader};

use crate::mmap_col::MmapColumn;
use bloom::BloomFilter;
use calyx_core::{CalyxError, Result};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use io_helpers::{record_crc, section_crc, storage_error, sync_parent};
pub(crate) use point_read::{SstPointReader, SstStreamingReader};

const MAGIC: &[u8; 4] = b"CXS1";
const LEGACY_VERSION: u32 = 1;
const CHECKSUMMED_VERSION: u32 = 2;
const VERSION: u32 = 3;
const HEADER_LEN: usize = 32;
const RECORD_HEADER_LEN: usize = 12;
const INDEX_ENTRY_FIXED_LEN: usize = 12;
const RANGE_METADATA_VERSION: u32 = 1;
const RANGE_METADATA_FIXED_LEN: usize = 24;
const RANGE_METADATA_CRC_LEN: usize = 4;
const RANGE_FOOTER_LEN: usize = 8;
const RANGE_FOOTER_MAGIC: &[u8; 4] = b"CXR3";
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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

/// A key-only SST row view used by latest-read scans that must respect
/// tombstones without cloning record values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstKeyState {
    pub key: Vec<u8>,
    pub is_tombstone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    key: Vec<u8>,
    offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SstLookupMetadata {
    pub(crate) first_key: Vec<u8>,
    pub(crate) last_key: Vec<u8>,
    bloom: BloomFilter,
    index: Vec<IndexEntry>,
}

/// Authoritative key bounds persisted in v3 SSTs. Unlike the full lookup
/// metadata, these bytes can be read and verified from the fixed footer
/// without mapping or checksumming the value body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SstKeyBounds {
    entries: u32,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
}

impl SstKeyBounds {
    pub(crate) fn contains(&self, key: &[u8]) -> bool {
        self.entries != 0 && key >= self.first_key.as_slice() && key <= self.last_key.as_slice()
    }

    pub(crate) fn overlaps(&self, start: &[u8], end: Option<&[u8]>) -> bool {
        self.entries != 0
            && self.last_key.as_slice() >= start
            && end.is_none_or(|end| self.first_key.as_slice() < end)
    }

    pub(crate) fn may_precede(&self, start: &[u8], upper: &[u8], inclusive: bool) -> bool {
        self.entries != 0
            && self.last_key.as_slice() >= start
            && if inclusive {
                self.first_key.as_slice() <= upper
            } else {
                self.first_key.as_slice() < upper
            }
    }
}

impl SstLookupMetadata {
    pub(crate) fn record_offset(&self, key: &[u8]) -> Option<u64> {
        self.index
            .binary_search_by(|entry| entry.key.as_slice().cmp(key))
            .ok()
            .map(|position| self.index[position].offset)
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &[u8]> {
        self.index.iter().map(|entry| entry.key.as_slice())
    }
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
    write_range_metadata(&mut bytes, &entries, &index)?;
    let body_crc = section_crc(&bytes[HEADER_LEN..]);
    write_header(
        &mut bytes,
        entries.len() as u32,
        index_offset,
        bloom_offset,
        body_crc,
    );

    let mut tmp = SstTemp::new(path)?;
    {
        let mut file = crate::private_fs::open_private_temp_file(tmp.path())?;
        file.write_all(&bytes).map_err(|error| {
            storage_error(&format!("write SST {}", tmp.path().display()), error)
        })?;
        file.sync_all().map_err(|error| {
            storage_error(&format!("fsync SST {}", tmp.path().display()), error)
        })?;
    }
    fs::rename(tmp.path(), path).map_err(|error| {
        storage_error(
            &format!("rename SST {} to {}", tmp.path().display(), path.display()),
            error,
        )
    })?;
    tmp.commit();
    sync_parent(path)?;

    Ok(SstSummary {
        path: path.to_path_buf(),
        entries: entries.len(),
        bytes: bytes.len() as u64,
        index_offset,
        bloom_offset,
    })
}

struct SstTemp {
    path: PathBuf,
    committed: bool,
}

impl SstTemp {
    fn new(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| CalyxError::disk_pressure("SST path has no parent"))?;
        let name = path
            .file_name()
            .ok_or_else(|| CalyxError::disk_pressure("SST path has no file name"))?;
        let mut temp_name = OsString::from(name);
        temp_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        Ok(Self {
            path: parent.join(temp_name),
            committed: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for SstTemp {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl SstReader {
    /// Opens an SSTable through mmap.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let column = MmapColumn::open(path.as_ref())?;
        let bytes = column.as_bytes();
        let header = read_header(bytes)?;
        let (bounds, bloom_end) = if header.version == VERSION {
            let (bounds, metadata_start, last_record_offset) =
                decode_range_metadata(bytes, &header)?;
            (Some((bounds, last_record_offset)), metadata_start)
        } else {
            (None, bytes.len())
        };
        let index = read_index(
            bytes,
            header.entries,
            header.index_offset,
            header.bloom_offset,
        )?;
        let bloom_bytes = bytes
            .get(header.bloom_offset as usize..bloom_end)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST bloom offset out of bounds"))?;
        let bloom = BloomFilter::decode(bloom_bytes)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("invalid SST bloom filter"))?;
        if let Some((bounds, last_record_offset)) = bounds {
            validate_range_metadata_against_index(
                &bounds,
                last_record_offset,
                &index,
                header.index_offset,
            )?;
        }
        let file_len = column.file_len();
        if file_len > HEADER_LEN {
            column.drop_pages(HEADER_LEN, file_len - HEADER_LEN);
        }
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

    /// Reads at most one record: the greatest key in the requested bound.
    pub(crate) fn predecessor(
        &self,
        start: &[u8],
        upper: &[u8],
        inclusive: bool,
    ) -> Result<Option<SstEntry>> {
        let position = self.index.partition_point(|entry| {
            if inclusive {
                entry.key.as_slice() <= upper
            } else {
                entry.key.as_slice() < upper
            }
        });
        let Some(entry) = position
            .checked_sub(1)
            .and_then(|index| self.index.get(index))
        else {
            return Ok(None);
        };
        if entry.key.as_slice() < start {
            return Ok(None);
        }
        read_record(self.column.as_bytes(), entry.offset).map(Some)
    }

    pub fn range_key_states(&self, start: &[u8], end: &[u8]) -> Result<Vec<SstKeyState>> {
        self.range_key_states_until(start, Some(end))
    }

    pub fn range_key_states_until(
        &self,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Result<Vec<SstKeyState>> {
        let start_at = self
            .index
            .partition_point(|entry| entry.key.as_slice() < start);
        let mut rows = Vec::new();
        for entry in &self.index[start_at..] {
            if end.is_some_and(|end| entry.key.as_slice() >= end) {
                break;
            }
            let record = read_record_ref(self.column.as_bytes(), entry.offset)?;
            rows.push(SstKeyState {
                key: record.key.to_vec(),
                is_tombstone: crate::mvcc::is_tombstone_value(record.value),
            });
        }
        Ok(rows)
    }

    pub fn iter(&self) -> Result<Vec<SstEntry>> {
        self.index
            .iter()
            .map(|entry| read_record(self.column.as_bytes(), entry.offset))
            .collect()
    }

    pub(crate) fn iter_with_offsets(&self) -> Result<Vec<(u64, SstEntry)>> {
        self.index
            .iter()
            .map(|entry| {
                Ok((
                    entry.offset,
                    read_record(self.column.as_bytes(), entry.offset)?,
                ))
            })
            .collect()
    }

    pub fn bloom_may_contain(&self, key: &[u8]) -> bool {
        self.bloom.may_contain(key)
    }

    /// First and last key stored in this SST, or `None` for an empty file.
    pub fn key_range(&self) -> Option<(&[u8], &[u8])> {
        Some((
            self.index.first()?.key.as_slice(),
            self.index.last()?.key.as_slice(),
        ))
    }

    pub(crate) fn lookup_metadata(&self) -> Option<SstLookupMetadata> {
        let first_key = self.index.first()?.key.clone();
        let last_key = self.index.last()?.key.clone();
        Some(SstLookupMetadata {
            first_key,
            last_key,
            bloom: self.bloom.clone(),
            index: self.index.clone(),
        })
    }

    pub(crate) fn key_bounds(&self) -> SstKeyBounds {
        SstKeyBounds {
            entries: self.index.len() as u32,
            first_key: self
                .index
                .first()
                .map(|entry| entry.key.clone())
                .unwrap_or_default(),
            last_key: self
                .index
                .last()
                .map(|entry| entry.key.clone())
                .unwrap_or_default(),
        }
    }
}

struct SstRecordRef<'a> {
    key: &'a [u8],
    value: &'a [u8],
}

#[derive(Debug, Clone, Copy)]
struct Header {
    version: u32,
    entries: u32,
    index_offset: u64,
    bloom_offset: u64,
}

fn write_range_metadata(
    out: &mut Vec<u8>,
    entries: &[(Vec<u8>, Vec<u8>)],
    index: &[IndexEntry],
) -> Result<()> {
    let entry_count = u32::try_from(entries.len())
        .map_err(|_| CalyxError::disk_pressure("SST entry count exceeds u32"))?;
    let first_key = entries
        .first()
        .map(|entry| entry.0.as_slice())
        .unwrap_or_default();
    let last_key = entries
        .last()
        .map(|entry| entry.0.as_slice())
        .unwrap_or_default();
    let first_len = u32::try_from(first_key.len())
        .map_err(|_| CalyxError::disk_pressure("SST first key length exceeds u32"))?;
    let last_len = u32::try_from(last_key.len())
        .map_err(|_| CalyxError::disk_pressure("SST last key length exceeds u32"))?;
    let last_record_offset = index.last().map(|entry| entry.offset).unwrap_or(0);
    let metadata_start = out.len();
    out.extend_from_slice(&RANGE_METADATA_VERSION.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&first_len.to_le_bytes());
    out.extend_from_slice(&last_len.to_le_bytes());
    out.extend_from_slice(&last_record_offset.to_le_bytes());
    out.extend_from_slice(first_key);
    out.extend_from_slice(last_key);
    let crc = section_crc(&out[metadata_start..]);
    out.extend_from_slice(&crc.to_le_bytes());
    let metadata_len = u32::try_from(out.len() - metadata_start)
        .map_err(|_| CalyxError::disk_pressure("SST range metadata exceeds u32"))?;
    out.extend_from_slice(&metadata_len.to_le_bytes());
    out.extend_from_slice(RANGE_FOOTER_MAGIC);
    Ok(())
}

fn decode_range_metadata(bytes: &[u8], header: &Header) -> Result<(SstKeyBounds, usize, u64)> {
    let footer_start = bytes
        .len()
        .checked_sub(RANGE_FOOTER_LEN)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST v3 range footer is missing"))?;
    let footer = &bytes[footer_start..];
    if &footer[4..8] != RANGE_FOOTER_MAGIC {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range footer magic mismatch",
        ));
    }
    let metadata_len =
        u32::from_le_bytes(footer[0..4].try_into().expect("range metadata len")) as usize;
    let metadata_start = footer_start.checked_sub(metadata_len).ok_or_else(|| {
        CalyxError::aster_corrupt_shard("SST v3 range metadata length is out of bounds")
    })?;
    if metadata_start < header.bloom_offset as usize {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata overlaps the bloom section",
        ));
    }
    let metadata = &bytes[metadata_start..footer_start];
    let (bounds, last_record_offset) = parse_range_metadata(metadata, header.entries)?;
    Ok((bounds, metadata_start, last_record_offset))
}

fn parse_range_metadata(metadata: &[u8], header_entries: u32) -> Result<(SstKeyBounds, u64)> {
    if metadata.len() < RANGE_METADATA_FIXED_LEN + RANGE_METADATA_CRC_LEN {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata is truncated",
        ));
    }
    let payload_end = metadata.len() - RANGE_METADATA_CRC_LEN;
    let expected_crc = u32::from_le_bytes(
        metadata[payload_end..]
            .try_into()
            .expect("range metadata crc"),
    );
    let actual_crc = section_crc(&metadata[..payload_end]);
    if actual_crc != expected_crc {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 range metadata crc mismatch: expected {expected_crc:08x}, got {actual_crc:08x}"
        )));
    }
    let version = u32::from_le_bytes(metadata[0..4].try_into().expect("range version"));
    if version != RANGE_METADATA_VERSION {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "unsupported SST range metadata version {version}"
        )));
    }
    let entries = u32::from_le_bytes(metadata[4..8].try_into().expect("range entries"));
    if entries != header_entries {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST range metadata entry count {entries} does not match header {header_entries}"
        )));
    }
    let first_len = u32::from_le_bytes(metadata[8..12].try_into().expect("first key len")) as usize;
    let last_len = u32::from_le_bytes(metadata[12..16].try_into().expect("last key len")) as usize;
    let last_record_offset =
        u64::from_le_bytes(metadata[16..24].try_into().expect("last record offset"));
    let keys_end = RANGE_METADATA_FIXED_LEN
        .checked_add(first_len)
        .and_then(|end| end.checked_add(last_len))
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST range key lengths overflow"))?;
    if keys_end != payload_end {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata length mismatch",
        ));
    }
    let first_key =
        metadata[RANGE_METADATA_FIXED_LEN..RANGE_METADATA_FIXED_LEN + first_len].to_vec();
    let last_key = metadata[RANGE_METADATA_FIXED_LEN + first_len..keys_end].to_vec();
    if entries == 0 {
        if first_len != 0 || last_len != 0 || last_record_offset != 0 {
            return Err(CalyxError::aster_corrupt_shard(
                "empty SST v3 range metadata contains key bounds",
            ));
        }
    } else if first_key.is_empty() || last_key.is_empty() || first_key > last_key {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata contains invalid key bounds",
        ));
    }
    Ok((
        SstKeyBounds {
            entries,
            first_key,
            last_key,
        },
        last_record_offset,
    ))
}

fn validate_range_metadata_against_index(
    bounds: &SstKeyBounds,
    last_record_offset: u64,
    index: &[IndexEntry],
    index_offset: u64,
) -> Result<()> {
    if index.is_empty() {
        if bounds.entries == 0 && index_offset == HEADER_LEN as u64 {
            return Ok(());
        }
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata declares rows but the validated index is empty",
        ));
    }
    if index.len() != bounds.entries as usize
        || index.first().map(|entry| entry.key.as_slice()) != Some(bounds.first_key.as_slice())
        || index.last().map(|entry| entry.key.as_slice()) != Some(bounds.last_key.as_slice())
        || index.last().map(|entry| entry.offset) != Some(last_record_offset)
    {
        return Err(CalyxError::aster_corrupt_shard(
            "SST v3 range metadata disagrees with the validated index",
        ));
    }
    Ok(())
}

/// Reads the authoritative v3 key bounds without mmap or a whole-file body
/// checksum. Version 1/2 files return `None` and remain readable until a
/// verified compaction migrates them to v3.
pub(crate) fn read_sst_key_bounds(path: impl AsRef<Path>) -> Result<Option<SstKeyBounds>> {
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|error| {
        CalyxError::disk_pressure(format!("open SST range footer {}: {error}", path.display()))
    })?;
    let file_len = file
        .metadata()
        .map_err(|error| {
            CalyxError::disk_pressure(format!("stat SST range footer {}: {error}", path.display()))
        })?
        .len();
    let mut header_bytes = [0_u8; HEADER_LEN];
    read_file_exact_at(&mut file, &mut header_bytes, 0, path, "header")?;
    let header = read_header_fields(&header_bytes, file_len)?;
    if header.version != VERSION {
        return Ok(None);
    }
    if file_len < (HEADER_LEN + RANGE_FOOTER_LEN) as u64 {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 range footer is missing in {}",
            path.display()
        )));
    }
    let footer_offset = file_len - RANGE_FOOTER_LEN as u64;
    let mut footer = [0_u8; RANGE_FOOTER_LEN];
    read_file_exact_at(&mut file, &mut footer, footer_offset, path, "range footer")?;
    if &footer[4..8] != RANGE_FOOTER_MAGIC {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 range footer magic mismatch in {}",
            path.display()
        )));
    }
    let metadata_len =
        u32::from_le_bytes(footer[0..4].try_into().expect("range metadata len")) as u64;
    let metadata_offset = footer_offset.checked_sub(metadata_len).ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!(
            "SST v3 range metadata length is out of bounds in {}",
            path.display()
        ))
    })?;
    if metadata_offset < header.bloom_offset {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 range metadata overlaps the bloom section in {}",
            path.display()
        )));
    }
    let metadata_len = usize::try_from(metadata_len)
        .map_err(|_| CalyxError::aster_corrupt_shard("SST range metadata exceeds usize"))?;
    let mut metadata = vec![0_u8; metadata_len];
    read_file_exact_at(
        &mut file,
        &mut metadata,
        metadata_offset,
        path,
        "range metadata",
    )?;
    let (bounds, last_record_offset) = parse_range_metadata(&metadata, header.entries)?;
    validate_boundary_keys(&mut file, path, &header, &bounds, last_record_offset)?;
    Ok(Some(bounds))
}

fn validate_boundary_keys(
    file: &mut fs::File,
    path: &Path,
    header: &Header,
    bounds: &SstKeyBounds,
    last_record_offset: u64,
) -> Result<()> {
    if bounds.entries == 0 {
        if header.index_offset != HEADER_LEN as u64 {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "empty SST data section does not end at the header in {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let (first_key, first_end) =
        read_boundary_key(file, path, HEADER_LEN as u64, header.index_offset)?;
    let (last_key, last_end) =
        read_boundary_key(file, path, last_record_offset, header.index_offset)?;
    if first_key != bounds.first_key
        || last_key != bounds.last_key
        || last_end != header.index_offset
    {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 range metadata disagrees with boundary records in {}",
            path.display()
        )));
    }
    if bounds.entries > 1 && first_end > last_record_offset {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST v3 boundary record offsets overlap in {}",
            path.display()
        )));
    }
    Ok(())
}

fn read_boundary_key(
    file: &mut fs::File,
    path: &Path,
    offset: u64,
    data_end: u64,
) -> Result<(Vec<u8>, u64)> {
    if offset < HEADER_LEN as u64 || offset >= data_end {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST boundary record offset {offset} is outside data section in {}",
            path.display()
        )));
    }
    let mut record_header = [0_u8; RECORD_HEADER_LEN];
    read_file_exact_at(
        file,
        &mut record_header,
        offset,
        path,
        "boundary record header",
    )?;
    let key_len = u32::from_le_bytes(record_header[0..4].try_into().expect("key len")) as u64;
    let value_len = u32::from_le_bytes(record_header[4..8].try_into().expect("value len")) as u64;
    let key_offset = offset
        .checked_add(RECORD_HEADER_LEN as u64)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST boundary key offset overflow"))?;
    let record_end = key_offset
        .checked_add(key_len)
        .and_then(|end| end.checked_add(value_len))
        .ok_or_else(|| CalyxError::aster_corrupt_shard("SST boundary record length overflow"))?;
    if record_end > data_end {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "SST boundary record exceeds data section in {}",
            path.display()
        )));
    }
    let key_len = usize::try_from(key_len)
        .map_err(|_| CalyxError::aster_corrupt_shard("SST boundary key exceeds usize"))?;
    let mut key = vec![0_u8; key_len];
    read_file_exact_at(file, &mut key, key_offset, path, "boundary key")?;
    Ok((key, record_end))
}

fn read_file_exact_at(
    file: &mut fs::File,
    out: &mut [u8],
    offset: u64,
    path: &Path,
    section: &str,
) -> Result<()> {
    file.seek(SeekFrom::Start(offset)).map_err(|error| {
        CalyxError::disk_pressure(format!("seek SST {section} {}: {error}", path.display()))
    })?;
    file.read_exact(out).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!(
            "read SST {section} at {}:{offset} failed: {error}",
            path.display()
        ))
    })
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
    let record = read_record_ref(bytes, offset)?;
    Ok(SstEntry {
        key: record.key.to_vec(),
        value: record.value.to_vec(),
    })
}

fn read_record_ref(bytes: &[u8], offset: u64) -> Result<SstRecordRef<'_>> {
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
    Ok(SstRecordRef { key, value })
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
    if index.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(CalyxError::aster_corrupt_shard(
            "SST index keys must be strictly sorted",
        ));
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
    let parsed = read_header_fields(header, bytes.len() as u64)?;
    if parsed.version >= CHECKSUMMED_VERSION {
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
    Ok(parsed)
}

fn read_header_fields(header: &[u8], len: u64) -> Result<Header> {
    if header.len() != HEADER_LEN {
        return Err(CalyxError::aster_corrupt_shard("SST header missing"));
    }
    if &header[0..4] != MAGIC {
        return Err(CalyxError::aster_corrupt_shard("SST magic mismatch"));
    }
    let version = u32::from_le_bytes(header[4..8].try_into().expect("version"));
    if version != VERSION && version != CHECKSUMMED_VERSION && version != LEGACY_VERSION {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "unsupported SST version {version}"
        )));
    }
    let entries = u32::from_le_bytes(header[8..12].try_into().expect("entries"));
    let index_offset = u64::from_le_bytes(header[12..20].try_into().expect("index offset"));
    let bloom_offset = u64::from_le_bytes(header[20..28].try_into().expect("bloom offset"));
    if index_offset < HEADER_LEN as u64
        || index_offset > len
        || bloom_offset < index_offset
        || bloom_offset > len
    {
        return Err(CalyxError::aster_corrupt_shard(
            "SST header offsets out of bounds",
        ));
    }
    Ok(Header {
        version,
        entries,
        index_offset,
        bloom_offset,
    })
}

#[cfg(test)]
mod tests;
