use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::args::Args;
use super::local_error;
use super::report::{Report, SourceFile};
use crate::assay_anchor_audit::AnchorAudit;
use crate::error::CliResult;

const GDELT_FIELD_COUNT: usize = 61;
const GLOBAL_EVENT_ID: usize = 0;
const SQL_DATE: usize = 1;
const ACTOR1_NAME: usize = 6;
const ACTOR1_COUNTRY: usize = 7;
const ACTOR2_NAME: usize = 16;
const ACTOR2_COUNTRY: usize = 17;
const EVENT_CODE: usize = 26;
const EVENT_ROOT: usize = 28;
const QUAD_CLASS: usize = 29;
const GOLDSTEIN: usize = 30;
const AVG_TONE: usize = 34;
const ACTION_GEO_FULL: usize = 52;
const ACTION_GEO_COUNTRY: usize = 53;
const DATE_ADDED: usize = 59;
const SOURCE_URL: usize = 60;

pub(super) fn run(args: &Args) -> CliResult<Report> {
    validate_paths(args)?;
    let files = source_files(&args.source_dir)?;
    let rows_tmp = with_tmp_extension(&args.out, "staging");
    let manifest_tmp = with_tmp_extension(&args.manifest, "staging");
    ensure_absent(&rows_tmp)?;
    ensure_absent(&manifest_tmp)?;
    let result = run_with_staging(args, &files, &rows_tmp, &manifest_tmp);
    if result.is_err() {
        let _ = fs::remove_file(&rows_tmp);
        let _ = fs::remove_file(&manifest_tmp);
    }
    result
}

fn run_with_staging(
    args: &Args,
    files: &[PathBuf],
    rows_tmp: &Path,
    manifest_tmp: &Path,
) -> CliResult<Report> {
    let mut writer = BufWriter::new(File::create(rows_tmp).map_err(io_error)?);
    let mut state = State::default();
    let mut sources = Vec::new();
    for file in files {
        let source = read_source_file(file)?;
        let rows_read = convert_source(args, &source.text, &mut writer, &mut state)?;
        sources.push(SourceFile {
            path: file.display().to_string(),
            sha256: source.sha256,
            bytes: source.bytes,
            rows_read,
        });
        if state.finished(args) {
            break;
        }
    }
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    validate_state(args, &state)?;
    let rows_hash = sha256_file(rows_tmp)?;
    let manifest = manifest_value(args, &state, &sources, &rows_hash);
    fs::write(
        manifest_tmp,
        serde_json::to_vec_pretty(&manifest).map_err(json_error)?,
    )
    .map_err(io_error)?;
    let manifest_hash = sha256_file(manifest_tmp)?;
    fs::rename(rows_tmp, &args.out).map_err(io_error)?;
    fs::rename(manifest_tmp, &args.manifest).map_err(io_error)?;
    Ok(Report {
        format: "calyx-gdelt-rows-report-v1",
        dataset: args.dataset.clone(),
        rows_jsonl: args.out.display().to_string(),
        manifest: args.manifest.display().to_string(),
        rows: state.rows,
        label_counts: state.label_counts_json(),
        source_files: sources.len(),
        source_bytes: sources.iter().map(|source| source.bytes).sum(),
        rows_jsonl_sha256: rows_hash,
        manifest_sha256: manifest_hash,
        first_row: state.first_row,
        last_row: state.last_row,
    })
}

fn validate_paths(args: &Args) -> CliResult {
    if !args.source_dir.is_dir() {
        return Err(local_error(
            "CALYX_FSV_GDELT_SOURCE_DIR_MISSING",
            format!("{} is not a directory", args.source_dir.display()),
            "download real GDELT export.CSV.zip files before conversion",
        ));
    }
    ensure_absent(&args.out)?;
    ensure_absent(&args.manifest)?;
    Ok(())
}

fn ensure_absent(path: &Path) -> CliResult {
    if path.exists() {
        return Err(local_error(
            "CALYX_FSV_GDELT_OUTPUT_EXISTS",
            format!("{} already exists", path.display()),
            "choose a new output path or inspect and remove the stale artifact",
        ));
    }
    Ok(())
}

