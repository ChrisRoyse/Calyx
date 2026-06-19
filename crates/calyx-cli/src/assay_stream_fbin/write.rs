use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::assay_corpus_build::lens::{BuildLens, load_lenses, measure_text_batch};
use crate::error::{CliError, CliResult};

use super::args::Args;
use super::rows::{self, Row, RowStats};
use super::{MIN_A35_LENSES, io_error, local_error};

mod bits;
mod evidence;
mod panel;
mod paths;
mod progress;

use super::format::{self, VectorFormat};
use bits::{BitsLens, load_bits};
use evidence::{Evidence, LensEvidence, TEMPORAL_COUNTS_TOWARD_A35, TEMPORAL_LANE_ROLE};
use paths::{display, display_final, lens_prefix};

struct FbinSink {
    corpus: BufWriter<File>,
    queries: BufWriter<File>,
    format: VectorFormat,
    corpus_written: usize,
    query_written: usize,
}

struct LensStream<'a> {
    args: &'a Args,
    stats: &'a RowStats,
    lens: &'a BuildLens,
    effective_batch_size: usize,
    sink: &'a mut FbinSink,
    progress: &'a mut progress::ProgressLog,
}

struct StagedExport {
    evidence: Evidence,
    progress: progress::ProgressLog,
}

pub(crate) fn run(args: &Args) -> CliResult<Evidence> {
    let staging = staging_dir(&args.out_dir);
    fail_if_exists(&args.out_dir)?;
    fail_if_exists(&staging)?;
    panel::validate_floor_before_runtime(args)?;
    let lenses = selected_lenses(args)?;
    let stats = rows::scan(args)?;
    create_parent(&args.out_dir)?;
    fs::create_dir(&staging).map_err(io_error)?;
    let result = run_staged(args, &stats, lenses, &staging);
    match result {
        Ok(mut staged) => {
            if let Err(error) = fs::rename(&staging, &args.out_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(io_error(error));
            }
            if let Err(error) = staged.progress.export_finished_after_promotion() {
                let _ = fs::remove_dir_all(&args.out_dir);
                return Err(error);
            }
            Ok(staged.evidence)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn run_staged(
    args: &Args,
    stats: &RowStats,
    lenses: Vec<(BuildLens, BitsLens)>,
    staging: &Path,
) -> CliResult<StagedExport> {
    let vector_dir = staging.join(args.vector_format.dir_name());
    let vault_root = staging.join("vaults");
    fs::create_dir_all(&vector_dir).map_err(io_error)?;
    fs::create_dir_all(&vault_root).map_err(io_error)?;
    let mut roster = Vec::with_capacity(lenses.len());
    let mut progress = progress::ProgressLog::create(
        &staging.join(progress::FILE_NAME),
        args,
        stats,
        lenses.len(),
        staging,
    )?;
    for (slot, (lens, bits)) in lenses.into_iter().enumerate() {
        let prefix = lens_prefix(slot, lens.name());
        let ext = args.vector_format.extension();
        let corpus_path = vector_dir.join(format!("{prefix}_corpus.{ext}"));
        let queries_path = vector_dir.join(format!("{prefix}_queries.{ext}"));
        let mut sink = create_sink(&corpus_path, &queries_path, lens.dim(), stats.rows, args)?;
        let write_timeline = slot == 0;
        let timeline_path = staging.join("timeline.jsonl");
        let effective_batch_size = lens.effective_batch_size(args.batch_size);
        progress.lens_started(slot, &lens, bits.bits_about, effective_batch_size)?;
        let lens_started = Instant::now();
        stream_lens(
            LensStream {
                args,
                stats,
                lens: &lens,
                effective_batch_size,
                sink: &mut sink,
                progress: &mut progress,
            },
            write_timeline,
            &timeline_path,
        )?;
        let elapsed_ms = elapsed_ms(lens_started.elapsed())?;
        let ms_per_input = elapsed_ms as f64 / stats.rows.max(1) as f64;
        finish_sink(&mut sink)?;
        progress.lens_finished(sink.corpus_written, sink.query_written, elapsed_ms)?;
        roster.push(LensEvidence {
            slot: u16::try_from(slot).map_err(|_| CliError::usage("slot exceeds u16"))?,
            name: lens.name().to_string(),
            lens_id: lens.lens_id(),
            weights_sha256: lens.weights_sha256_hex(),
            bits_about: bits.bits_about,
            dim: lens.dim(),
            max_batch: lens.max_batch(),
            effective_batch_size,
            elapsed_ms,
            ms_per_input,
            manifest: display(lens.manifest()),
            corpus_path: display_final(
                args,
                &format!("{}/{prefix}_corpus.{ext}", args.vector_format.dir_name()),
            ),
            queries_path: display_final(
                args,
                &format!("{}/{prefix}_queries.{ext}", args.vector_format.dir_name()),
            ),
            vault_path: display_final(args, &format!("vaults/{prefix}")),
            corpus_rows_written: sink.corpus_written,
            query_rows_written: sink.query_written,
        });
    }
    write_plan(
        &staging.join("partitioned_rrf_plan.json"),
        &display_final(args, "timeline.jsonl"),
        &roster,
    )?;
    let evidence = Evidence {
        out_dir: display(&args.out_dir),
        rows_jsonl: display(&args.rows_jsonl),
        plan_path: display_final(args, "partitioned_rrf_plan.json"),
        timeline_path: display_final(args, "timeline.jsonl"),
        progress_path: display_final(args, progress::FILE_NAME),
        export_report_path: display_final(args, "stream_fbin_report.json"),
        vector_dir: display_final(args, args.vector_format.dir_name()),
        fbin_dir: (args.vector_format == VectorFormat::Fbin)
            .then(|| display_final(args, args.vector_format.dir_name())),
        vault_root: display_final(args, "vaults"),
        dataset: args.dataset.clone(),
        vector_format: args.vector_format,
        vector_storage_contract: args.vector_format.storage_contract(),
        rows: stats.clone(),
        query_count: args.query_count,
        batch_size: args.batch_size,
        min_bits: args.min_bits,
        streaming: true,
        temporal_counts_toward_a35: TEMPORAL_COUNTS_TOWARD_A35,
        temporal_lane_role: TEMPORAL_LANE_ROLE,
        lens_roster: roster,
    };
    fs::write(
        staging.join("stream_fbin_report.json"),
        serde_json::to_vec_pretty(&evidence).map_err(CliError::from)?,
    )
    .map_err(io_error)?;
    Ok(StagedExport { evidence, progress })
}

fn selected_lenses(args: &Args) -> CliResult<Vec<(BuildLens, BitsLens)>> {
    let request = args.corpus_request();
    let lenses = load_lenses(&request).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
            error,
            "fix the frozen lens manifests before streaming FBIN",
        )
    })?;
    let bits = load_bits(args)?;
    let mut selected = Vec::new();
    for lens in lenses {
        let Some(bits) = bits.get(lens.name()).cloned() else {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
                format!("lens {} missing from bits report", lens.name()),
                "run bits-validate and pass a report containing every streamed lens",
            ));
        };
        if !bits.admitted || !bits.bits_about.is_finite() || bits.bits_about < args.min_bits {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED",
                format!(
                    "lens {} admitted={} bits_about={} min_bits={}",
                    lens.name(),
                    bits.admitted,
                    bits.bits_about,
                    args.min_bits
                ),
                "stream only admitted signal-bearing lenses or lower --min-bits deliberately",
            ));
        }
        selected.push((lens, bits));
    }
    if selected.len() < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!(
                "selected {} admitted lenses; A35 requires at least {MIN_A35_LENSES}",
                selected.len()
            ),
            "provide at least ten real frozen content lens manifests",
        ));
    }
    Ok(selected)
}

