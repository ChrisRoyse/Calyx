//! WAL record framing.
//!
//! Two on-disk physical record formats coexist, distinguished by their magic:
//!
//! * **`CXW1` full record** — a single self-contained record whose payload fits
//!   in one physical record (`payload.len() <= MAX_RECORD_BYTES`). This is the
//!   common case and is byte-identical to the original single-format WAL, so
//!   every WAL written before fragmentation existed still replays unchanged.
//! * **`CXW2` fragment record** — one physical fragment of a *logical* record
//!   whose payload exceeds `MAX_RECORD_BYTES`. A logical record is split into a
//!   chain of `frag_count` fragments (RocksDB/LevelDB-style framing) that all
//!   share one logical `seq`, are written contiguously within a single segment
//!   file, and are CRC-protected per fragment exactly like a full record.
//!
//! Readers reassemble a fragment chain back into one logical payload, so every
//! payload-level consumer (replay, recovery, `decode_write_batch`) is oblivious
//! to fragmentation. Only physical-offset consumers (the Base page index point
//! reader) care, and they fail closed on fragmented records.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// Magic for a `CXW1` full record (payload fits in one physical record).
pub(super) const MAGIC: u32 = u32::from_le_bytes(*b"CXW1");
/// Magic for a `CXW2` fragment record (one fragment of a large logical record).
pub(super) const MAGIC_FRAGMENT: u32 = u32::from_le_bytes(*b"CXW2");
/// Fixed header length of a `CXW1` full record.
pub(super) const HEADER_LEN: usize = 20;
/// Fixed header length of a `CXW2` fragment record.
pub(super) const FRAGMENT_HEADER_LEN: usize = 36;
/// Hard cap on one physical record's payload, keeping read buffers bounded. A
/// logical payload larger than this is split into fragments of at most this many
/// payload bytes each.
pub(super) const MAX_RECORD_BYTES: u32 = 64 * 1024 * 1024;
/// Sanity ceiling on one *logical* WAL payload (the encoded bytes of a single
/// commit batch). A commit larger than this fails closed with
/// `CALYX_WAL_PAYLOAD_TOO_LARGE` rather than fragmenting without bound; it is a
/// per-commit capacity limit (split the batch), not disk pressure.
pub(super) const MAX_LOGICAL_PAYLOAD_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DecodeStatus {
    Complete(DecodedRecord),
    Eof,
    Torn { offset: u64, message: String },
}

/// A fully decoded, reassembled logical WAL record.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DecodedRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
    pub start_offset: u64,
    pub end_offset: u64,
    /// True when the logical payload was reassembled from a `CXW2` fragment
    /// chain. Physical-offset consumers must refuse fragmented records.
    pub fragmented: bool,
}

/// Physical span of one logical record read from headers alone (no payload
/// materialization, no CRC), used by inventory and checkpoint fast-skip.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SpanStatus {
    Complete(LogicalSpan),
    Eof,
    Torn { offset: u64, message: String },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct LogicalSpan {
    pub seq: u64,
    pub start_offset: u64,
    pub end_offset: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum HeaderStatus {
    Complete(RecordHeader),
    Eof,
    Torn { offset: u64, message: String },
}

/// Header of a single `CXW1` full record (physical reader for the Base page
/// index point reader; fragment records are rejected here).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RecordHeader {
    pub seq: u64,
    pub len: u32,
    pub expected_crc: u32,
    pub start_offset: u64,
    pub end_offset: u64,
}

/// Encodes a single `CXW1` full record. Callers must keep `payload.len() <=
/// MAX_RECORD_BYTES`; larger logical payloads are fragmented via
/// [`fragment_header`] by the writer.
pub(super) fn encode(seq: u64, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_RECORD_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("WAL payload exceeds max record size {MAX_RECORD_BYTES}"),
        ));
    }

    let len = payload.len() as u32;
    let crc = payload_crc(seq, len, payload);
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&MAGIC.to_le_bytes());
    bytes.extend_from_slice(&seq.to_le_bytes());
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Number of fragments a logical payload of `len` bytes splits into.
pub(super) fn fragment_count(len: usize) -> usize {
    len.div_ceil(MAX_RECORD_BYTES as usize).max(1)
}