fn source_files(source_dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut files = fs::read_dir(source_dir)
        .map_err(io_error)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(io_error))
        .collect::<CliResult<Vec<_>>>()?;
    files.retain(|path| is_gdelt_source(path));
    files.sort();
    if files.is_empty() {
        return Err(local_error(
            "CALYX_FSV_GDELT_SOURCE_EMPTY",
            format!(
                "{} has no .export.CSV or .export.CSV.zip files",
                source_dir.display()
            ),
            "download real GDELT v2 event export files into --source-dir",
        ));
    }
    Ok(files)
}

fn is_gdelt_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    name.ends_with(".export.CSV") || name.ends_with(".export.CSV.zip")
}

struct SourceText {
    text: String,
    sha256: String,
    bytes: u64,
}

fn read_source_file(path: &Path) -> CliResult<SourceText> {
    let bytes = fs::read(path).map_err(io_error)?;
    let sha256 = sha256_bytes(&bytes);
    let text = if path.extension().and_then(OsStr::to_str) == Some("zip") {
        unzip_text(path)?
    } else {
        String::from_utf8(bytes).map_err(|error| {
            local_error(
                "CALYX_FSV_GDELT_SOURCE_UTF8",
                format!("{} is not UTF-8: {error}", path.display()),
                "use GDELT text export files, not binary artifacts",
            )
        })?
    };
    Ok(SourceText {
        text,
        sha256,
        bytes: fs::metadata(path).map_err(io_error)?.len(),
    })
}

fn unzip_text(path: &Path) -> CliResult<String> {
    let output = Command::new("unzip")
        .arg("-p")
        .arg(path)
        .output()
        .map_err(|error| {
            local_error(
                "CALYX_FSV_GDELT_UNZIP_UNAVAILABLE",
                format!("failed to launch unzip for {}: {error}", path.display()),
                "install Info-ZIP unzip or extract the GDELT CSV before conversion",
            )
        })?;
    if !output.status.success() {
        return Err(local_error(
            "CALYX_FSV_GDELT_UNZIP_FAILED",
            format!("unzip -p {} exited with {}", path.display(), output.status),
            "inspect the zip file hash and replace corrupt downloads",
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        local_error(
            "CALYX_FSV_GDELT_SOURCE_UTF8",
            format!("{} unzipped bytes are not UTF-8: {error}", path.display()),
            "use GDELT text export files, not binary artifacts",
        )
    })
}

#[derive(Default)]
struct State {
    rows: usize,
    label_counts: BTreeMap<usize, usize>,
    first_row: Option<Value>,
    last_row: Option<Value>,
}

impl State {
    fn finished(&self, args: &Args) -> bool {
        if let Some(max_rows) = args.max_rows {
            return self.rows >= max_rows;
        }
        args.limit_per_class.is_some_and(|limit| {
            self.label_counts.get(&0).copied().unwrap_or(0) >= limit
                && self.label_counts.get(&1).copied().unwrap_or(0) >= limit
        })
    }

    fn accept(&self, args: &Args, label: usize) -> bool {
        if self.limit_reached(args.limit_per_class, label)
            || self.limit_reached(args.max_rows, usize::MAX)
        {
            return false;
        }
        true
    }

    fn limit_reached(&self, limit: Option<usize>, label: usize) -> bool {
        match (limit, label) {
            (Some(limit), usize::MAX) => self.rows >= limit,
            (Some(limit), label) => self.label_counts.get(&label).copied().unwrap_or(0) >= limit,
            (None, _) => false,
        }
    }

    fn push(&mut self, label: usize, row: Value) {
        *self.label_counts.entry(label).or_insert(0) += 1;
        self.rows += 1;
        if self.first_row.is_none() {
            self.first_row = Some(row.clone());
        }
        self.last_row = Some(row);
    }

    fn label_counts_json(&self) -> BTreeMap<String, usize> {
        self.label_counts
            .iter()
            .map(|(label, count)| (label.to_string(), *count))
            .collect()
    }
}

fn convert_source(
    args: &Args,
    text: &str,
    writer: &mut BufWriter<File>,
    state: &mut State,
) -> CliResult<usize> {
    let mut rows_read = 0usize;
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rows_read += 1;
        let record = parse_record(line_idx, line)?;
        let label = label(args, &record);
        if !state.accept(args, label) {
            continue;
        }
        let row = row_json(&record, label)?;
        serde_json::to_writer(&mut *writer, &row).map_err(json_error)?;
        writer.write_all(b"\n").map_err(io_error)?;
        state.push(label, row);
        if state.finished(args) {
            break;
        }
    }
    Ok(rows_read)
}

