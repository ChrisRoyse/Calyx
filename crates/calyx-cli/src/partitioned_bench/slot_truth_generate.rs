use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use calyx_sextant::index::{DenseVectorFile, cuvs_bruteforce_topk};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::a35_signal::require_recorded_countable_content_signal_kind;
use crate::error::{CliError, CliResult};

#[path = "slot_truth_generate/support.rs"]
mod support;

use support::{io_error, sha256_file, st_error};

const FORMAT: &str = "calyx-partitioned-rrf-slot-ground-truth-v1";
const MODE: &str = "per_slot_ranked_rrf_reference";
const ROW_ID_SPACE: &str = "partitioned_rrf_plan_corpus_row_idx";
const BACKEND: &str = "cuvs-bruteforce-chunked-v1";
const DEFAULT_CHUNK_ROWS: usize = 100_000;
const MIN_A35_LENSES: usize = 10;
#[derive(Clone, Debug)]
struct Args {
    plan: PathBuf,
    out_dir: PathBuf,
    query_count: usize,
    truth_depth: usize,
    chunk_rows: usize,
}
#[derive(Clone, Debug, Deserialize)]
struct Plan {
    slots: Vec<PlanSlot>,
}
#[derive(Clone, Debug, Deserialize)]
struct PlanSlot {
    slot: u16,
    name: Option<String>,
    lens_id: Option<String>,
    weights_sha256: Option<String>,
    signal_kind: Option<String>,
    corpus: PathBuf,
    queries: PathBuf,
}
#[derive(Clone, Debug)]
struct SlotEvidence {
    slot: u16,
    lens_id: String,
    weights_sha256: String,
    signal_kind: String,
    file: String,
    file_sha256: String,
    rows: usize,
    width: usize,
    corpus: String,
    queries: String,
    dim: usize,
    chunks: usize,
    elapsed_ms: u128,
}
impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let mut out_dir = None;
        let mut query_count = None;
        let mut truth_depth = None;
        let mut chunk_rows = DEFAULT_CHUNK_ROWS;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--plan" => plan = Some(PathBuf::from(next()?)),
                "--out-dir" => out_dir = Some(PathBuf::from(next()?)),
                "--query-count" => query_count = Some(super::parse(&next()?, "--query-count")?),
                "--truth-depth" => truth_depth = Some(super::parse(&next()?, "--truth-depth")?),
                "--chunk-rows" => chunk_rows = super::parse(&next()?, "--chunk-rows")?,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let plan = plan.ok_or_else(|| CliError::usage("--plan <json> is required"))?;
        let out_dir = out_dir.ok_or_else(|| CliError::usage("--out-dir <dir> is required"))?;
        let query_count =
            query_count.ok_or_else(|| CliError::usage("--query-count <n> is required"))?;
        let truth_depth =
            truth_depth.ok_or_else(|| CliError::usage("--truth-depth <n> is required"))?;
        if query_count == 0 || truth_depth == 0 || chunk_rows == 0 {
            return Err(CliError::usage(
                "--query-count, --truth-depth, and --chunk-rows must be > 0",
            ));
        }
        Ok(Self {
            plan,
            out_dir,
            query_count,
            truth_depth,
            chunk_rows,
        })
    }
}

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    fail_if_exists(&args.out_dir)?;
    let staging = staging_dir(&args.out_dir);
    fail_if_exists(&staging)?;
    create_parent(&args.out_dir)?;
    fs::create_dir(&staging).map_err(io_error)?;
    let result = run_staged(&args, &staging);
    match result {
        Ok(report) => {
            fs::rename(&staging, &args.out_dir).map_err(io_error)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(CliError::from)?
            );
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn run_staged(args: &Args, staging: &Path) -> CliResult<Value> {
    let plan = load_plan(&args.plan)?;
    validate_plan(&plan)?;
    let plan_sha256 = sha256_file(&args.plan)?;
    let plan_base = args.plan.parent().unwrap_or_else(|| Path::new(""));
    let first_corpus = DenseVectorFile::open(&resolve(plan_base, &plan.slots[0].corpus))
        .map_err(CliError::Calyx)?;
    let corpus_rows = usize::try_from(first_corpus.count()).map_err(|_| {
        st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_INVALID",
            "corpus row count exceeds usize",
            "use a supported corpus row count",
        )
    })?;
    if args.truth_depth > corpus_rows {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_INVALID",
            "truth depth exceeds corpus row count",
            "choose a truth depth <= corpus rows",
        ));
    }
    drop(first_corpus);
    let mut slots = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        slots.push(generate_slot(args, staging, plan_base, slot, corpus_rows)?);
    }
    let manifest = manifest(args, &plan_sha256, corpus_rows, &slots);
    let manifest_path = staging.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(CliError::from)?,
    )
    .map_err(io_error)?;
    let report = json!({
        "trigger": "calyx bench partitioned-rrf-slot-truth",
        "format": FORMAT,
        "mode": MODE,
        "row_id_space": ROW_ID_SPACE,
        "reference_backend": BACKEND,
        "scale_suitable": true,
        "plan": args.plan,
        "plan_sha256": plan_sha256,
        "out_dir": args.out_dir,
        "manifest": args.out_dir.join("manifest.json"),
        "manifest_sha256": sha256_file(&manifest_path)?,
        "query_count": args.query_count,
        "truth_depth": args.truth_depth,
        "corpus_rows": corpus_rows,
        "chunk_rows": args.chunk_rows,
        "slots": slots.iter().map(slot_report).collect::<Vec<_>>(),
    });
    fs::write(
        staging.join("generation_report.json"),
        serde_json::to_vec_pretty(&report).map_err(CliError::from)?,
    )
    .map_err(io_error)?;
    Ok(report)
}

