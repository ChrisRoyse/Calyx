use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use calyx_poly::{
    FILE_SIZE_LINT_PASSED, FileSizeLintReport, FileSizeLintRequest, evaluate_file_size_lint,
    read_file_size_lint_report, write_file_size_lint_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const OVER_LIMIT: &str = "POLY_FILE_SIZE_LINT_OVER_LIMIT";
const ROOT_MISSING: &str = "POLY_FILE_SIZE_LINT_ROOT_MISSING";

#[test]
fn issue19_file_size_lint_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE19_FSV_ROOT", "poly-issue19-file-size");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy-current-tree",
        EdgeInputClass::HappyPath,
        FILE_SIZE_LINT_PASSED,
        Scenario::CurrentTree,
    );
    let exact = run_case(
        &root,
        "edge-exact-limit",
        EdgeInputClass::MaxLimit,
        FILE_SIZE_LINT_PASSED,
        Scenario::ExactLimit,
    );
    let over = run_case(
        &root,
        "edge-over-limit",
        EdgeInputClass::InvalidInput,
        OVER_LIMIT,
        Scenario::OverLimit,
    );
    let missing = run_case(
        &root,
        "edge-missing-root",
        EdgeInputClass::EmptyInput,
        ROOT_MISSING,
        Scenario::MissingRoot,
    );

    for outcome in [&happy, &exact, &over, &missing] {
        assert!(
            outcome.ok,
            "{} expected {} got {}",
            outcome.name, outcome.expected_code, outcome.observed_code
        );
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 19,
        "source_of_truth": [
            "physical Rust source files under configured roots",
            "file-size-lint-report.json files read back from disk",
            "per-case before.json, decision.json, after.json, and edge-case-outcome.json"
        ],
        "happy_path": happy,
        "edge_cases": {
            "exact_500_lines": exact,
            "over_501_lines": over,
            "missing_root": missing
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue19_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue19_fsv_root={}", root.display());
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    CurrentTree,
    ExactLimit,
    OverLimit,
    MissingRoot,
}

struct Fixture {
    request: FileSizeLintRequest,
    report_path: PathBuf,
    known_paths: Vec<PathBuf>,
}

fn run_case(
    root: &Path,
    name: &str,
    input_class: EdgeInputClass,
    expected_code: &str,
    scenario: Scenario,
) -> calyx_poly::edge_audit::EdgeCaseOutcome {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let fixture = prepare_fixture(&case_dir, scenario);

    drive_edge_case(
        EdgeCaseSpec {
            case_dir: &case_dir,
            name,
            input_class,
            expected_code,
            expect_state_change: true,
        },
        EdgeCaseDriver {
            read_before: || state(&fixture),
            execute: || {
                let report = evaluate_file_size_lint(&fixture.request);
                write_file_size_lint_report(&fixture.report_path, &report)
                    .expect("write file-size lint report");
                let readback =
                    read_file_size_lint_report(&fixture.report_path).expect("read report");
                assert_eq!(readback, report);
                report
            },
            read_after: || state(&fixture),
            decision_record,
        },
    )
    .expect("drive edge case")
}

fn prepare_fixture(case_dir: &Path, scenario: Scenario) -> Fixture {
    let report_path = case_dir.join("file-size-lint-report.json");
    match scenario {
        Scenario::CurrentTree => {
            let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            Fixture {
                request: FileSizeLintRequest::calyx_poly_crate(&crate_root),
                report_path,
                known_paths: vec![crate_root.join("src/lib.rs"), crate_root.join("tests")],
            }
        }
        Scenario::ExactLimit => fixture_with_file(case_dir, report_path, "exact.rs", 500),
        Scenario::OverLimit => fixture_with_file(case_dir, report_path, "over.rs", 501),
        Scenario::MissingRoot => Fixture {
            request: FileSizeLintRequest {
                roots: vec![case_dir.join("missing-root")],
                line_limit: 500,
            },
            report_path,
            known_paths: vec![case_dir.join("missing-root")],
        },
    }
}

fn fixture_with_file(
    case_dir: &Path,
    report_path: PathBuf,
    file_name: &str,
    lines: usize,
) -> Fixture {
    let root = case_dir.join("fixture-root");
    fs::create_dir_all(&root).expect("create fixture root");
    let file = root.join(file_name);
    write_rs_lines(&file, lines);
    fs::write(root.join("not-rust.txt"), "not counted\n").expect("write ignored fixture");
    Fixture {
        request: FileSizeLintRequest {
            roots: vec![root],
            line_limit: 500,
        },
        report_path,
        known_paths: vec![file],
    }
}

fn write_rs_lines(path: &Path, lines: usize) {
    let mut contents = String::new();
    for line in 1..=lines {
        contents.push_str(&format!("// known truth line {line}\n"));
    }
    fs::write(path, contents).expect("write known-truth Rust fixture");
}

fn decision_record(report: FileSizeLintReport) -> (String, Value) {
    (
        report.status_code.clone(),
        json!({
            "code": report.status_code,
            "report": report_summary(&report)
        }),
    )
}

fn state(fixture: &Fixture) -> Value {
    let report = fs::read(&fixture.report_path).ok();
    json!({
        "request": {
            "roots": fixture.request.roots.iter().map(|root| root.display().to_string()).collect::<Vec<_>>(),
            "line_limit": fixture.request.line_limit
        },
        "roots": fixture.request.roots.iter().map(|root| path_state(root)).collect::<Vec<_>>(),
        "known_paths": fixture.known_paths.iter().map(|path| path_hash_state(path)).collect::<Vec<_>>(),
        "report": {
            "path": fixture.report_path.display().to_string(),
            "exists": fixture.report_path.exists(),
            "bytes": report.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": report.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "readback": report.as_ref().and_then(|bytes| {
                serde_json::from_slice::<FileSizeLintReport>(bytes)
                    .ok()
                    .map(|report| report_summary(&report))
            })
        }
    })
}

fn path_state(path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "is_dir": path.is_dir(),
        "is_file": path.is_file()
    })
}

fn path_hash_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
    })
}

fn report_summary(report: &FileSizeLintReport) -> Value {
    json!({
        "schema_version": report.schema_version,
        "passed": report.passed,
        "status_code": report.status_code,
        "line_limit": report.line_limit,
        "checked_file_count": report.checked_file_count,
        "violation_count": report.violation_count,
        "max_line_count": report.max_line_count,
        "failure": report.failure,
        "files": report.files.iter().map(|file| {
            json!({
                "relative_path": file.relative_path,
                "lines": file.lines,
                "bytes": file.bytes,
                "within_limit": file.within_limit,
                "blake3": file.blake3
            })
        }).collect::<Vec<_>>()
    })
}