fn parse_record(line_idx: usize, line: &str) -> CliResult<Vec<&str>> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != GDELT_FIELD_COUNT {
        return Err(local_error(
            "CALYX_FSV_GDELT_ROW_MALFORMED",
            format!(
                "line {line_idx} has {} fields; expected {GDELT_FIELD_COUNT}",
                fields.len()
            ),
            "inspect the GDELT export schema before using the source file",
        ));
    }
    Ok(fields)
}

fn label(args: &Args, fields: &[&str]) -> usize {
    let actor1 = fields[ACTOR1_COUNTRY].trim().to_ascii_uppercase();
    let actor2 = fields[ACTOR2_COUNTRY].trim().to_ascii_uppercase();
    let action_country = fields[ACTION_GEO_COUNTRY].trim().to_ascii_uppercase();
    let action_name = fields[ACTION_GEO_FULL].to_ascii_uppercase();
    usize::from(
        actor1 == args.actor_country
            || actor2 == args.actor_country
            || action_country == args.action_country
            || action_name.contains(&args.action_name_contains.to_ascii_uppercase()),
    )
}

fn row_json(fields: &[&str], label: usize) -> CliResult<Value> {
    let event_id = required(fields[GLOBAL_EVENT_ID], "GLOBALEVENTID")?;
    let date_added = fields[DATE_ADDED].trim();
    let event_time = gdelt_stamp_to_utc(fields[DATE_ADDED])?;
    let text = gdelt_text(fields);
    let anchor_audit = AnchorAudit::gdelt_country_text_leak();
    Ok(json!({
        "id": format!("gdelt-v2://{}/{}", &date_added[0..8], event_id),
        "split": "fsv",
        "text": text,
        "label": label,
        "anchor_leaks_into_input": anchor_audit.anchor_leaks_into_input,
        "anchor_audit": anchor_audit,
        "event_time": event_time,
        "source_url": fields[SOURCE_URL].trim(),
        "gdelt_dateadded": fields[DATE_ADDED].trim(),
        "gdelt_sql_date": fields[SQL_DATE].trim(),
        "gdelt_actor1_country": fields[ACTOR1_COUNTRY].trim(),
        "gdelt_actor2_country": fields[ACTOR2_COUNTRY].trim(),
        "gdelt_action_geo_country": fields[ACTION_GEO_COUNTRY].trim(),
        "gdelt_action_geo_fullname": fields[ACTION_GEO_FULL].trim(),
    }))
}

fn required<'a>(value: &'a str, field: &str) -> CliResult<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(local_error(
            "CALYX_FSV_GDELT_ROW_MALFORMED",
            format!("{field} is empty"),
            "inspect the GDELT source row before conversion",
        ));
    }
    Ok(trimmed)
}

fn gdelt_stamp_to_utc(raw: &str) -> CliResult<String> {
    let stamp = raw.trim();
    if stamp.len() != 14 || !stamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(local_error(
            "CALYX_FSV_GDELT_INVALID_DATEADDED",
            format!("DATEADDED {stamp:?} must be YYYYMMDDHHMMSS"),
            "use real GDELT v2 export rows with valid DATEADDED timestamps",
        ));
    }
    Ok(format!(
        "{}-{}-{}T{}:{}:{}Z",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[8..10],
        &stamp[10..12],
        &stamp[12..14]
    ))
}