fn generate_slot(
    args: &Args,
    staging: &Path,
    plan_base: &Path,
    slot: &PlanSlot,
    corpus_rows: usize,
) -> CliResult<SlotEvidence> {
    let started = Instant::now();
    let corpus_path = resolve(plan_base, &slot.corpus);
    let query_path = resolve(plan_base, &slot.queries);
    let corpus = DenseVectorFile::open(&corpus_path).map_err(CliError::Calyx)?;
    let queries = DenseVectorFile::open(&query_path).map_err(CliError::Calyx)?;
    validate_files(&corpus, &queries, args, corpus_rows, slot.slot)?;
    let mut query_rows = load_rows(&queries, 0, args.query_count)?;
    let mut merged = vec![Vec::<(u64, f32)>::new(); args.query_count];
    let mut chunks = 0usize;
    let mut base = 0usize;
    while base < corpus_rows {
        let take = args.chunk_rows.min(corpus_rows - base);
        let mut chunk = load_rows(&corpus, base, take)?;
        let chunk_k = args.truth_depth.min(take);
        let result = cuvs_bruteforce_topk(
            &mut chunk,
            take,
            corpus.dim(),
            &mut query_rows,
            args.query_count,
            chunk_k,
        )
        .map_err(CliError::Calyx)?;
        merge_chunk(&mut merged, &result, base, args.truth_depth)?;
        chunks += 1;
        base += take;
    }
    let file_name = format!("slot_{:02}_truth.i32bin", slot.slot);
    let file_path = staging.join(&file_name);
    write_i32bin(&file_path, &merged, args.truth_depth)?;
    Ok(SlotEvidence {
        slot: slot.slot,
        lens_id: required(&slot.lens_id, "lens_id", slot.slot)?,
        weights_sha256: required(&slot.weights_sha256, "weights_sha256", slot.slot)?,
        signal_kind: required(&slot.signal_kind, "signal_kind", slot.slot)?,
        file: file_name,
        file_sha256: sha256_file(&file_path)?,
        rows: args.query_count,
        width: args.truth_depth,
        corpus: corpus_path.display().to_string(),
        queries: query_path.display().to_string(),
        dim: corpus.dim(),
        chunks,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn merge_chunk(
    merged: &mut [Vec<(u64, f32)>],
    result: &calyx_sextant::index::CuvsBruteForceTopK,
    base: usize,
    depth: usize,
) -> CliResult {
    if result.query_count != merged.len() {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_BACKEND",
            "cuVS result query count mismatch",
            "inspect cuVS brute-force output before trusting generated truth",
        ));
    }
    for (query_idx, row) in merged.iter_mut().enumerate() {
        let (neighbors, distances) = result.row(query_idx);
        for (&neighbor, &distance) in neighbors.iter().zip(distances) {
            let neighbor = u64::try_from(neighbor).map_err(|_| {
                st_error(
                    "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_BACKEND",
                    "cuVS returned a negative row id",
                    "inspect cuVS brute-force output before trusting generated truth",
                )
            })?;
            row.push((base as u64 + neighbor, distance));
        }
        row.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
        row.dedup_by_key(|(id, _)| *id);
        row.truncate(depth);
    }
    Ok(())
}

fn validate_plan(plan: &Plan) -> CliResult {
    if plan.slots.len() < MIN_A35_LENSES {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_A35",
            format!(
                "plan has {} lenses; A35 requires at least {MIN_A35_LENSES}",
                plan.slots.len()
            ),
            "export a real frozen panel with at least ten content lenses",
        ));
    }
    let mut seen_slots = BTreeSet::new();
    let mut seen_lenses = HashSet::new();
    for slot in &plan.slots {
        let name = slot.name.as_deref().unwrap_or("<unnamed>");
        require_recorded_countable_content_signal_kind(
            name,
            slot.signal_kind.as_deref(),
            "partitioned-rrf-slot-truth A35 gate",
        )?;
        if !seen_slots.insert(slot.slot)
            || !seen_lenses.insert(required(&slot.lens_id, "lens_id", slot.slot)?)
        {
            return Err(st_error(
                "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_A35",
                "plan contains duplicate slot or lens_id",
                "use a unique frozen lens roster",
            ));
        }
        let _ = required(&slot.weights_sha256, "weights_sha256", slot.slot)?;
    }
    Ok(())
}

