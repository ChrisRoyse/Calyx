//! Region/geography vocabulary extraction for the region one-hot lens (issue #44).
//!
//! The extractor is deterministic and local: it scans event/tag text for a fixed geography alias
//! table, groups canonical regions by Poly domain, and persists the resulting vocab report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::domain::Domain;
use crate::error::{PolyError, Result};

pub const REGION_VOCAB_SCHEMA_VERSION: &str = "poly.region_vocab.v1";
pub const REGION_VOCAB_ARTIFACT_KIND: &str = "poly_region_vocab";
pub const REGION_VOCAB_FILE: &str = "region_vocab.json";

pub const ERR_REGION_VOCAB_INVALID_REQUEST: &str = "CALYX_POLY_REGION_VOCAB_INVALID_REQUEST";
pub const ERR_REGION_VOCAB_READBACK_MISMATCH: &str = "CALYX_POLY_REGION_VOCAB_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionTextRecord {
    pub domain: Domain,
    pub event_text: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionVocabRequest {
    pub records: Vec<RegionTextRecord>,
    pub min_count: usize,
    pub max_terms_per_domain: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionVocabEntry {
    pub domain: Domain,
    pub region: String,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionRejectedRecord {
    pub domain: Domain,
    pub event_text: String,
    pub tags: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionVocabReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub min_count: usize,
    pub max_terms_per_domain: usize,
    pub input_record_count: usize,
    pub matched_record_count: usize,
    pub domain_vocab: BTreeMap<String, Vec<String>>,
    pub entries: Vec<RegionVocabEntry>,
    pub rejected_records: Vec<RegionRejectedRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionVocabRun {
    pub report_path: PathBuf,
    pub report: RegionVocabReport,
}

pub fn run_region_vocab_report(
    request: &RegionVocabRequest,
    output_root: &Path,
) -> Result<RegionVocabRun> {
    let report = build_region_vocab_report(request)?;
    let report_path = write_region_vocab_report(output_root, &report)?;
    let readback = read_region_vocab_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_REGION_VOCAB_READBACK_MISMATCH,
            format!(
                "region vocab report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(RegionVocabRun {
        report_path,
        report: readback,
    })
}

pub fn build_region_vocab_report(request: &RegionVocabRequest) -> Result<RegionVocabReport> {
    validate_request(request)?;
    let mut counts = BTreeMap::<(String, String), (Domain, usize)>::new();
    let mut rejected_records = Vec::new();
    let mut matched_record_count = 0;

    for record in &request.records {
        let regions = infer_regions(record);
        if regions.is_empty() {
            rejected_records.push(RegionRejectedRecord {
                domain: record.domain,
                event_text: record.event_text.clone(),
                tags: record.tags.clone(),
                reason: "no known geography alias matched event/tag text".to_string(),
            });
            continue;
        }
        matched_record_count += 1;
        for region in regions {
            let key = (record.domain.slug().to_string(), region);
            counts.entry(key).or_insert((record.domain, 0)).1 += 1;
        }
    }

    let mut by_domain = BTreeMap::<String, (Domain, Vec<(String, usize)>)>::new();
    for ((domain_slug, region), (domain, count)) in counts {
        if count >= request.min_count {
            by_domain
                .entry(domain_slug)
                .or_insert_with(|| (domain, Vec::new()))
                .1
                .push((region, count));
        }
    }

    let mut domain_vocab = BTreeMap::new();
    let mut entries = Vec::new();
    for (domain_slug, (domain, mut rows)) in by_domain {
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows.truncate(request.max_terms_per_domain);
        let vocab = rows
            .iter()
            .map(|(region, _)| region.clone())
            .collect::<Vec<_>>();
        for (region, count) in rows {
            entries.push(RegionVocabEntry {
                domain,
                region,
                count,
            });
        }
        domain_vocab.insert(domain_slug, vocab);
    }
    entries.sort_by(|a, b| {
        a.domain
            .slug()
            .cmp(b.domain.slug())
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.region.cmp(&b.region))
    });

    Ok(RegionVocabReport {
        schema_version: REGION_VOCAB_SCHEMA_VERSION.to_string(),
        artifact_kind: REGION_VOCAB_ARTIFACT_KIND.to_string(),
        min_count: request.min_count,
        max_terms_per_domain: request.max_terms_per_domain,
        input_record_count: request.records.len(),
        matched_record_count,
        domain_vocab,
        entries,
        rejected_records,
    })
}

pub fn region_vocab_for_domain(report: &RegionVocabReport, domain: Domain) -> Vec<String> {
    report
        .domain_vocab
        .get(domain.slug())
        .cloned()
        .unwrap_or_default()
}

pub fn write_region_vocab_report(dir: &Path, report: &RegionVocabReport) -> Result<PathBuf> {
    write_json(dir, REGION_VOCAB_FILE, report)
}

pub fn read_region_vocab_report(path: &Path) -> Result<RegionVocabReport> {
    read_json(path)
}

fn validate_request(request: &RegionVocabRequest) -> Result<()> {
    if request.records.is_empty() {
        return invalid("region vocabulary requires at least one event/tag record");
    }
    if request.min_count == 0 || request.max_terms_per_domain == 0 {
        return invalid("min_count and max_terms_per_domain must both be positive");
    }
    for (idx, record) in request.records.iter().enumerate() {
        if record.event_text.trim().is_empty()
            && record.tags.iter().all(|tag| tag.trim().is_empty())
        {
            return invalid(format!(
                "record {idx} must contain event_text or at least one non-empty tag"
            ));
        }
    }
    Ok(())
}

fn infer_regions(record: &RegionTextRecord) -> Vec<String> {
    let text = normalize(&format!("{} {}", record.event_text, record.tags.join(" ")));
    let padded = format!(" {text} ");
    let mut regions = BTreeSet::new();
    for (region, aliases) in REGION_ALIASES {
        if aliases
            .iter()
            .any(|alias| padded.contains(&format!(" {} ", normalize(alias))))
        {
            regions.insert((*region).to_string());
        }
    }
    regions.into_iter().collect()
}

fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_space = true;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_REGION_VOCAB_INVALID_REQUEST,
        message.into(),
    ))
}

const REGION_ALIASES: &[(&str, &[&str])] = &[
    ("california", &["california", "ca"]),
    ("canada", &["canada", "canadian"]),
    ("china", &["china", "chinese"]),
    ("europe", &["europe", "european union", "eu"]),
    ("florida", &["florida", "fl"]),
    ("gaza", &["gaza"]),
    ("india", &["india", "indian"]),
    ("israel", &["israel", "israeli"]),
    ("london", &["london"]),
    ("mexico", &["mexico", "mexican"]),
    ("new_york", &["new york", "ny"]),
    ("new_york_city", &["new york city", "nyc"]),
    ("ohio", &["ohio", "oh"]),
    ("paris", &["paris"]),
    ("russia", &["russia", "russian"]),
    ("taiwan", &["taiwan", "taiwanese"]),
    ("texas", &["texas", "tx"]),
    ("uk", &["united kingdom", "uk", "britain", "england"]),
    ("ukraine", &["ukraine", "ukrainian"]),
    (
        "us",
        &["united states", "usa", "u s", "america", "american"],
    ),
    ("washington_dc", &["washington dc", "washington d c", "dc"]),
];