fn create_sink(
    corpus_path: &Path,
    queries_path: &Path,
    dim: usize,
    rows: usize,
    args: &Args,
) -> CliResult<FbinSink> {
    let mut corpus = BufWriter::new(File::create(corpus_path).map_err(io_error)?);
    let mut queries = BufWriter::new(File::create(queries_path).map_err(io_error)?);
    format::write_header(&mut corpus, args.vector_format, dim, rows)?;
    format::write_header(&mut queries, args.vector_format, dim, args.query_count)?;
    Ok(FbinSink {
        corpus,
        queries,
        format: args.vector_format,
        corpus_written: 0,
        query_written: 0,
    })
}

fn stream_lens(
    mut stream: LensStream<'_>,
    write_timeline: bool,
    timeline_path: &Path,
) -> CliResult {
    let mut timeline = if write_timeline {
        Some(BufWriter::new(
            File::create(timeline_path).map_err(io_error)?,
        ))
    } else {
        None
    };
    let mut texts = Vec::with_capacity(stream.effective_batch_size);
    let mut metas = Vec::with_capacity(stream.effective_batch_size);
    rows::for_each_selected(stream.args, |row_idx, row| {
        texts.push(row.text.clone());
        metas.push((row_idx, row));
        if texts.len() >= stream.effective_batch_size {
            flush_batch(&mut stream, &mut timeline, &mut texts, &mut metas)?;
        }
        Ok(())
    })?;
    flush_batch(&mut stream, &mut timeline, &mut texts, &mut metas)?;
    if stream.sink.corpus_written != stream.stats.rows
        || stream.sink.query_written != stream.args.query_count
    {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_COUNT_MISMATCH",
            format!(
                "lens {} wrote corpus={} queries={} expected corpus={} queries={}",
                stream.lens.name(),
                stream.sink.corpus_written,
                stream.sink.query_written,
                stream.stats.rows,
                stream.args.query_count
            ),
            "inspect rows-jsonl selection and rerun stream-fbin",
        ));
    }
    if let Some(writer) = timeline.as_mut() {
        writer.flush().map_err(io_error)?;
        writer.get_ref().sync_all().map_err(io_error)?;
    }
    Ok(())
}

