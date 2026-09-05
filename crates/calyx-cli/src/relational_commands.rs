use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use calyx_core::CalyxError;
use calyx_turso::{CALYX_TURSO_RECORD_INVALID, CalyxTurso, MAX_BATCH_SIZE, RecordInput};
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

const USAGE: &str =
    "usage: calyx relational <init|create-dataset|ingest|project|status|readback|verify> ...";

pub(crate) fn run(topic: &str, args: &[String]) -> CliResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| CliError::runtime(format!("create Turso runtime: {source}")))?;
    runtime.block_on(run_async(topic, args))
}

async fn run_async(topic: &str, args: &[String]) -> CliResult {
    match (topic, args) {
        ("init", [database]) => {
            let (_, report) = CalyxTurso::initialize(database).await?;
            print_json(&report)
        }
        ("create-dataset", [database, dataset, rest @ ..]) => {
            let panel_version = parse_single_u32_option(rest, "--panel-version", 1)?;
            let mut store = CalyxTurso::open(database).await?;
            let report = store.create_dataset(dataset, panel_version).await?;
            print_json(&report)
        }
        ("ingest", [database, dataset, input, rest @ ..]) => {
            let batch_size = parse_batch_size(rest)?;
            ingest(database, dataset, input, batch_size).await
        }
        ("project", [database, dataset, vault, rest @ ..]) => {
            let batch_size = parse_batch_size(rest)?;
            let mut store = CalyxTurso::open(database).await?;
            let report = store.project(dataset, vault, batch_size).await?;
            print_json(&report)
        }
        ("status", [database, dataset]) => {
            let store = CalyxTurso::open(database).await?;
            print_json(&store.status(dataset).await?)
        }
        ("readback", [database, dataset, record_id, rest @ ..]) => {
            let vault = parse_optional_path(rest, "--vault")?;
            let store = CalyxTurso::open(database).await?;
            let report = store.readback(dataset, record_id, vault.as_deref()).await?;
            print_json(&report)
        }
        ("verify", [database, dataset, vault]) => {
            let store = CalyxTurso::open(database).await?;
            print_json(&store.verify(dataset, vault).await?)
        }
        _ => Err(CliError::usage(USAGE)),
    }
}

async fn ingest(database: &str, dataset: &str, input: &str, batch_size: usize) -> CliResult {
    let mut store = CalyxTurso::open(database).await?;
    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(BufReader::new(io::stdin()))
    } else {
        Box::new(BufReader::new(File::open(input).map_err(|source| {
            CliError::io(format!("open input {input:?}: {source}"))
        })?))
    };
    let mut batch = Vec::with_capacity(batch_size);
    let mut totals = IngestTotals::new(dataset);
    for (index, line) in reader.lines().enumerate() {
        let line = line
            .map_err(|source| CliError::io(format!("read input line {}: {source}", index + 1)))?;
        if line.trim().is_empty() {
            return Err(invalid_record(format!(
                "input line {} is empty; every line must contain one JSON record",
                index + 1
            ))
            .into());
        }
        let record = serde_json::from_str::<RecordInput>(&line).map_err(|source| {
            invalid_record(format!("parse input line {} as JSON: {source}", index + 1))
        })?;
        batch.push(record);
        if batch.len() == batch_size {
            let report = store.ingest(dataset, std::mem::take(&mut batch)).await?;
            totals.add(report);
            batch = Vec::with_capacity(batch_size);
        }
    }
    if !batch.is_empty() {
        let report = store.ingest(dataset, batch).await?;
        totals.add(report);
    }
    if totals.submitted == 0 {
        return Err(invalid_record("input contains zero records").into());
    }
    if totals.inserted != 0 || totals.updated != 0 {
        totals.checkpointed_frames = store.checkpoint().await?;
        totals.checkpoints = 1;
    }
    print_json(&totals)
}

#[derive(Serialize)]
struct IngestTotals {
    dataset_id: String,
    submitted: usize,
    inserted: usize,
    updated: usize,
    unchanged: usize,
    batches: usize,
    checkpoints: usize,
    checkpointed_frames: i64,
}

impl IngestTotals {
    fn new(dataset_id: &str) -> Self {
        Self {
            dataset_id: dataset_id.to_string(),
            submitted: 0,
            inserted: 0,
            updated: 0,
            unchanged: 0,
            batches: 0,
            checkpoints: 0,
            checkpointed_frames: 0,
        }
    }

    fn add(&mut self, report: calyx_turso::IngestReport) {
        self.submitted += report.submitted;
        self.inserted += report.inserted;
        self.updated += report.updated;
        self.unchanged += report.unchanged;
        self.batches += 1;
        self.checkpointed_frames += report.checkpointed_frames;
    }
}

fn parse_batch_size(args: &[String]) -> CliResult<usize> {
    let value = parse_single_u32_option(args, "--batch-size", MAX_BATCH_SIZE as u32)?;
    let value = value as usize;
    if (1..=MAX_BATCH_SIZE).contains(&value) {
        Ok(value)
    } else {
        Err(CliError::usage(format!(
            "--batch-size must be within 1..={MAX_BATCH_SIZE}"
        )))
    }
}

fn parse_single_u32_option(args: &[String], name: &str, default: u32) -> CliResult<u32> {
    match args {
        [] => Ok(default),
        [flag, value] if flag == name => value
            .parse::<u32>()
            .map_err(|source| CliError::usage(format!("{name} must be a u32: {source}"))),
        _ => Err(CliError::usage(USAGE)),
    }
}

fn parse_optional_path(args: &[String], name: &str) -> CliResult<Option<std::path::PathBuf>> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == name => Ok(Some(Path::new(value).to_path_buf())),
        _ => Err(CliError::usage(USAGE)),
    }
}

fn invalid_record(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TURSO_RECORD_INVALID,
        message: message.into(),
        remediation: "correct the named JSONL record; invalid input is never partially committed",
    }
}