/// Builds the 36-byte `CXW2` fragment header for one chunk. The chunk itself is
/// written immediately after by the caller (avoiding a second copy of the
/// payload). The CRC covers the logical framing fields and the chunk bytes.
pub(super) fn fragment_header(
    seq: u64,
    frag_index: u32,
    frag_count: u32,
    logical_len: u64,
    chunk: &[u8],
) -> [u8; FRAGMENT_HEADER_LEN] {
    let frag_len = chunk.len() as u32;
    let crc = fragment_crc(seq, frag_index, frag_count, logical_len, frag_len, chunk);
    let mut header = [0u8; FRAGMENT_HEADER_LEN];
    header[0..4].copy_from_slice(&MAGIC_FRAGMENT.to_le_bytes());
    header[4..12].copy_from_slice(&seq.to_le_bytes());
    header[12..16].copy_from_slice(&frag_index.to_le_bytes());
    header[16..20].copy_from_slice(&frag_count.to_le_bytes());
    header[20..28].copy_from_slice(&logical_len.to_le_bytes());
    header[28..32].copy_from_slice(&frag_len.to_le_bytes());
    header[32..36].copy_from_slice(&crc.to_le_bytes());
    header
}

/// Decodes and reassembles one logical record starting at `offset`.
///
/// A `CXW1` record decodes directly; a `CXW2` fragment reassembles its whole
/// chain, validating per-fragment CRCs and framing consistency. Any truncation
/// or corruption in a chain reports `Torn` at the chain's start offset so the
/// whole logical record is discarded as a unit (see the replay torn-tail rule).
pub(super) fn decode_at(file: &mut File, offset: u64) -> io::Result<DecodeStatus> {
    match peek_magic(file, offset)? {
        MagicPeek::Eof => Ok(DecodeStatus::Eof),
        MagicPeek::Torn { message } => Ok(DecodeStatus::Torn { offset, message }),
        MagicPeek::Magic(MAGIC) => decode_full_at(file, offset),
        MagicPeek::Magic(MAGIC_FRAGMENT) => decode_fragment_chain_at(file, offset),
        MagicPeek::Magic(other) => Ok(DecodeStatus::Torn {
            offset,
            message: format!("bad WAL magic 0x{other:08x}"),
        }),
    }
}

/// Reusable-buffer counterpart of [`DecodeStatus`]: the reassembled payload
/// lives in a caller-owned buffer instead of a freshly allocated `Vec`, so a
/// streaming scan reuses one buffer across every record.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DecodeInto {
    Complete {
        seq: u64,
        start_offset: u64,
        end_offset: u64,
        fragmented: bool,
    },
    Eof,
    Torn {
        offset: u64,
        message: String,
    },
}

/// Decodes and reassembles one logical record starting at `offset` INTO `buf`
/// (cleared first), reusing the caller's allocation instead of returning an
/// owned `Vec` per record.
///
/// This is the byte-for-byte decode of [`decode_at`] — same CRC checks, same
/// torn/framing semantics — but a scan over a huge WAL keeps only one buffer
/// resident (grown to the largest record) rather than churning one owned Vec
/// per record, which bloated the glibc arena to ~O(total WAL bytes) and
/// OOM-killed `wal-prune`/reader opens on a 27 GB WAL (issue #1992).
pub(super) fn decode_into(
    file: &mut File,
    offset: u64,
    buf: &mut Vec<u8>,
) -> io::Result<DecodeInto> {
    buf.clear();
    match peek_magic(file, offset)? {
        MagicPeek::Eof => Ok(DecodeInto::Eof),
        MagicPeek::Torn { message } => Ok(DecodeInto::Torn { offset, message }),
        MagicPeek::Magic(MAGIC) => decode_full_into(file, offset, buf),
        MagicPeek::Magic(MAGIC_FRAGMENT) => decode_fragment_chain_into(file, offset, buf),
        MagicPeek::Magic(other) => Ok(DecodeInto::Torn {
            offset,
            message: format!("bad WAL magic 0x{other:08x}"),
        }),
    }
}