fn flush_batch(
    stream: &mut LensStream<'_>,
    timeline: &mut Option<BufWriter<File>>,
    texts: &mut Vec<String>,
    metas: &mut Vec<(usize, Row)>,
) -> CliResult {
    if texts.is_empty() {
        return Ok(());
    }
    let last_row_idx = metas.last().map(|(row_idx, _)| *row_idx).ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_BATCH_EMPTY",
            "batch metadata is empty while text batch is non-empty",
            "fix stream-fbin batching before trusting progress or FBIN output",
        )
    })?;
    let vectors =
        measure_text_batch(stream.lens, texts, stream.effective_batch_size).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_MEASURE",
                error,
                "inspect the lens runtime and source row batch",
            )
        })?;
    if vectors.len() != metas.len() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_VECTOR_COUNT_MISMATCH",
            format!(
                "lens {} returned {} vectors for {} rows",
                stream.lens.name(),
                vectors.len(),
                metas.len()
            ),
            "fix the lens runtime batch contract",
        ));
    }
    for (vector, (row_idx, row)) in vectors.iter().zip(metas.iter()) {
        validate_vector(stream.lens, vector)?;
        format::write_row(&mut stream.sink.corpus, stream.sink.format, vector)?;
        stream.sink.corpus_written += 1;
        if *row_idx < stream.args.query_count {
            format::write_row(&mut stream.sink.queries, stream.sink.format, vector)?;
            stream.sink.query_written += 1;
        }
        if let Some(writer) = timeline.as_mut() {
            write_timeline_row(writer, *row_idx, row, stream.args.query_count)?;
        }
    }
    stream.progress.batch_written(
        stream.sink.corpus_written,
        stream.sink.query_written,
        last_row_idx,
    )?;
    texts.clear();
    metas.clear();
    Ok(())
}

fn elapsed_ms(duration: Duration) -> CliResult<u64> {
    u64::try_from(duration.as_millis())
        .map_err(|_| CliError::usage("stream-fbin lens elapsed_ms exceeds u64"))
}

fn validate_vector(lens: &BuildLens, vector: &[f32]) -> CliResult {
    if vector.len() != lens.dim() || vector.iter().any(|value| !value.is_finite()) {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_VECTOR",
            format!(
                "lens {} produced dim={} expected={} or non-finite value",
                lens.name(),
                vector.len(),
                lens.dim()
            ),
            "inspect the offending lens runtime before trusting streamed FBIN",
        ));
    }
    Ok(())
}

fn write_timeline_row(
    writer: &mut BufWriter<File>,
    row_idx: usize,
    row: &Row,
    query_count: usize,
) -> CliResult {
    serde_json::to_writer(
        &mut *writer,
        &json!({
            "row_idx": row_idx,
            "id": row.id,
            "source_event_time_secs": row.event_time_secs,
            "source_event_time_raw": row.event_time_raw,
            "temporal_lane_state": row.temporal_lane_state,
            "temporal_inactive_reason": row.temporal_inactive_reason,
            "source_sequence": "jsonl_line",
            "source_sequence_index": row.source_sequence_index,
            "query_row": row_idx < query_count,
        }),
    )
    .map_err(CliError::from)?;
    writer.write_all(b"\n").map_err(io_error)
}

fn finish_sink(sink: &mut FbinSink) -> CliResult {
    sink.corpus.flush().map_err(io_error)?;
    sink.queries.flush().map_err(io_error)?;
    sink.corpus.get_ref().sync_all().map_err(io_error)?;
    sink.queries.get_ref().sync_all().map_err(io_error)
}

fn write_plan(path: &Path, timeline_path: &str, lenses: &[LensEvidence]) -> CliResult {
    let slots = lenses
        .iter()
        .map(|lens| {
            json!({
                "slot": lens.slot,
                "name": lens.name,
                "lens_id": lens.lens_id,
                "weights_sha256": lens.weights_sha256,
                "bits_about": lens.bits_about,
                "vault": lens.vault_path,
                "queries": lens.queries_path,
                "corpus": lens.corpus_path,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "timeline": timeline_path,
            "timeline_format": "calyx-assay-timeline-v1",
            "temporal_counts_toward_a35": TEMPORAL_COUNTS_TOWARD_A35,
            "temporal_lane_role": TEMPORAL_LANE_ROLE,
            "streaming_fbin_source": true,
            "slots": slots
        }))
        .map_err(CliError::from)?,
    )
    .map_err(io_error)
}

fn fail_if_exists(path: &Path) -> CliResult {
    if path.exists() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_OUTPUT_EXISTS",
            format!("{} already exists", path.display()),
            "choose a fresh immutable output directory",
        ));
    }
    Ok(())
}

fn create_parent(path: &Path) -> CliResult {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    Ok(())
}

fn staging_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("assay-stream-fbin");
    out_dir.with_file_name(format!(".{name}.tmp-{}", process::id()))
}
