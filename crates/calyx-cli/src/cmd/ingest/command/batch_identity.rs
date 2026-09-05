use std::io::{self, Read};
use std::path::Path;

use calyx_core::CalyxError;
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

const CALYX_INGEST_BATCH_CHANGED: &str = "CALYX_INGEST_BATCH_CHANGED";
const BATCH_CHANGED_REMEDIATION: &str =
    "stop modifying the batch input, restore the exact validated bytes, and retry";

/// Immutable content identity of a batch JSONL file, captured once via a
/// streaming byte hash before any authoritative work.
///
/// The batch pipeline re-reads the same file in several phases (parser
/// preflight, structured preflight, session start, replay preflight, and the
/// streaming write). Before this snapshot the only cross-phase tie was the
/// non-blank row count, so a same-row-count mutation between validation and the
/// authoritative write was invisible (a TOCTOU into Base/slot/WAL/Ledger). This
/// struct is threaded into every phase; each phase re-verifies the digest of the
/// bytes it actually consumed and fails with `CALYX_INGEST_BATCH_CHANGED` before
/// any mutation. `nonblank_row_count` stays as the cheap fast-fail; `sha256` and
/// `byte_len` are the authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BatchSnapshotIdentity {
    pub(crate) sha256: String,
    pub(crate) byte_len: u64,
    pub(crate) nonblank_row_count: usize,
}

impl BatchSnapshotIdentity {
    /// Capture the identity in one streaming pass over the raw file bytes.
    /// `nonblank_row_count` comes from the parser preflight (`validate_batch_file`);
    /// the hash and byte length are computed here so no other phase has to trust a
    /// stat-only length or a decorative recompute.
    pub(crate) fn capture(path: &Path, nonblank_row_count: usize) -> CliResult<Self> {
        let mut file = std::fs::File::open(path).map_err(|err| {
            CliError::io(format!(
                "open batch {} for snapshot identity: {err}",
                path.display()
            ))
        })?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut byte_len = 0u64;
        loop {
            let read = file.read(&mut buf).map_err(|err| {
                CliError::io(format!(
                    "read batch {} for snapshot identity: {err}",
                    path.display()
                ))
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
            byte_len += read as u64;
        }
        Ok(Self {
            sha256: format!("{:x}", hasher.finalize()),
            byte_len,
            nonblank_row_count,
        })
    }

    /// Fail with `CALYX_INGEST_BATCH_CHANGED` unless the observed digest and byte
    /// length match the captured snapshot. `context` names the phase that consumed
    /// the bytes so the operator can see exactly where the divergence was caught.
    pub(crate) fn verify_observed(
        &self,
        path: &Path,
        observed: &ObservedBatchBytes,
        context: &str,
    ) -> CliResult<()> {
        if observed.sha256 == self.sha256 && observed.byte_len == self.byte_len {
            return Ok(());
        }
        Err(CliError::from(CalyxError {
            code: CALYX_INGEST_BATCH_CHANGED,
            message: format!(
                "batch {} changed between snapshot and {context}: snapshot sha256={} byte_len={}, observed sha256={} byte_len={}",
                path.display(),
                self.sha256,
                self.byte_len,
                observed.sha256,
                observed.byte_len,
            ),
            remediation: BATCH_CHANGED_REMEDIATION,
        }))
    }

    /// Fail-fast row-count guard (cheap; the digest is the authority).
    pub(crate) fn verify_row_count(
        &self,
        path: &Path,
        observed_rows: usize,
        context: &str,
    ) -> CliResult<()> {
        if observed_rows == self.nonblank_row_count {
            return Ok(());
        }
        Err(CliError::from(CalyxError {
            code: CALYX_INGEST_BATCH_CHANGED,
            message: format!(
                "batch {} changed between snapshot and {context}: expected {} rows, observed {observed_rows}",
                path.display(),
                self.nonblank_row_count,
            ),
            remediation: BATCH_CHANGED_REMEDIATION,
        }))
    }
}

/// The digest and byte length of the bytes a phase actually consumed, produced by
/// [`HashingReader::finish`].
pub(crate) struct ObservedBatchBytes {
    pub(crate) sha256: String,
    pub(crate) byte_len: u64,
}

/// A reader that maintains a running SHA-256 over every byte pulled from the
/// underlying source. A phase wraps its file in this reader (under a
/// `BufReader`), reads to end-of-stream as usual, then calls [`finish`] to obtain
/// the digest of exactly the bytes it consumed and verifies it against the
/// captured [`BatchSnapshotIdentity`] before any commit.
///
/// [`finish`]: HashingReader::finish
pub(crate) struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    byte_len: u64,
}

impl<R: Read> HashingReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            byte_len: 0,
        }
    }

    pub(crate) fn finish(self) -> ObservedBatchBytes {
        ObservedBatchBytes {
            sha256: format!("{:x}", self.hasher.finalize()),
            byte_len: self.byte_len,
        }
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        self.byte_len += read as u64;
        Ok(read)
    }
}
