use std::collections::BTreeMap;
use std::fs;

use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_docs_coverage_classify::classify_docs_rows;
use crate::raw_source_support::{file_state, now_unix_ms, sha256_hex, write_json};
use crate::raw_sources::{
    RawEndpointSample, RawFileState, RawSourceCoverage, RawSourceFailure, RawSourceSamplingRequest,
};
use crate::{PolyError, Result};

pub const DOCS_INDEX_URL: &str = "https://docs.polymarket.com/llms.txt";

const DOCS_COVERAGE_SCHEMA_VERSION: &str = "poly.docs_index_coverage.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDocsIndexSnapshot {
    pub url: String,
    pub status_code: u16,
    pub body_path: String,
    pub metadata_path: String,
    pub body_bytes: u64,
    pub body_sha256: String,
    pub before: RawFileState,
    pub after: RawFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDocsCoverageRow {
    pub title: String,
    pub url: String,
    pub normalized_url: String,
    pub source_family: String,
    pub classification: String,
    pub policy_status: String,
    pub justification: String,
    pub sample_names: Vec<String>,
    pub artifact_paths: Vec<String>,
    pub related_issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawDocsIndexCoverage {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub docs_index: RawDocsIndexSnapshot,
    pub row_count: usize,
    pub classification_counts: BTreeMap<String, usize>,
    pub rows: Vec<RawDocsCoverageRow>,
    pub artifact_failures: Vec<String>,
    pub not_yet_sampled_count: usize,
    pub blocked_runtime_count: usize,
    pub forbidden_count: usize,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<RawSourceFailure>,
}

pub(crate) fn build_docs_index_coverage(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
    samples: &[RawEndpointSample],
    coverage: &RawSourceCoverage,
    clock: &dyn Clock,
) -> Result<RawDocsIndexCoverage> {
    let snapshot = capture_docs_index(request, agent, clock)?;
    let text = fs::read_to_string(&snapshot.body_path).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_READBACK_FAILED",
            format!("read docs index body {}: {err}", snapshot.body_path),
        )
    })?;
    let links = parse_markdown_links(&text);
    if links.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_EMPTY",
            format!(
                "docs index {} contained no Markdown links",
                snapshot.body_path
            ),
        ));
    }
    let (rows, artifact_failures) = classify_docs_rows(links, samples, coverage);
    let mut classification_counts = BTreeMap::new();
    for row in &rows {
        *classification_counts
            .entry(row.classification.clone())
            .or_insert(0) += 1;
    }
    let not_yet_sampled_count = classification_counts
        .get("not-yet-sampled")
        .copied()
        .unwrap_or(0);
    let blocked_runtime_count = classification_counts
        .get("blocked-runtime")
        .copied()
        .unwrap_or(0);
    let forbidden_count = classification_counts
        .iter()
        .filter(|(classification, _)| classification.starts_with("forbidden-"))
        .map(|(_, count)| *count)
        .sum();
    let failure = make_docs_coverage_failure(not_yet_sampled_count, &artifact_failures, &rows);
    let passed = failure.is_none();
    Ok(RawDocsIndexCoverage {
        schema_version: DOCS_COVERAGE_SCHEMA_VERSION.to_string(),
        captured_at_unix_ms: now_unix_ms(clock),
        docs_index: snapshot,
        row_count: rows.len(),
        classification_counts,
        rows,
        artifact_failures,
        not_yet_sampled_count,
        blocked_runtime_count,
        forbidden_count,
        status_code: if passed {
            "POLY_RAW_SOURCE_DOCS_COVERAGE_PASSED".to_string()
        } else {
            failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_else(|| "POLY_RAW_SOURCE_DOCS_COVERAGE_FAILED".to_string())
        },
        passed,
        failure,
    })
}

pub(crate) fn docs_coverage_failure(coverage: &RawDocsIndexCoverage) -> Option<RawSourceFailure> {
    coverage.failure.clone()
}

fn capture_docs_index(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
    clock: &dyn Clock,
) -> Result<RawDocsIndexSnapshot> {
    let sample_dir = request.output_root.join("raw").join("docs-index");
    let body_path = sample_dir.join("llms.txt");
    let metadata_path = sample_dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&sample_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_DIR_CREATE_FAILED",
            format!(
                "create docs-index sample dir {}: {err}",
                sample_dir.display()
            ),
        )
    })?;
    let max_body_bytes = u64::try_from(request.max_body_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_BODY_LIMIT_CONVERT_FAILED",
            format!(
                "convert max body bytes {} to u64: {err}",
                request.max_body_bytes
            ),
        )
    })?;
    let endpoint = RateLimitEndpoint::new("docs-index", "llms.txt", "GET");
    let (status_code, bytes) = execute_rate_limited_request(clock, &endpoint, || {
        let mut response = agent
            .get(DOCS_INDEX_URL)
            .header("Accept", "text/plain")
            .call()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_RAW_SOURCE_DOCS_INDEX_HTTP_FAILED",
                    format!("fetch docs index {DOCS_INDEX_URL}: {err}"),
                )
            })?;
        let status_code = response.status().as_u16();
        let retry_after_ms = parse_retry_after_ms(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        );
        let bytes = response
            .body_mut()
            .with_config()
            .limit(max_body_bytes)
            .read_to_vec()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_RAW_SOURCE_DOCS_INDEX_BODY_READ_FAILED",
                    format!("read docs index {DOCS_INDEX_URL}: {err}"),
                )
            })?;
        Ok(RateLimitedHttpOutcome {
            status_code: Some(status_code),
            retry_after_ms,
            value: (status_code, bytes),
        })
    })?;
    if !(200..300).contains(&status_code) {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_HTTP_STATUS",
            format!("docs index {DOCS_INDEX_URL} returned HTTP {status_code}"),
        ));
    }
    if bytes.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_BODY_EMPTY",
            format!("docs index {DOCS_INDEX_URL} returned an empty body"),
        ));
    }
    let body_sha256 = sha256_hex(&bytes);
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_WRITE_FAILED",
            format!("write docs index {}: {err}", body_path.display()),
        )
    })?;
    let readback = fs::read(&body_path).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_FILE_READBACK_FAILED",
            format!("read docs index after write {}: {err}", body_path.display()),
        )
    })?;
    let readback_sha256 = sha256_hex(&readback);
    if readback_sha256 != body_sha256 {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_DOCS_INDEX_READBACK_MISMATCH",
            format!(
                "docs index readback mismatch {}; expected_sha256={} actual_sha256={}",
                body_path.display(),
                body_sha256,
                readback_sha256
            ),
        ));
    }
    let after = file_state(&body_path, &metadata_path)?;
    let snapshot = RawDocsIndexSnapshot {
        url: DOCS_INDEX_URL.to_string(),
        status_code,
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_bytes: bytes.len() as u64,
        body_sha256,
        before,
        after,
    };
    write_json(&metadata_path, &snapshot)?;
    let after_metadata = file_state(&body_path, &metadata_path)?;
    Ok(RawDocsIndexSnapshot {
        after: after_metadata,
        ..snapshot
    })
}

