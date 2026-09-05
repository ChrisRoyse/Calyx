//! Stateful readback cache for on-chain backfill verification (issue #214).
//!
//! Extracted verbatim from `raw_onchain_backfill_runner_readback` to keep every source file under the
//! 500-line doctrine limit. `ReadbackContext` owns the dedup cache of already-read artifacts, the
//! byte/parse counters, the failure sinks, and the progress-log writer. Its fields and methods are
//! `pub(crate)` so the parent orchestrator can drive it and assemble the final report; nothing here
//! is exported beyond the crate.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::IgnoredAny;
use sha2::{Digest, Sha256};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_onchain_backfill_readback_checks::{canonical_display_path, sha256_page_metadata};
use crate::raw_onchain_backfill_runner_types::ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION;
use crate::raw_source_support::{display_safe_path, sha256_hex};
use crate::{PolyError, Result};

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    byte_count: u64,
}

impl<R: Read> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            byte_count: 0,
        }
    }

    fn finish(self) -> (String, u64) {
        (format!("{:x}", self.hasher.finalize()), self.byte_count)
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.hasher.update(&buf[..read]);
            self.byte_count = self.byte_count.saturating_add(read as u64);
        }
        Ok(read)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OnchainBackfillReadbackProgressEvent {
    schema_version: String,
    event_code: String,
    phase: String,
    output_root: String,
    page_index: Option<usize>,
    page_count: Option<usize>,
    checkpoint_range_index: Option<usize>,
    checkpoint_range_count: Option<usize>,
    path: Option<String>,
    checked_file_count: usize,
    unique_file_read_count: usize,
    deduplicated_file_read_count: usize,
    json_parse_count: usize,
    readback_bytes_read: u64,
    readback_body_bytes_read: u64,
    readback_request_bytes_read: u64,
    readback_metadata_bytes_read: u64,
    missing_file_count: usize,
    sha_mismatch_count: usize,
    parse_failure_count: usize,
    progress_event_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadbackArtifactKind {
    Body,
    Request,
    Metadata,
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedArtifact {
    pub(crate) actual_sha256: String,
    pub(crate) byte_count: u64,
    pub(crate) json_parse_checked: bool,
    pub(crate) json_parse_ok: bool,
}

pub(crate) struct ReadbackContext {
    pub(crate) root: PathBuf,
    pub(crate) progress_path: PathBuf,
    pub(crate) progress_writer: BufWriter<File>,
    pub(crate) progress_event_count: usize,
    pub(crate) checked_file_count: usize,
    pub(crate) unique_file_read_count: usize,
    pub(crate) deduplicated_file_read_count: usize,
    pub(crate) json_parse_count: usize,
    pub(crate) readback_bytes_read: u64,
    pub(crate) readback_body_bytes_read: u64,
    pub(crate) readback_request_bytes_read: u64,
    pub(crate) readback_metadata_bytes_read: u64,
    pub(crate) missing_files: Vec<String>,
    pub(crate) sha_mismatches: Vec<String>,
    pub(crate) parse_failures: Vec<String>,
    pub(crate) artifacts: BTreeMap<String, CachedArtifact>,
    pub(crate) pages_by_metadata_path: BTreeMap<String, LargeCorpusPage>,
    pub(crate) pages_by_body_path: BTreeMap<String, LargeCorpusPage>,
}

impl ReadbackContext {
    pub(crate) fn new(root: &Path, progress_file: &str) -> Result<Self> {
        let progress_path = root.join(progress_file);
        let progress_file = File::create(&progress_path).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_READBACK_PROGRESS_CREATE_FAILED",
                format!("create progress log {}: {err}", progress_path.display()),
            )
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            progress_path,
            progress_writer: BufWriter::new(progress_file),
            progress_event_count: 0,
            checked_file_count: 0,
            unique_file_read_count: 0,
            deduplicated_file_read_count: 0,
            json_parse_count: 0,
            readback_bytes_read: 0,
            readback_body_bytes_read: 0,
            readback_request_bytes_read: 0,
            readback_metadata_bytes_read: 0,
            missing_files: Vec::new(),
            sha_mismatches: Vec::new(),
            parse_failures: Vec::new(),
            artifacts: BTreeMap::new(),
            pages_by_metadata_path: BTreeMap::new(),
            pages_by_body_path: BTreeMap::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit(
        &mut self,
        event_code: &str,
        phase: &str,
        page_index: Option<usize>,
        page_count: Option<usize>,
        checkpoint_range_index: Option<usize>,
        checkpoint_range_count: Option<usize>,
        path: Option<String>,
    ) -> Result<()> {
        let event = OnchainBackfillReadbackProgressEvent {
            schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
            event_code: event_code.to_string(),
            phase: phase.to_string(),
            output_root: self.root.display().to_string(),
            page_index,
            page_count,
            checkpoint_range_index,
            checkpoint_range_count,
            path,
            checked_file_count: self.checked_file_count,
            unique_file_read_count: self.unique_file_read_count,
            deduplicated_file_read_count: self.deduplicated_file_read_count,
            json_parse_count: self.json_parse_count,
            readback_bytes_read: self.readback_bytes_read,
            readback_body_bytes_read: self.readback_body_bytes_read,
            readback_request_bytes_read: self.readback_request_bytes_read,
            readback_metadata_bytes_read: self.readback_metadata_bytes_read,
            missing_file_count: self.missing_files.len(),
            sha_mismatch_count: self.sha_mismatches.len(),
            parse_failure_count: self.parse_failures.len(),
            progress_event_count: self.progress_event_count + 1,
        };
        let bytes = serde_json::to_vec(&event).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_READBACK_PROGRESS_ENCODE_FAILED",
                format!("encode progress event {event_code}: {err}"),
            )
        })?;
        self.progress_writer.write_all(&bytes).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_READBACK_PROGRESS_WRITE_FAILED",
                format!("write progress event {event_code}: {err}"),
            )
        })?;
        self.progress_writer.write_all(b"\n").map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_READBACK_PROGRESS_WRITE_FAILED",
                format!("write progress newline for {event_code}: {err}"),
            )
        })?;
        self.progress_event_count += 1;
        Ok(())
    }

    pub(crate) fn flush_progress(&mut self) -> Result<()> {
        self.progress_writer.flush().map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_READBACK_PROGRESS_FLUSH_FAILED",
                format!("flush progress log {}: {err}", self.progress_path.display()),
            )
        })
    }

    pub(crate) fn path_key(&self, path: &Path) -> String {
        canonical_display_path(path)
            .unwrap_or_else(|| display_safe_path(path.to_path_buf()))
            .display()
            .to_string()
    }

    pub(crate) fn record_artifact_read(&mut self, kind: ReadbackArtifactKind, byte_count: u64) {
        self.checked_file_count += 1;
        self.unique_file_read_count += 1;
        self.readback_bytes_read = self.readback_bytes_read.saturating_add(byte_count);
        match kind {
            ReadbackArtifactKind::Body => {
                self.readback_body_bytes_read =
                    self.readback_body_bytes_read.saturating_add(byte_count);
            }
            ReadbackArtifactKind::Request => {
                self.readback_request_bytes_read =
                    self.readback_request_bytes_read.saturating_add(byte_count);
            }
            ReadbackArtifactKind::Metadata | ReadbackArtifactKind::Control => {
                self.readback_metadata_bytes_read =
                    self.readback_metadata_bytes_read.saturating_add(byte_count);
            }
        }
    }

    pub(crate) fn check_artifact_sha(
        &mut self,
        path: &Path,
        expected: &str,
        kind: ReadbackArtifactKind,
        expect_json: bool,
    ) -> Result<()> {
        let key = self.path_key(path);
        if let Some(cached) = self.artifacts.get(&key) {
            self.deduplicated_file_read_count += 1;
            let actual = cached.actual_sha256.clone();
            let json_checked = cached.json_parse_checked;
            let json_ok = cached.json_parse_ok;
            if actual != expected {
                self.sha_mismatches.push(format!(
                    "{} expected {} actual {}",
                    path.display(),
                    expected,
                    actual
                ));
            }
            if expect_json && (!json_checked || !json_ok) {
                self.parse_failures.push(format!(
                    "{} JSON parse was not clean in cached readback state",
                    path.display()
                ));
            }
            return Ok(());
        }

        let Some((actual, byte_count, json_parse_checked, json_parse_ok)) =
            self.read_artifact_state(path, kind, expect_json)?
        else {
            return Ok(());
        };
        if actual != expected {
            self.sha_mismatches.push(format!(
                "{} expected {} actual {}",
                path.display(),
                expected,
                actual
            ));
        }
        self.artifacts.insert(
            key,
            CachedArtifact {
                actual_sha256: actual,
                byte_count,
                json_parse_checked,
                json_parse_ok,
            },
        );
        Ok(())
    }

    fn read_artifact_state(
        &mut self,
        path: &Path,
        kind: ReadbackArtifactKind,
        expect_json: bool,
    ) -> Result<Option<(String, u64, bool, bool)>> {
        if !expect_json {
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.missing_files
                        .push(format!("{} read failed: {err}", path.display()));
                    return Ok(None);
                }
            };
            let byte_count = bytes.len() as u64;
            self.record_artifact_read(kind, byte_count);
            return Ok(Some((sha256_hex(&bytes), byte_count, false, true)));
        }

        let file = match File::open(path) {
            Ok(file) => file,
            Err(err) => {
                self.missing_files
                    .push(format!("{} read failed: {err}", path.display()));
                return Ok(None);
            }
        };
        let mut reader = HashingReader::new(BufReader::new(file));
        let mut json_parse_ok = true;
        self.json_parse_count += 1;
        if let Err(err) = serde_json::from_reader::<_, IgnoredAny>(&mut reader) {
            json_parse_ok = false;
            self.parse_failures
                .push(format!("parse JSON {}: {err}", path.display()));
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(read_err) => {
                    self.missing_files.push(format!(
                        "{} read failed after JSON parse failure: {read_err}",
                        path.display()
                    ));
                    return Ok(None);
                }
            };
            let byte_count = bytes.len() as u64;
            self.record_artifact_read(kind, byte_count);
            return Ok(Some((sha256_hex(&bytes), byte_count, true, json_parse_ok)));
        }
        let (actual, byte_count) = reader.finish();
        self.record_artifact_read(kind, byte_count);
        Ok(Some((actual, byte_count, true, json_parse_ok)))
    }

    pub(crate) fn check_json_file(
        &mut self,
        path: &Path,
        kind: ReadbackArtifactKind,
    ) -> Result<()> {
        let key = self.path_key(path);
        if let Some(cached) = self.artifacts.get(&key) {
            self.deduplicated_file_read_count += 1;
            if !cached.json_parse_checked || !cached.json_parse_ok {
                self.parse_failures.push(format!(
                    "{} JSON parse was not clean in cached readback state",
                    path.display()
                ));
            }
            return Ok(());
        }

        let Some((actual, byte_count, json_parse_checked, json_parse_ok)) =
            self.read_artifact_state(path, kind, true)?
        else {
            return Ok(());
        };
        self.artifacts.insert(
            key,
            CachedArtifact {
                actual_sha256: actual,
                byte_count,
                json_parse_checked,
                json_parse_ok,
            },
        );
        Ok(())
    }

    pub(crate) fn read_page_metadata(&mut self, path: &Path) -> Result<Option<LargeCorpusPage>> {
        let key = self.path_key(path);
        if let Some(page) = self.pages_by_metadata_path.get(&key) {
            self.deduplicated_file_read_count += 1;
            return Ok(Some(page.clone()));
        }

        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.missing_files
                    .push(format!("{} read failed: {err}", path.display()));
                return Ok(None);
            }
        };
        let byte_count = bytes.len() as u64;
        self.record_artifact_read(ReadbackArtifactKind::Metadata, byte_count);
        let actual = sha256_hex(&bytes);
        self.json_parse_count += 1;
        let page = match serde_json::from_slice::<LargeCorpusPage>(&bytes) {
            Ok(page) => page,
            Err(err) => {
                self.parse_failures
                    .push(format!("decode page metadata {}: {err}", path.display()));
                self.artifacts.insert(
                    key,
                    CachedArtifact {
                        actual_sha256: actual,
                        byte_count,
                        json_parse_checked: true,
                        json_parse_ok: false,
                    },
                );
                return Ok(None);
            }
        };
        self.artifacts.insert(
            key.clone(),
            CachedArtifact {
                actual_sha256: actual,
                byte_count,
                json_parse_checked: true,
                json_parse_ok: true,
            },
        );
        self.pages_by_body_path
            .insert(self.path_key(Path::new(&page.body_path)), page.clone());
        self.pages_by_metadata_path.insert(key, page.clone());
        Ok(Some(page))
    }

    pub(crate) fn check_page_metadata(&mut self, page: &LargeCorpusPage) -> Result<()> {
        let metadata_path = Path::new(&page.metadata_path);
        let expected_metadata_sha = match sha256_page_metadata(page) {
            Ok(expected_metadata_sha) => expected_metadata_sha,
            Err(err) => {
                self.parse_failures.push(err.message());
                return Ok(());
            }
        };
        let decoded = self.read_page_metadata(metadata_path)?;
        let key = self.path_key(metadata_path);
        if let Some(cached) = self.artifacts.get(&key)
            && cached.actual_sha256 != expected_metadata_sha
        {
            self.sha_mismatches.push(format!(
                "{} expected {} actual {}",
                metadata_path.display(),
                expected_metadata_sha,
                cached.actual_sha256
            ));
        }
        if let Some(decoded) = decoded
            && decoded != *page
        {
            self.parse_failures.push(format!(
                "{} decoded metadata did not match in-memory page state",
                metadata_path.display()
            ));
        }
        self.pages_by_body_path
            .insert(self.path_key(Path::new(&page.body_path)), page.clone());
        self.pages_by_metadata_path
            .insert(self.path_key(metadata_path), page.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    include!("raw_onchain_backfill_readback_context_tests.rs");
}
