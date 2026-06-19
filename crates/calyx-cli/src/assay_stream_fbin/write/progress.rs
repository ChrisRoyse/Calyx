use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::assay_corpus_build::lens::BuildLens;
use crate::error::{CliError, CliResult};

use super::super::args::Args;
use super::super::rows::RowStats;
use super::super::{io_error, local_error};
use super::paths::display;

pub(super) const FILE_NAME: &str = "stream_fbin_progress.json";

const SCHEMA: &str = "calyx-assay-stream-fbin-progress-v1";
const SNAPSHOT_ROW_INTERVAL: usize = 10_000;

pub(super) struct ProgressLog {
    path: PathBuf,
    tmp_path: PathBuf,
    published_path: PathBuf,
    state: ProgressState,
    sequence: u64,
    last_snapshot_row: usize,
}

#[derive(Clone, Debug, Serialize)]
struct ProgressState {
    state: &'static str,
    dataset: String,
    rows_jsonl: String,
    out_dir: String,
    staging_dir: String,
    rows_total: usize,
    query_count: usize,
    batch_size: usize,
    min_bits: f32,
    vector_format: &'static str,
    vector_storage_contract: &'static str,
    streaming_fbin_source: bool,
    temporal_counts_toward_a35: bool,
    temporal_lane_role: &'static str,
    lens_total: usize,
    lenses_completed: usize,
    completed_corpus_rows: usize,
    completed_query_rows: usize,
    current_lens: Option<LensProgress>,
}

#[derive(Clone, Debug, Serialize)]
struct LensProgress {
    slot: u16,
    name: String,
    lens_id: String,
    weights_sha256: String,
    bits_about: f32,
    dim: usize,
    max_batch: Option<usize>,
    manifest: String,
    corpus_rows_written: usize,
    query_rows_written: usize,
    last_row_idx: Option<usize>,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    schema: &'static str,
    state: &'static str,
    event: &'static str,
    sequence: u64,
    updated_unix_ms: u64,
    dataset: &'a str,
    rows_jsonl: &'a str,
    out_dir: &'a str,
    staging_dir: &'a str,
    progress_path: String,
    rows_total: usize,
    query_count: usize,
    batch_size: usize,
    min_bits: f32,
    vector_format: &'static str,
    vector_storage_contract: &'static str,
    streaming_fbin_source: bool,
    temporal_counts_toward_a35: bool,
    temporal_lane_role: &'static str,
    lens_total: usize,
    lenses_completed: usize,
    completed_corpus_rows: usize,
    completed_query_rows: usize,
    current_lens: Option<&'a LensProgress>,
    total_lens_corpus_rows_expected: usize,
    total_lens_query_rows_expected: usize,
    percent_complete_basis: &'static str,
    percent_complete: f64,
}

impl ProgressLog {
    pub(super) fn create(
        path: &Path,
        args: &Args,
        stats: &RowStats,
        lens_total: usize,
        staging: &Path,
    ) -> CliResult<Self> {
        let tmp_path = path.with_extension("json.tmp");
        let published_path = args.out_dir.join(FILE_NAME);
        let mut log = Self {
            path: path.to_path_buf(),
            tmp_path,
            published_path,
            state: ProgressState {
                state: "running",
                dataset: args.dataset.clone(),
                rows_jsonl: display(&args.rows_jsonl),
                out_dir: display(&args.out_dir),
                staging_dir: display(staging),
                rows_total: stats.rows,
                query_count: args.query_count,
                batch_size: args.batch_size,
                min_bits: args.min_bits,
                vector_format: args.vector_format.as_str(),
                vector_storage_contract: args.vector_format.storage_contract(),
                streaming_fbin_source: true,
                temporal_counts_toward_a35: false,
                temporal_lane_role: "event_time_forward_backward_as_of_sidecar",
                lens_total,
                lenses_completed: 0,
                completed_corpus_rows: 0,
                completed_query_rows: 0,
                current_lens: None,
            },
            sequence: 0,
            last_snapshot_row: 0,
        };
        log.write_snapshot("export_started")?;
        Ok(log)
    }

    pub(super) fn lens_started(
        &mut self,
        slot: usize,
        lens: &BuildLens,
        bits_about: f32,
    ) -> CliResult {
        self.state.current_lens = Some(LensProgress {
            slot: u16::try_from(slot).map_err(|_| CliError::usage("slot exceeds u16"))?,
            name: lens.name().to_string(),
            lens_id: lens.lens_id(),
            weights_sha256: lens.weights_sha256_hex(),
            bits_about,
            dim: lens.dim(),
            max_batch: lens.max_batch(),
            manifest: display(lens.manifest()),
            corpus_rows_written: 0,
            query_rows_written: 0,
            last_row_idx: None,
        });
        self.last_snapshot_row = 0;
        self.write_snapshot("lens_started")
    }

