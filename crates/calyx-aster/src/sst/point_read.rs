use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

use super::{
    CHECKSUMMED_VERSION, HEADER_LEN, IndexEntry, LEGACY_VERSION, MAGIC, RECORD_HEADER_LEN,
    SstEntry, SstReader, VERSION, record_crc,
};

/// Fully validates an immutable SST once, retains only its small sorted index,
/// and performs subsequent row reads with exact file I/O. Unlike `SstReader`,
/// this type does not retain an mmap of the SST value section for the lifetime
/// of a range/compaction cursor.
#[derive(Debug)]
pub(crate) struct SstStreamingReader {
    path: PathBuf,
    index: Vec<IndexEntry>,
}

impl SstStreamingReader {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        let reader = SstReader::open(path)?;
        let SstReader {
            column,
            index,
            bloom: _,
        } = reader;
        let path = column.path().to_path_buf();
        // `SstReader::open` has already verified the whole-file body CRC and
        // decoded the index. Drop the mmap before this reader can escape so
        // scanning many SSTs cannot retain every visited value page in RSS.
        drop(column);
        Ok(Self { path, index })
    }

    pub(crate) fn lower_bound(&self, key: &[u8], exclusive: bool) -> usize {
        if exclusive {
            self.index
                .partition_point(|entry| entry.key.as_slice() <= key)
        } else {
            self.index
                .partition_point(|entry| entry.key.as_slice() < key)
        }
    }

    pub(crate) fn key_at(&self, position: usize) -> Option<&[u8]> {
        self.index.get(position).map(|entry| entry.key.as_slice())
    }

    pub(crate) fn entry_at(&self, position: usize) -> Result<SstEntry> {
        let indexed = self.index.get(position).ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "SST streaming row position {position} is outside index length {} in {}",
                self.index.len(),
                self.path.display()
            ))
        })?;
        let mut reader = SstPointReader::open(&self.path)?;
        let value = reader.read_value(indexed.offset, &indexed.key)?;
        Ok(SstEntry {
            key: indexed.key.clone(),
            value,
        })
    }
}

pub(crate) struct SstPointReader {
    file: File,
    path: PathBuf,
    data_end: u64,
}

impl SstPointReader {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|error| storage_error("open SST for indexed row read", path, error))?;
        #[cfg(target_os = "linux")]
        {
            use nix::fcntl::{PosixFadviseAdvice, posix_fadvise};

            posix_fadvise(&file, 0, 0, PosixFadviseAdvice::POSIX_FADV_RANDOM).map_err(|error| {
                storage_error(
                    "declare random SST point-read access",
                    path,
                    io::Error::from(error),
                )
            })?;
            posix_fadvise(&file, 0, 0, PosixFadviseAdvice::POSIX_FADV_NOREUSE).map_err(
                |error| {
                    storage_error(
                        "declare one-pass SST point-read access",
                        path,
                        io::Error::from(error),
                    )
                },
            )?;
        }
        let file_len = file
            .metadata()
            .map_err(|error| storage_error("stat SST for indexed row read", path, error))?
            .len();
        let mut header = [0_u8; HEADER_LEN];
        read_exact_indexed(&mut file, &mut header, path, 0)?;
        if &header[0..4] != MAGIC {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST {} magic mismatch during indexed row read",
                path.display()
            )));
        }
        let version = u32::from_le_bytes(header[4..8].try_into().expect("version"));
        if version != VERSION && version != CHECKSUMMED_VERSION && version != LEGACY_VERSION {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "unsupported SST version {version} in {}",
                path.display()
            )));
        }
        let index_offset = u64::from_le_bytes(header[12..20].try_into().expect("index offset"));
        let bloom_offset = u64::from_le_bytes(header[20..28].try_into().expect("bloom offset"));
        if index_offset < HEADER_LEN as u64
            || index_offset > file_len
            || bloom_offset < index_offset
            || bloom_offset > file_len
        {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST {} header offsets are out of bounds for file length {file_len}",
                path.display()
            )));
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            data_end: index_offset,
        })
    }

    /// Reads and CRC-validates one exact SST record. The caller separately
    /// authenticates the value against the SHA-256 stored in the page index.
    pub(crate) fn read_value(
        &mut self,
        record_offset: u64,
        expected_key: &[u8],
    ) -> Result<Vec<u8>> {
        if record_offset < HEADER_LEN as u64 || record_offset >= self.data_end {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST indexed record offset {record_offset} is outside data section {}..{} in {}",
                HEADER_LEN,
                self.data_end,
                self.path.display()
            )));
        }
        self.file
            .seek(SeekFrom::Start(record_offset))
            .map_err(|error| storage_error("seek SST indexed row", &self.path, error))?;
        let mut header = [0_u8; RECORD_HEADER_LEN];
        read_exact_indexed(&mut self.file, &mut header, &self.path, record_offset)?;
        let key_len = u32::from_le_bytes(header[0..4].try_into().expect("key len")) as usize;
        let value_len = u32::from_le_bytes(header[4..8].try_into().expect("value len")) as usize;
        let expected_crc = u32::from_le_bytes(header[8..12].try_into().expect("record crc"));
        let key_start = record_offset
            .checked_add(RECORD_HEADER_LEN as u64)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST key offset overflow"))?;
        let value_start = key_start
            .checked_add(key_len as u64)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST value offset overflow"))?;
        let value_end = value_start
            .checked_add(value_len as u64)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("SST value length overflow"))?;
        if value_end > self.data_end {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST indexed record {record_offset} ends at {value_end}, beyond data section end {} in {}",
                self.data_end,
                self.path.display()
            )));
        }
        let mut key = vec![0_u8; key_len];
        read_exact_indexed(&mut self.file, &mut key, &self.path, key_start)?;
        if key != expected_key {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST indexed record at {}:{record_offset} has key {} instead of {}",
                self.path.display(),
                hex_bytes(&key),
                hex_bytes(expected_key)
            )));
        }
        let mut value = vec![0_u8; value_len];
        read_exact_indexed(&mut self.file, &mut value, &self.path, value_start)?;
        let actual_crc = record_crc(&key, &value);
        if actual_crc != expected_crc {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "SST indexed record CRC mismatch at {}:{record_offset}: expected {expected_crc:08x}, got {actual_crc:08x}",
                self.path.display()
            )));
        }
        Ok(value)
    }
}