fn validate_files(
    corpus: &DenseVectorFile,
    queries: &DenseVectorFile,
    args: &Args,
    corpus_rows: usize,
    slot: u16,
) -> CliResult {
    if corpus.count() as usize != corpus_rows
        || queries.count() < args.query_count as u64
        || corpus.dim() != queries.dim()
    {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_MISMATCH",
            format!(
                "slot {slot} corpus_rows={} expected={corpus_rows} queries={} needs={} corpus_dim={} query_dim={}",
                corpus.count(),
                queries.count(),
                args.query_count,
                corpus.dim(),
                queries.dim()
            ),
            "regenerate/export a consistent partitioned RRF plan",
        ));
    }
    Ok(())
}

fn load_rows(file: &DenseVectorFile, start: usize, rows: usize) -> CliResult<Vec<f32>> {
    let mut out = Vec::with_capacity(rows * file.dim());
    for offset in 0..rows {
        out.extend(file.row_f32((start + offset) as u64));
    }
    Ok(out)
}

fn write_i32bin(path: &Path, rows: &[Vec<(u64, f32)>], width: usize) -> CliResult {
    let mut out = BufWriter::new(File::create(path).map_err(io_error)?);
    out.write_all(&(rows.len() as u32).to_le_bytes())
        .map_err(io_error)?;
    out.write_all(&(width as u32).to_le_bytes())
        .map_err(io_error)?;
    for (idx, row) in rows.iter().enumerate() {
        if row.len() < width {
            return Err(st_error(
                "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_BACKEND",
                format!("query {idx} produced {} ranks, expected {width}", row.len()),
                "inspect cuVS chunk output and corpus row count",
            ));
        }
        for (id, _) in row.iter().take(width) {
            let id = i32::try_from(*id).map_err(|_| {
                st_error(
                    "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_INVALID",
                    "row id exceeds i32 truth file format",
                    "use a row-id space that fits the persisted i32bin contract",
                )
            })?;
            out.write_all(&id.to_le_bytes()).map_err(io_error)?;
        }
    }
    out.flush().map_err(io_error)?;
    out.get_ref().sync_all().map_err(io_error)
}

fn manifest(args: &Args, plan_sha256: &str, corpus_rows: usize, slots: &[SlotEvidence]) -> Value {
    json!({
        "format": FORMAT,
        "mode": MODE,
        "row_id_space": ROW_ID_SPACE,
        "plan_sha256": plan_sha256,
        "query_count": args.query_count,
        "truth_depth": args.truth_depth,
        "corpus_rows": corpus_rows,
        "reference_backend": BACKEND,
        "scale_suitable": true,
        "chunk_rows": args.chunk_rows,
        "slots": slots.iter().map(|slot| json!({
            "slot": slot.slot,
            "lens_id": slot.lens_id,
            "weights_sha256": slot.weights_sha256,
            "signal_kind": slot.signal_kind,
            "file": slot.file,
            "file_sha256": slot.file_sha256,
            "rows": slot.rows,
            "width": slot.width,
        })).collect::<Vec<_>>(),
    })
}

fn slot_report(slot: &SlotEvidence) -> Value {
    json!({
        "slot": slot.slot,
        "lens_id": slot.lens_id,
        "weights_sha256": slot.weights_sha256,
        "signal_kind": slot.signal_kind,
        "file": slot.file,
        "file_sha256": slot.file_sha256,
        "rows": slot.rows,
        "width": slot.width,
        "corpus": slot.corpus,
        "queries": slot.queries,
        "dim": slot.dim,
        "chunks": slot.chunks,
        "elapsed_ms": slot.elapsed_ms,
    })
}

fn load_plan(path: &Path) -> CliResult<Plan> {
    let text = fs::read_to_string(path).map_err(io_error)?;
    serde_json::from_str(&text).map_err(|error| {
        st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_PLAN_INVALID",
            format!("parse {} failed: {error}", path.display()),
            "pass a partitioned_rrf_plan.json produced by assay export-fbin",
        )
    })
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn required(value: &Option<String>, field: &'static str, slot: u16) -> CliResult<String> {
    value
        .clone()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            st_error(
                "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_A35",
                format!("slot {slot} missing {field}"),
                "export a plan that records every frozen lens id and weights hash",
            )
        })
}

fn fail_if_exists(path: &Path) -> CliResult {
    if path.exists() {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_OUTPUT_EXISTS",
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
        .unwrap_or("partitioned-rrf-slot-truth");
    out_dir.with_file_name(format!(".{name}.tmp-{}", process::id()))
}