fn decode_full_into(file: &mut File, offset: u64, buf: &mut Vec<u8>) -> io::Result<DecodeInto> {
    let header = match read_full_header_at(file, offset)? {
        HeaderStatus::Complete(header) => header,
        HeaderStatus::Eof => return Ok(DecodeInto::Eof),
        HeaderStatus::Torn { offset, message } => {
            return Ok(DecodeInto::Torn { offset, message });
        }
    };
    buf.resize(header.len as usize, 0);
    file.seek(SeekFrom::Start(offset + HEADER_LEN as u64))?;
    if let Err(error) = file.read_exact(buf) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(DecodeInto::Torn {
                offset,
                message: format!(
                    "partial WAL payload for seq {}: wanted {} bytes",
                    header.seq, header.len
                ),
            });
        }
        return Err(error);
    }
    let actual_crc = payload_crc(header.seq, header.len, buf);
    if actual_crc != header.expected_crc {
        return Ok(DecodeInto::Torn {
            offset,
            message: format!(
                "crc mismatch for seq {}: expected {:08x}, got {actual_crc:08x}",
                header.seq, header.expected_crc
            ),
        });
    }
    Ok(DecodeInto::Complete {
        seq: header.seq,
        start_offset: offset,
        end_offset: header.end_offset,
        fragmented: false,
    })
}

fn decode_fragment_chain_into(
    file: &mut File,
    chain_start: u64,
    buf: &mut Vec<u8>,
) -> io::Result<DecodeInto> {
    let mut offset = chain_start;
    let mut expected_index: u32 = 0;
    let mut chain_seq: Option<u64> = None;
    let mut chain_count: Option<u32> = None;
    let mut chain_logical_len: Option<u64> = None;

    loop {
        let header = match read_fragment_header_at(file, offset)? {
            FragmentHeaderStatus::Complete(header) => header,
            FragmentHeaderStatus::Eof => {
                return Ok(DecodeInto::Torn {
                    offset: chain_start,
                    message: format!("fragment chain truncated before fragment {expected_index}"),
                });
            }
            FragmentHeaderStatus::Torn { message } => {
                return Ok(DecodeInto::Torn {
                    offset: chain_start,
                    message,
                });
            }
        };
        if let Some(message) = fragment_framing_error(
            &header,
            expected_index,
            &mut chain_seq,
            &mut chain_count,
            &mut chain_logical_len,
        ) {
            return Ok(DecodeInto::Torn {
                offset: chain_start,
                message,
            });
        }

        // Read the fragment payload directly onto the tail of `buf` (no
        // per-fragment temporary), so a fragment chain also reassembles with
        // zero incremental allocation beyond the one reused buffer.
        let payload_start = offset + FRAGMENT_HEADER_LEN as u64;
        let chunk_start = buf.len();
        buf.resize(chunk_start + header.frag_len as usize, 0);
        file.seek(SeekFrom::Start(payload_start))?;
        if let Err(error) = file.read_exact(&mut buf[chunk_start..]) {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(DecodeInto::Torn {
                    offset: chain_start,
                    message: format!(
                        "partial WAL fragment payload for seq {} fragment {}: wanted {} bytes",
                        header.seq, header.frag_index, header.frag_len
                    ),
                });
            }
            return Err(error);
        }
        let actual_crc = fragment_crc(
            header.seq,
            header.frag_index,
            header.frag_count,
            header.logical_len,
            header.frag_len,
            &buf[chunk_start..],
        );
        if actual_crc != header.expected_crc {
            return Ok(DecodeInto::Torn {
                offset: chain_start,
                message: format!(
                    "crc mismatch for seq {} fragment {}: expected {:08x}, got {actual_crc:08x}",
                    header.seq, header.frag_index, header.expected_crc
                ),
            });
        }
        offset = payload_start + header.frag_len as u64;
        expected_index += 1;

        if expected_index == header.frag_count {
            let logical_len = header.logical_len;
            if buf.len() as u64 != logical_len {
                return Ok(DecodeInto::Torn {
                    offset: chain_start,
                    message: format!(
                        "fragment chain for seq {} reassembled {} bytes, header declared {logical_len}",
                        header.seq,
                        buf.len()
                    ),
                });
            }
            return Ok(DecodeInto::Complete {
                seq: header.seq,
                start_offset: chain_start,
                end_offset: offset,
                fragmented: true,
            });
        }
    }
}