fn read_exact_indexed(file: &mut File, out: &mut [u8], path: &Path, offset: u64) -> Result<()> {
    file.read_exact(out).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            CalyxError::aster_corrupt_shard(format!(
                "SST indexed row is truncated at {}:{offset} while reading {} bytes",
                path.display(),
                out.len()
            ))
        } else {
            storage_error("read SST indexed row", path, error)
        }
    })
}

fn storage_error(context: &str, path: &Path, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context} {}: {error}", path.display()))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod streaming_tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::sst::write_sst;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fully_validated_streaming_reader_retains_only_index_and_reads_exact_large_rows() {
        let dir = test_dir("large-row");
        let path = dir.join("large.sst");
        let large = vec![0x5a; 2 * 1024 * 1024];
        write_sst(
            &path,
            [
                (b"a".as_slice(), b"small".as_slice()),
                (b"b".as_slice(), large.as_slice()),
            ],
        )
        .unwrap();

        let reader = SstStreamingReader::open(&path).unwrap();
        assert_eq!(reader.index.len(), 2);
        assert_eq!(reader.key_at(0), Some(b"a".as_slice()));
        assert_eq!(reader.key_at(1), Some(b"b".as_slice()));
        assert_eq!(reader.entry_at(0).unwrap().value, b"small");
        assert_eq!(reader.entry_at(1).unwrap().value, large);
        println!(
            "SST_STREAMING_READER_FSV source_bytes={} retained_index_entries={} exact_large_value_bytes={}",
            fs::metadata(&path).unwrap().len(),
            reader.index.len(),
            large.len()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn streaming_reader_fails_before_escape_when_unselected_body_is_corrupt() {
        let dir = test_dir("corrupt-body");
        let path = dir.join("corrupt.sst");
        let large = vec![0x33; 2 * 1024 * 1024];
        write_sst(
            &path,
            [
                (b"a".as_slice(), b"selected".as_slice()),
                (b"z".as_slice(), large.as_slice()),
            ],
        )
        .unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let corrupt_at = bytes.len() / 2;
        bytes[corrupt_at] ^= 0xff;
        fs::write(&path, bytes).unwrap();

        let error = SstStreamingReader::open(&path).unwrap_err();
        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
        assert!(error.message.contains("body crc mismatch"));
        println!(
            "SST_STREAMING_CORRUPTION_FSV before=open after=error code={} detail={}",
            error.code, error.message
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-sst-streaming-{name}-{}-{id}",
            std::process::id()
        ));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