fn parse_markdown_links(text: &str) -> Vec<(String, String)> {
    let chars = text.char_indices().collect::<Vec<_>>();
    let mut links = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].1 != '[' {
            index += 1;
            continue;
        }
        let Some(title_end) = find_char(&chars, index + 1, ']') else {
            break;
        };
        let url_open = title_end + 1;
        if url_open >= chars.len() || chars[url_open].1 != '(' {
            index += 1;
            continue;
        }
        let Some(url_end) = find_char(&chars, url_open + 1, ')') else {
            break;
        };
        let title_start_byte = chars[index].0 + '['.len_utf8();
        let title_end_byte = chars[title_end].0;
        let url_start_byte = chars[url_open].0 + '('.len_utf8();
        let url_end_byte = chars[url_end].0;
        let title = text[title_start_byte..title_end_byte].trim().to_string();
        let url = text[url_start_byte..url_end_byte].trim().to_string();
        if !title.is_empty() && url.starts_with("https://") {
            links.push((title, url));
        }
        index = url_end + 1;
    }
    links
}

fn find_char(chars: &[(usize, char)], start: usize, needle: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, (_, ch))| (*ch == needle).then_some(index))
}

fn make_docs_coverage_failure(
    not_yet_sampled_count: usize,
    artifact_failures: &[String],
    rows: &[RawDocsCoverageRow],
) -> Option<RawSourceFailure> {
    if !artifact_failures.is_empty() {
        return Some(RawSourceFailure {
            code: "POLY_RAW_SOURCE_DOCS_COVERAGE_ARTIFACT_MISSING".to_string(),
            message: format!(
                "{} docs coverage sample artifacts failed physical readback",
                artifact_failures.len()
            ),
            sample_name: None,
        });
    }
    if not_yet_sampled_count > 0 {
        return Some(RawSourceFailure {
            code: "POLY_RAW_SOURCE_DOCS_COVERAGE_INCOMPLETE".to_string(),
            message: format!(
                "{not_yet_sampled_count} docs-index rows are public/read-only but not yet sampled"
            ),
            sample_name: None,
        });
    }
    let unaccepted_blocked_runtime_count = rows
        .iter()
        .filter(|row| row.classification == "blocked-runtime")
        .filter(|row| !accepted_blocked_runtime_row(row))
        .count();
    if unaccepted_blocked_runtime_count > 0 {
        return Some(RawSourceFailure {
            code: "POLY_RAW_SOURCE_DOCS_COVERAGE_BLOCKED_RUNTIME".to_string(),
            message: format!(
                "{unaccepted_blocked_runtime_count} docs-index rows are blocked by live runtime"
            ),
            sample_name: None,
        });
    }
    None
}

fn accepted_blocked_runtime_row(row: &RawDocsCoverageRow) -> bool {
    row.source_family == "websocket-sports"
        && row.related_issues.iter().any(|issue| issue == "#187")
}

#[cfg(test)]
mod tests {
    use super::{RawDocsCoverageRow, make_docs_coverage_failure};

    #[test]
    fn sports_blocked_runtime_is_accepted_but_rtds_is_not() {
        let sports_rows = vec![blocked_row(
            "Sports WebSocket",
            "websocket-sports",
            vec!["#187".to_string()],
        )];
        assert!(make_docs_coverage_failure(0, &[], &sports_rows).is_none());

        let rtds_rows = vec![blocked_row(
            "Real-Time Data Socket",
            "websocket-rtds",
            vec!["#198".to_string()],
        )];
        let failure = make_docs_coverage_failure(0, &[], &rtds_rows).expect("RTDS fails");
        assert_eq!(
            failure.code,
            "POLY_RAW_SOURCE_DOCS_COVERAGE_BLOCKED_RUNTIME"
        );
    }

    fn blocked_row(
        title: &str,
        source_family: &str,
        related_issues: Vec<String>,
    ) -> RawDocsCoverageRow {
        RawDocsCoverageRow {
            title: title.to_string(),
            url: format!("https://docs.polymarket.com/{source_family}"),
            normalized_url: format!("https://docs.polymarket.com/{source_family}"),
            source_family: source_family.to_string(),
            classification: "blocked-runtime".to_string(),
            policy_status: "public-read-only-blocked".to_string(),
            justification: "unit-test blocked runtime".to_string(),
            sample_names: Vec::new(),
            artifact_paths: Vec::new(),
            related_issues,
        }
    }
}