fn gdelt_text(fields: &[&str]) -> String {
    [
        format!("GDELT event {}", fields[GLOBAL_EVENT_ID].trim()),
        format!("SQLDATE {}", fields[SQL_DATE].trim()),
        format!(
            "Actor1 {} country {}",
            empty_as_unknown(fields[ACTOR1_NAME]),
            empty_as_unknown(fields[ACTOR1_COUNTRY])
        ),
        format!(
            "Actor2 {} country {}",
            empty_as_unknown(fields[ACTOR2_NAME]),
            empty_as_unknown(fields[ACTOR2_COUNTRY])
        ),
        format!(
            "EventCode {} root {} quad {}",
            fields[EVENT_CODE].trim(),
            fields[EVENT_ROOT].trim(),
            fields[QUAD_CLASS].trim()
        ),
        format!(
            "Goldstein {} tone {}",
            fields[GOLDSTEIN].trim(),
            fields[AVG_TONE].trim()
        ),
        format!(
            "ActionGeo {} country {}",
            empty_as_unknown(fields[ACTION_GEO_FULL]),
            empty_as_unknown(fields[ACTION_GEO_COUNTRY])
        ),
        format!("SourceURL {}", fields[SOURCE_URL].trim()),
    ]
    .join(" | ")
}

fn empty_as_unknown(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() { "UNK" } else { trimmed }
}

fn validate_state(args: &Args, state: &State) -> CliResult {
    if state.rows == 0 {
        return Err(local_error(
            "CALYX_FSV_GDELT_NO_ROWS",
            "conversion produced zero rows",
            "inspect source files and label filters before running stream-fbin",
        ));
    }
    if let Some(limit) = args.limit_per_class {
        for label in [0usize, 1usize] {
            let count = state.label_counts.get(&label).copied().unwrap_or(0);
            if count < limit {
                return Err(local_error(
                    "CALYX_FSV_GDELT_CLASS_UNDERFLOW",
                    format!("label {label} count {count} < required {limit}"),
                    "expand the source date range or lower --limit-per-class",
                ));
            }
        }
    }
    Ok(())
}

fn manifest_value(args: &Args, state: &State, sources: &[SourceFile], rows_hash: &str) -> Value {
    let anchor_audit = AnchorAudit::gdelt_country_text_leak();
    json!({
        "format": "calyx-gdelt-rows-source-v1",
        "source": "GDELT 2.0 Event Database export.CSV",
        "dataset": args.dataset,
        "anchor_leaks_into_input": anchor_audit.anchor_leaks_into_input,
        "trivial_anchor": anchor_audit.trivial_anchor,
        "grounded_gate_eligible": anchor_audit.grounded_gate_eligible,
        "anchor_audit": anchor_audit,
        "label_definition": {
            "positive_label": 1,
            "actor_country": args.actor_country,
            "action_country": args.action_country,
            "action_name_contains": args.action_name_contains,
        },
        "target_class": args.target_class,
        "row_count": state.rows,
        "label_counts": state.label_counts_json(),
        "rows_jsonl": args.out,
        "rows_jsonl_sha256": rows_hash,
        "source_files": sources,
        "first_row": state.first_row,
        "last_row": state.last_row,
    })
}

fn with_tmp_extension(path: &Path, extension: &str) -> PathBuf {
    let mut tmp = path.to_path_buf();
    tmp.set_extension(extension);
    tmp
}

fn sha256_file(path: &Path) -> CliResult<String> {
    fs::read(path)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(io_error)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn io_error(error: std::io::Error) -> crate::error::CliError {
    crate::error::CliError::io(error.to_string())
}

fn json_error(error: serde_json::Error) -> crate::error::CliError {
    local_error(
        "CALYX_FSV_GDELT_JSON",
        error.to_string(),
        "inspect generated row fields for invalid JSON values",
    )
}