fn decode_full_at(file: &mut File, offset: u64) -> io::Result<DecodeStatus> {
    let header = match read_full_header_at(file, offset)? {
        HeaderStatus::Complete(header) => header,
        HeaderStatus::Eof => return Ok(DecodeStatus::Eof),
        HeaderStatus::Torn { offset, message } => {
            return Ok(DecodeStatus::Torn { offset, message });
        }
    };
    let mut payload = vec![0u8; header.len as usize];
    file.seek(SeekFrom::Start(offset + HEADER_LEN as u64))?;
    if let Err(error) = file.read_exact(&mut payload) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(DecodeStatus::Torn {
                offset,
                message: format!(
                    "partial WAL payload for seq {}: wanted {} bytes",
                    header.seq, header.len
                ),
            });
        }
        return Err(error);
    }

    let actual_crc = payload_crc(header.seq, header.len, &payload);
    if actual_crc != header.expected_crc {
        return Ok(DecodeStatus::Torn {
            offset,
            message: format!(
                "crc mismatch for seq {}: expected {:08x}, got {actual_crc:08x}",
                header.seq, header.expected_crc
            ),
        });
    }

    Ok(DecodeStatus::Complete(DecodedRecord {
        seq: header.seq,
        payload,
        start_offset: offset,
        end_offset: header.end_offset,
        fragmented: false,
    }))
}

fn decode_fragment_chain_at(file: &mut File, chain_start: u64) -> io::Result<DecodeStatus> {
    let mut offset = chain_start;
    let mut payload: Vec<u8> = Vec::new();
    let mut expected_index: u32 = 0;
    let mut chain_seq: Option<u64> = None;
    let mut chain_count: Option<u32> = None;
    let mut chain_logical_len: Option<u64> = None;

    loop {
        let header = match read_fragment_header_at(file, offset)? {
            FragmentHeaderStatus::Complete(header) => header,
            FragmentHeaderStatus::Eof => {
                return Ok(torn_chain(
                    chain_start,
                    format!("fragment chain truncated before fragment {expected_index}"),
                ));
            }
            FragmentHeaderStatus::Torn { message } => {
                return Ok(torn_chain(chain_start, message));
            }
        };
        if let Some(message) = fragment_framing_error(
            &header,
            expected_index,
            &mut chain_seq,
            &mut chain_count,
            &mut chain_logical_len,
        ) {
            return Ok(torn_chain(chain_start, message));
        }

        let payload_start = offset + FRAGMENT_HEADER_LEN as u64;
        let mut chunk = vec![0u8; header.frag_len as usize];
        file.seek(SeekFrom::Start(payload_start))?;
        if let Err(error) = file.read_exact(&mut chunk) {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(torn_chain(
                    chain_start,
                    format!(
                        "partial WAL fragment payload for seq {} fragment {}: wanted {} bytes",
                        header.seq, header.frag_index, header.frag_len
                    ),
                ));
            }
            return Err(error);
        }
        let actual_crc = fragment_crc(
            header.seq,
            header.frag_index,
            header.frag_count,
            header.logical_len,
            header.frag_len,
            &chunk,
        );
        if actual_crc != header.expected_crc {
            return Ok(torn_chain(
                chain_start,
                format!(
                    "crc mismatch for seq {} fragment {}: expected {:08x}, got {actual_crc:08x}",
                    header.seq, header.frag_index, header.expected_crc
                ),
            ));
        }
        payload.extend_from_slice(&chunk);
        offset = payload_start + header.frag_len as u64;
        expected_index += 1;

        if expected_index == header.frag_count {
            let logical_len = header.logical_len;
            if payload.len() as u64 != logical_len {
                return Ok(torn_chain(
                    chain_start,
                    format!(
                        "fragment chain for seq {} reassembled {} bytes, header declared {logical_len}",
                        header.seq,
                        payload.len()
                    ),
                ));
            }
            return Ok(DecodeStatus::Complete(DecodedRecord {
                seq: header.seq,
                payload,
                start_offset: chain_start,
                end_offset: offset,
                fragmented: true,
            }));
        }
    }
}

