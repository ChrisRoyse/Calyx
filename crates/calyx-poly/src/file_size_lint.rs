//! Local file-size lint gate for Poly source hygiene.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PolyError, Result};

pub const FILE_SIZE_LINT_SCHEMA_VERSION: &str = "poly.file_size_lint.v1";
pub const FILE_SIZE_LINT_PASSED: &str = "CALYX_POLY_FILE_SIZE_LINT_PASSED";
pub const DEFAULT_FILE_SIZE_LINE_LIMIT: usize = 500;

const ERR_EMPTY_ROOTS: &str = "POLY_FILE_SIZE_LINT_EMPTY_ROOTS";
const ERR_INVALID_LIMIT: &str = "POLY_FILE_SIZE_LINT_INVALID_LIMIT";
const ERR_ROOT_MISSING: &str = "POLY_FILE_SIZE_LINT_ROOT_MISSING";
const ERR_ROOT_NOT_DIRECTORY: &str = "POLY_FILE_SIZE_LINT_ROOT_NOT_DIRECTORY";
const ERR_READ_DIR: &str = "POLY_FILE_SIZE_LINT_READ_DIR_FAILED";
const ERR_READ_FILE: &str = "POLY_FILE_SIZE_LINT_READ_FILE_FAILED";
const ERR_SYMLINK: &str = "POLY_FILE_SIZE_LINT_SYMLINK_UNSUPPORTED";
const ERR_OVER_LIMIT: &str = "POLY_FILE_SIZE_LINT_OVER_LIMIT";

/// File-size lint request. Roots are scanned recursively for Rust source files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSizeLintRequest {
    pub roots: Vec<PathBuf>,
    pub line_limit: usize,
}

impl FileSizeLintRequest {
    /// Builds the default local `calyx-poly` source/test gate.
    pub fn calyx_poly_crate(crate_root: impl AsRef<Path>) -> Self {
        let crate_root = crate_root.as_ref();
        Self {
            roots: vec![crate_root.join("src"), crate_root.join("tests")],
            line_limit: DEFAULT_FILE_SIZE_LINE_LIMIT,
        }
    }
}

/// Root state recorded before scanning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSizeLintRootState {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
}

/// One checked Rust source file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSizeLintRecord {
    pub root: String,
    pub path: String,
    pub relative_path: String,
    pub bytes: u64,
    pub lines: usize,
    pub line_limit: usize,
    pub blake3: String,
    pub within_limit: bool,
}

/// Structured failure metadata for a failed lint run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSizeLintFailure {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}

/// Durable file-size lint report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSizeLintReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub line_limit: usize,
    pub roots: Vec<FileSizeLintRootState>,
    pub checked_file_count: usize,
    pub violation_count: usize,
    pub max_line_count: usize,
    pub files: Vec<FileSizeLintRecord>,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<FileSizeLintFailure>,
}

/// Evaluates the lint gate and returns a report for both success and failure states.
pub fn evaluate_file_size_lint(request: &FileSizeLintRequest) -> FileSizeLintReport {
    let mut report = empty_report(request.line_limit);
    if request.line_limit == 0 {
        return fail(
            report,
            ERR_INVALID_LIMIT,
            "line limit must be greater than zero",
            None,
        );
    }
    if request.roots.is_empty() {
        return fail(
            report,
            ERR_EMPTY_ROOTS,
            "at least one root is required",
            None,
        );
    }

    let mut files = Vec::new();
    for root in &request.roots {
        let state = root_state(root);
        report.roots.push(state.clone());
        if !state.exists {
            return fail(
                report,
                ERR_ROOT_MISSING,
                "configured lint root does not exist",
                Some(root),
            );
        }
        if !state.is_dir {
            return fail(
                report,
                ERR_ROOT_NOT_DIRECTORY,
                "configured lint root is not a directory",
                Some(root),
            );
        }
        if let Err(failure) = collect_rust_files(root, root, request.line_limit, &mut files) {
            report.failure = Some(failure.clone());
            report.status_code = failure.code;
            return report;
        }
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    report.max_line_count = files.iter().map(|file| file.lines).max().unwrap_or(0);
    report.violation_count = files.iter().filter(|file| !file.within_limit).count();
    report.checked_file_count = files.len();
    report.files = files;

    if report.violation_count == 0 {
        report.passed = true;
        report.status_code = FILE_SIZE_LINT_PASSED.to_string();
        report
    } else {
        fail(
            report,
            ERR_OVER_LIMIT,
            "one or more Rust source files exceed the configured line limit",
            None,
        )
    }
}

/// Returns `Ok` only when the report proves the file-size gate passed.
pub fn require_file_size_lint_passed(report: &FileSizeLintReport) -> Result<()> {
    if report.passed {
        Ok(())
    } else {
        let message = report
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "file-size lint report did not pass".to_string());
        Err(PolyError::file_size_lint(
            report.status_code.clone(),
            message,
        ))
    }
}