    pub(super) fn batch_written(
        &mut self,
        corpus_rows_written: usize,
        query_rows_written: usize,
        last_row_idx: usize,
    ) -> CliResult {
        if let Some(lens) = self.state.current_lens.as_mut() {
            lens.corpus_rows_written = corpus_rows_written;
            lens.query_rows_written = query_rows_written;
            lens.last_row_idx = Some(last_row_idx);
        }
        if corpus_rows_written == self.state.rows_total
            || corpus_rows_written.saturating_sub(self.last_snapshot_row) >= SNAPSHOT_ROW_INTERVAL
        {
            self.last_snapshot_row = corpus_rows_written;
            self.write_snapshot("batch_written")?;
        }
        Ok(())
    }

    pub(super) fn lens_finished(
        &mut self,
        corpus_rows_written: usize,
        query_rows_written: usize,
    ) -> CliResult {
        if let Some(lens) = self.state.current_lens.as_mut() {
            lens.corpus_rows_written = corpus_rows_written;
            lens.query_rows_written = query_rows_written;
            lens.last_row_idx = corpus_rows_written.checked_sub(1);
        }
        self.state.lenses_completed += 1;
        self.state.completed_corpus_rows += corpus_rows_written;
        self.state.completed_query_rows += query_rows_written;
        self.write_snapshot("lens_finished")
    }

    pub(super) fn export_finished_after_promotion(&mut self) -> CliResult {
        self.path = self.published_path.clone();
        self.tmp_path = self.path.with_extension("json.tmp");
        self.state.state = "complete";
        self.state.current_lens = None;
        self.write_snapshot("export_complete")
    }

    fn write_snapshot(&mut self, event: &'static str) -> CliResult {
        self.sequence += 1;
        let snapshot = self.snapshot(event)?;
        let mut file = File::create(&self.tmp_path).map_err(io_error)?;
        serde_json::to_writer_pretty(&mut file, &snapshot).map_err(CliError::from)?;
        file.write_all(b"\n").map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        fs::rename(&self.tmp_path, &self.path).map_err(io_error)?;
        sync_parent_dir(&self.path)
    }

    fn snapshot(&self, event: &'static str) -> CliResult<Snapshot<'_>> {
        let expected_corpus = self.state.rows_total.saturating_mul(self.state.lens_total);
        let expected_queries = self.state.query_count.saturating_mul(self.state.lens_total);
        let in_flight = self
            .state
            .current_lens
            .as_ref()
            .map(|lens| lens.corpus_rows_written)
            .unwrap_or(0);
        let completed = self.state.completed_corpus_rows.saturating_add(in_flight);
        let percent_complete = if expected_corpus == 0 {
            1.0
        } else {
            completed as f64 / expected_corpus as f64
        };
        Ok(Snapshot {
            schema: SCHEMA,
            state: self.state.state,
            event,
            sequence: self.sequence,
            updated_unix_ms: unix_ms()?,
            dataset: &self.state.dataset,
            rows_jsonl: &self.state.rows_jsonl,
            out_dir: &self.state.out_dir,
            staging_dir: &self.state.staging_dir,
            progress_path: display(&self.published_path),
            rows_total: self.state.rows_total,
            query_count: self.state.query_count,
            batch_size: self.state.batch_size,
            min_bits: self.state.min_bits,
            vector_format: self.state.vector_format,
            vector_storage_contract: self.state.vector_storage_contract,
            streaming_fbin_source: self.state.streaming_fbin_source,
            temporal_counts_toward_a35: self.state.temporal_counts_toward_a35,
            temporal_lane_role: self.state.temporal_lane_role,
            lens_total: self.state.lens_total,
            lenses_completed: self.state.lenses_completed,
            completed_corpus_rows: self.state.completed_corpus_rows,
            completed_query_rows: self.state.completed_query_rows,
            current_lens: self.state.current_lens.as_ref(),
            total_lens_corpus_rows_expected: expected_corpus,
            total_lens_query_rows_expected: expected_queries,
            percent_complete_basis: "completed_content_lens_corpus_rows",
            percent_complete,
        })
    }
}

fn unix_ms() -> CliResult<u64> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_CLOCK",
                format!("system time before Unix epoch: {error}"),
                "fix host clock before trusting progress timestamps",
            )
        })?
        .as_millis();
    u64::try_from(ms).map_err(|_| CliError::usage("progress timestamp exceeds u64"))
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> CliResult {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(io_error)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> CliResult {
    Ok(())
}