/// Reads the physical span of one logical record from headers alone.
///
/// The common `CXW1` case reads only the 20-byte header (no payload transfer).
/// A `CXW2` chain walks each fragment header, validating truncation against the
/// segment length without materializing any payload. CRCs are intentionally not
/// checked here — this mirrors the header-only fast-skip path and keeps buffers
/// bounded regardless of logical payload size.
pub(super) fn read_logical_span_at(file: &mut File, offset: u64) -> io::Result<SpanStatus> {
    let file_len = file.seek(SeekFrom::End(0))?;
    match peek_magic(file, offset)? {
        MagicPeek::Eof => Ok(SpanStatus::Eof),
        MagicPeek::Torn { message } => Ok(SpanStatus::Torn { offset, message }),
        MagicPeek::Magic(MAGIC) => match read_full_header_at(file, offset)? {
            HeaderStatus::Complete(header) => {
                if header.end_offset > file_len {
                    return Ok(SpanStatus::Torn {
                        offset,
                        message: format!(
                            "partial WAL payload for seq {}: record ends at {} but segment is {file_len} bytes",
                            header.seq, header.end_offset
                        ),
                    });
                }
                Ok(SpanStatus::Complete(LogicalSpan {
                    seq: header.seq,
                    start_offset: offset,
                    end_offset: header.end_offset,
                }))
            }
            HeaderStatus::Eof => Ok(SpanStatus::Eof),
            HeaderStatus::Torn { offset, message } => Ok(SpanStatus::Torn { offset, message }),
        },
        MagicPeek::Magic(MAGIC_FRAGMENT) => span_fragment_chain_at(file, offset, file_len),
        MagicPeek::Magic(other) => Ok(SpanStatus::Torn {
            offset,
            message: format!("bad WAL magic 0x{other:08x}"),
        }),
    }
}

fn span_fragment_chain_at(
    file: &mut File,
    chain_start: u64,
    file_len: u64,
) -> io::Result<SpanStatus> {
    let mut offset = chain_start;
    let mut expected_index: u32 = 0;
    let mut chain_seq: Option<u64> = None;
    let mut chain_count: Option<u32> = None;
    let mut chain_logical_len: Option<u64> = None;

    loop {
        let header = match read_fragment_header_at(file, offset)? {
            FragmentHeaderStatus::Complete(header) => header,
            FragmentHeaderStatus::Eof => {
                return Ok(SpanStatus::Torn {
                    offset: chain_start,
                    message: format!("fragment chain truncated before fragment {expected_index}"),
                });
            }
            FragmentHeaderStatus::Torn { message } => {
                return Ok(SpanStatus::Torn {
                    offset: chain_start,
                    message,
                });
            }
        };
        if let Some(message) = fragment_framing_error(
            &header,
            expected_index,
            &mut chain_seq,
            &mut chain_count,
            &mut chain_logical_len,
        ) {
            return Ok(SpanStatus::Torn {
                offset: chain_start,
                message,
            });
        }
        let frag_end = offset + FRAGMENT_HEADER_LEN as u64 + header.frag_len as u64;
        if frag_end > file_len {
            return Ok(SpanStatus::Torn {
                offset: chain_start,
                message: format!(
                    "partial WAL fragment payload for seq {} fragment {}: ends at {frag_end} but segment is {file_len} bytes",
                    header.seq, header.frag_index
                ),
            });
        }
        offset = frag_end;
        expected_index += 1;
        if expected_index == header.frag_count {
            return Ok(SpanStatus::Complete(LogicalSpan {
                seq: header.seq,
                start_offset: chain_start,
                end_offset: offset,
            }));
        }
    }
}