/// Writes a durable JSON lint report.
pub fn write_file_size_lint_report(path: &Path, report: &FileSizeLintReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::file_size_lint(
                "POLY_FILE_SIZE_LINT_REPORT_WRITE",
                format!("create report directory {}: {err}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        PolyError::file_size_lint(
            "POLY_FILE_SIZE_LINT_REPORT_ENCODE",
            format!("encode file-size lint report: {err}"),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        PolyError::file_size_lint(
            "POLY_FILE_SIZE_LINT_REPORT_WRITE",
            format!("write report {}: {err}", path.display()),
        )
    })
}

/// Reads a durable JSON lint report.
pub fn read_file_size_lint_report(path: &Path) -> Result<FileSizeLintReport> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::file_size_lint(
            "POLY_FILE_SIZE_LINT_REPORT_READ",
            format!("read report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::file_size_lint(
            "POLY_FILE_SIZE_LINT_REPORT_DECODE",
            format!("decode report {}: {err}", path.display()),
        )
    })
}

fn empty_report(line_limit: usize) -> FileSizeLintReport {
    FileSizeLintReport {
        schema_version: FILE_SIZE_LINT_SCHEMA_VERSION.to_string(),
        source_of_truth: "physical Rust source files under configured local calyx-poly roots"
            .to_string(),
        line_limit,
        roots: Vec::new(),
        checked_file_count: 0,
        violation_count: 0,
        max_line_count: 0,
        files: Vec::new(),
        passed: false,
        status_code: "POLY_FILE_SIZE_LINT_NOT_EVALUATED".to_string(),
        failure: None,
    }
}

fn fail(
    mut report: FileSizeLintReport,
    code: &'static str,
    message: impl Into<String>,
    path: Option<&Path>,
) -> FileSizeLintReport {
    report.passed = false;
    report.status_code = code.to_string();
    report.failure = Some(FileSizeLintFailure {
        code: code.to_string(),
        message: message.into(),
        path: path.map(|path| path.display().to_string()),
    });
    report
}

fn root_state(root: &Path) -> FileSizeLintRootState {
    FileSizeLintRootState {
        path: root.display().to_string(),
        exists: root.exists(),
        is_dir: root.is_dir(),
    }
}

fn collect_rust_files(
    root: &Path,
    dir: &Path,
    line_limit: usize,
    out: &mut Vec<FileSizeLintRecord>,
) -> std::result::Result<(), FileSizeLintFailure> {
    let mut entries = fs::read_dir(dir)
        .map_err(|err| {
            failure(
                ERR_READ_DIR,
                format!("read directory {}: {err}", dir.display()),
                dir,
            )
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| failure(ERR_READ_DIR, format!("read directory entry: {err}"), dir))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            failure(
                ERR_READ_DIR,
                format!("read file type {}: {err}", path.display()),
                &path,
            )
        })?;
        if file_type.is_symlink() {
            return Err(failure(
                ERR_SYMLINK,
                "symlinks are unsupported in file-size lint roots",
                &path,
            ));
        }
        if file_type.is_dir() {
            collect_rust_files(root, &path, line_limit, out)?;
        } else if file_type.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("rs")
        {
            out.push(read_record(root, &path, line_limit)?);
        }
    }
    Ok(())
}

fn read_record(
    root: &Path,
    path: &Path,
    line_limit: usize,
) -> std::result::Result<FileSizeLintRecord, FileSizeLintFailure> {
    let bytes = fs::read(path).map_err(|err| {
        failure(
            ERR_READ_FILE,
            format!("read Rust source file {}: {err}", path.display()),
            path,
        )
    })?;
    let lines = count_lines(&bytes);
    Ok(FileSizeLintRecord {
        root: root.display().to_string(),
        path: path.display().to_string(),
        relative_path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string(),
        bytes: bytes.len() as u64,
        lines,
        line_limit,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        within_limit: lines <= line_limit,
    })
}

fn failure(code: &'static str, message: impl Into<String>, path: &Path) -> FileSizeLintFailure {
    FileSizeLintFailure {
        code: code.to_string(),
        message: message.into(),
        path: Some(path.display().to_string()),
    }
}

fn count_lines(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        0
    } else {
        let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
        if bytes.ends_with(b"\n") {
            newlines
        } else {
            newlines + 1
        }
    }
}