/// Reads the header of a single `CXW1` full record for the Base page index
/// physical point reader. Fragment (`CXW2`) records are rejected as torn: the
/// page index never records physical offsets into a fragmented record.
pub(super) fn read_header_at(file: &mut File, offset: u64) -> io::Result<HeaderStatus> {
    match peek_magic(file, offset)? {
        MagicPeek::Eof => Ok(HeaderStatus::Eof),
        MagicPeek::Torn { message } => Ok(HeaderStatus::Torn { offset, message }),
        MagicPeek::Magic(MAGIC) => read_full_header_at(file, offset),
        MagicPeek::Magic(MAGIC_FRAGMENT) => Ok(HeaderStatus::Torn {
            offset,
            message:
                "WAL record is a CXW2 fragment; physical row reads require a single-record commit"
                    .to_string(),
        }),
        MagicPeek::Magic(other) => Ok(HeaderStatus::Torn {
            offset,
            message: format!("bad WAL magic 0x{other:08x}"),
        }),
    }
}

fn read_full_header_at(file: &mut File, offset: u64) -> io::Result<HeaderStatus> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; HEADER_LEN];
    let read = fill(file, &mut header)?;
    if read == 0 {
        return Ok(HeaderStatus::Eof);
    }
    if read < HEADER_LEN {
        return Ok(HeaderStatus::Torn {
            offset,
            message: format!("partial WAL header: {read}/{HEADER_LEN} bytes"),
        });
    }

    let magic = u32::from_le_bytes(header[0..4].try_into().expect("magic width"));
    if magic != MAGIC {
        return Ok(HeaderStatus::Torn {
            offset,
            message: format!("bad WAL magic 0x{magic:08x}"),
        });
    }

    let seq = u64::from_le_bytes(header[4..12].try_into().expect("seq width"));
    let len = u32::from_le_bytes(header[12..16].try_into().expect("len width"));
    let expected_crc = u32::from_le_bytes(header[16..20].try_into().expect("crc width"));
    if len > MAX_RECORD_BYTES {
        return Ok(HeaderStatus::Torn {
            offset,
            message: format!("record length {len} exceeds max {MAX_RECORD_BYTES}"),
        });
    }
    Ok(HeaderStatus::Complete(RecordHeader {
        seq,
        len,
        expected_crc,
        start_offset: offset,
        end_offset: offset + HEADER_LEN as u64 + len as u64,
    }))
}

struct FragmentHeader {
    seq: u64,
    frag_index: u32,
    frag_count: u32,
    logical_len: u64,
    frag_len: u32,
    expected_crc: u32,
}

enum FragmentHeaderStatus {
    Complete(FragmentHeader),
    Eof,
    Torn { message: String },
}

fn read_fragment_header_at(file: &mut File, offset: u64) -> io::Result<FragmentHeaderStatus> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; FRAGMENT_HEADER_LEN];
    let read = fill(file, &mut header)?;
    if read == 0 {
        return Ok(FragmentHeaderStatus::Eof);
    }
    if read < FRAGMENT_HEADER_LEN {
        return Ok(FragmentHeaderStatus::Torn {
            message: format!("partial WAL fragment header: {read}/{FRAGMENT_HEADER_LEN} bytes"),
        });
    }
    let magic = u32::from_le_bytes(header[0..4].try_into().expect("magic width"));
    if magic != MAGIC_FRAGMENT {
        return Ok(FragmentHeaderStatus::Torn {
            message: format!("expected CXW2 fragment magic, found 0x{magic:08x}"),
        });
    }
    let frag_len = u32::from_le_bytes(header[28..32].try_into().expect("frag_len width"));
    if frag_len > MAX_RECORD_BYTES {
        return Ok(FragmentHeaderStatus::Torn {
            message: format!("fragment payload length {frag_len} exceeds max {MAX_RECORD_BYTES}"),
        });
    }
    Ok(FragmentHeaderStatus::Complete(FragmentHeader {
        seq: u64::from_le_bytes(header[4..12].try_into().expect("seq width")),
        frag_index: u32::from_le_bytes(header[12..16].try_into().expect("frag_index width")),
        frag_count: u32::from_le_bytes(header[16..20].try_into().expect("frag_count width")),
        logical_len: u64::from_le_bytes(header[20..28].try_into().expect("logical_len width")),
        frag_len,
        expected_crc: u32::from_le_bytes(header[32..36].try_into().expect("crc width")),
    }))
}

/// Validates one fragment header against the running chain state, returning a
/// message if the chain framing is inconsistent (self-describing corruption).
fn fragment_framing_error(
    header: &FragmentHeader,
    expected_index: u32,
    chain_seq: &mut Option<u64>,
    chain_count: &mut Option<u32>,
    chain_logical_len: &mut Option<u64>,
) -> Option<String> {
    if header.frag_count == 0 {
        return Some(format!(
            "fragment for seq {} declares frag_count 0",
            header.seq
        ));
    }
    if header.frag_index >= header.frag_count {
        return Some(format!(
            "fragment for seq {} has index {} beyond count {}",
            header.seq, header.frag_index, header.frag_count
        ));
    }
    if header.frag_index != expected_index {
        return Some(format!(
            "fragment chain for seq {} expected index {expected_index}, found {}",
            header.seq, header.frag_index
        ));
    }
    if header.logical_len > MAX_LOGICAL_PAYLOAD_BYTES {
        return Some(format!(
            "fragment chain for seq {} declares logical length {} beyond ceiling {MAX_LOGICAL_PAYLOAD_BYTES}",
            header.seq, header.logical_len
        ));
    }
    match chain_seq {
        None => *chain_seq = Some(header.seq),
        Some(seq) if *seq != header.seq => {
            return Some(format!(
                "fragment chain seq changed from {seq} to {}",
                header.seq
            ));
        }
        Some(_) => {}
    }
    match chain_count {
        None => *chain_count = Some(header.frag_count),
        Some(count) if *count != header.frag_count => {
            return Some(format!(
                "fragment chain for seq {} count changed from {count} to {}",
                header.seq, header.frag_count
            ));
        }
        Some(_) => {}
    }
    match chain_logical_len {
        None => *chain_logical_len = Some(header.logical_len),
        Some(len) if *len != header.logical_len => {
            return Some(format!(
                "fragment chain for seq {} logical length changed from {len} to {}",
                header.seq, header.logical_len
            ));
        }
        Some(_) => {}
    }
    None
}

enum MagicPeek {
    Magic(u32),
    Eof,
    Torn { message: String },
}

fn peek_magic(file: &mut File, offset: u64) -> io::Result<MagicPeek> {
    file.seek(SeekFrom::Start(offset))?;
    let mut magic = [0u8; 4];
    let read = fill(file, &mut magic)?;
    if read == 0 {
        return Ok(MagicPeek::Eof);
    }
    if read < 4 {
        return Ok(MagicPeek::Torn {
            message: format!("partial WAL record magic: {read}/4 bytes"),
        });
    }
    Ok(MagicPeek::Magic(u32::from_le_bytes(magic)))
}

/// Reads until `buf` is full or EOF, returning the number of bytes read.
/// Distinguishes a clean EOF (`0`) from a partial read (`0 < n < buf.len()`)
/// without treating a short read on a healthy file as truncation.
fn fill(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn torn_chain(offset: u64, message: String) -> DecodeStatus {
    DecodeStatus::Torn { offset, message }
}

fn payload_crc(seq: u64, len: u32, payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(&len.to_le_bytes());
    hasher.update(payload);
    hasher.finalize()
}

fn fragment_crc(
    seq: u64,
    frag_index: u32,
    frag_count: u32,
    logical_len: u64,
    frag_len: u32,
    chunk: &[u8],
) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(&frag_index.to_le_bytes());
    hasher.update(&frag_count.to_le_bytes());
    hasher.update(&logical_len.to_le_bytes());
    hasher.update(&frag_len.to_le_bytes());
    hasher.update(chunk);
    hasher.finalize()
}
