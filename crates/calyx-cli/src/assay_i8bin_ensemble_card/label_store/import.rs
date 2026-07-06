use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{CliError, CliResult};

use super::{ImportedLabels, args::ImportArgs, error, hex_sha256, write};

pub(crate) fn run_import(raw: &[String]) -> CliResult {
    let args = ImportArgs::parse(raw)?;
    let anchor = AnchorSpec::parse(&args.derive_anchor)?;
    let imported = load_rows_jsonl(
        &args.rows_jsonl,
        args.target_class,
        &anchor,
        args.limit_per_class,
    )?;
    let anchor_name = args
        .anchor_name
        .unwrap_or_else(|| anchor.default_name(args.target_class));
    let readback = write(
        &args.cf_root,
        &args.association_key,
        &anchor_name,
        args.target_class,
        &imported.source_sha256,
        &imported.label_counts,
        &imported.labels,
        args.chunk_rows,
    )
    .map_err(CliError::Calyx)?;
    println!(
        "i8bin_label_anchor_db cf_root={} association_key={} anchor={} target_class={} rows={} positives={} negatives={} chunks={} manifest_value_sha256={} chunk_value_sha256={} readback_matches={}",
        readback.cf_root,
        readback.association_key,
        anchor_name,
        args.target_class,
        readback.row_count,
        readback.positive_count,
        readback.negative_count,
        readback.chunk_count,
        readback.manifest_value_sha256,
        readback.chunk_value_sha256,
        readback.readback_matches
    );
    Ok(())
}

pub(crate) fn load_rows_jsonl(
    path: &Path,
    target_class: usize,
    anchor: &AnchorSpec,
    limit_per_class: Option<usize>,
) -> CliResult<ImportedLabels> {
    let bytes = std::fs::read(path).map_err(|err| {
        CliError::Calyx(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_IO",
            format!("read {} failed: {err}", path.display()),
        ))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|err| {
        CliError::Calyx(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
            format!("{} is not utf8: {err}", path.display()),
        ))
    })?;
    let mut labels = Vec::new();
    let mut counts = BTreeMap::new();
    let mut limiter = RowLimiter::new(limit_per_class);
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: RowJson = serde_json::from_str(line).map_err(|err| {
            CliError::Calyx(error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
                format!("{} line {line_idx}: {err}", path.display()),
            ))
        })?;
        if !limiter.accept(&row).map_err(|message| {
            CliError::Calyx(error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
                format!("{} line {line_idx}: {message}", path.display()),
            ))
        })? {
            continue;
        }
        let label = anchor.value(&row, target_class).map_err(|message| {
            CliError::Calyx(error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
                format!("{} line {line_idx}: {message}", path.display()),
            ))
        })?;
        *counts.entry(u8::from(label).to_string()).or_insert(0) += 1;
        labels.push(label);
    }
    super::validate_labels(&labels).map_err(CliError::Calyx)?;
    Ok(ImportedLabels {
        labels,
        label_counts: counts,
        source_sha256: hex_sha256(&bytes),
    })
}

#[derive(Deserialize)]
struct RowJson {
    #[serde(default)]
    label: Option<usize>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    gdelt_sql_date: Option<String>,
    #[serde(default)]
    gdelt_event_code: Option<String>,
    #[serde(default)]
    gdelt_event_root: Option<String>,
    #[serde(default)]
    gdelt_quad_class: Option<String>,
    #[serde(default)]
    gdelt_goldstein: Option<String>,
    #[serde(default)]
    gdelt_avg_tone: Option<String>,
    #[serde(default)]
    gdelt_actor1_name: Option<String>,
    #[serde(default)]
    gdelt_action_geo_country: Option<String>,
    #[serde(default)]
    gdelt_action_geo_fullname: Option<String>,
    #[serde(default)]
    gdelt_actor1_country: Option<String>,
    #[serde(default)]
    gdelt_actor2_name: Option<String>,
    #[serde(default)]
    gdelt_actor2_country: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum AnchorSpec {
    Label,
    GdeltRoot(String),
    GdeltQuad(String),
    GdeltActionGeo(String),
    GeoFullContains(String),
    GdeltActor1Country(String),
    GdeltActor2Country(String),
    GdeltActorCountry(String),
    GdeltActorPair(String, String),
    GdeltActorCountryPair(String, String),
    GdeltActor1Present,
    GdeltActor2Present,
    GdeltEventCode(String),
    GdeltEventRoot(String),
    GdeltSqlDatePrefix(String),
    GdeltSourceHost(String),
    GdeltSourceHostContains(String),
    GdeltSourceTld(String),
    GdeltGoldsteinSign(String),
    GdeltToneSign(String),
    GdeltGoldsteinBucket(i32),
    GdeltToneBucket(i32),
    SourceDomainContains(String),
}

impl AnchorSpec {
    fn parse(raw: &str) -> CliResult<Self> {
        if raw == "label" {
            return Ok(Self::Label);
        }
        if let Some(value) = raw.strip_prefix("gdelt_root:") {
            return Ok(Self::GdeltRoot(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_quad:") {
            return Ok(Self::GdeltQuad(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_action_geo:") {
            return Ok(Self::GdeltActionGeo(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_action_geo_fullname_contains:") {
            return Ok(Self::GeoFullContains(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_actor1_country:") {
            return Ok(Self::GdeltActor1Country(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_actor2_country:") {
            return Ok(Self::GdeltActor2Country(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_actor_country:") {
            return Ok(Self::GdeltActorCountry(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_actor_pair:") {
            let (left, right) = split_pair(value, "gdelt_actor_pair")?;
            return Ok(Self::GdeltActorPair(left, right));
        }
        if let Some(value) = raw.strip_prefix("gdelt_actor_country_pair:") {
            let (left, right) = split_pair(value, "gdelt_actor_country_pair")?;
            return Ok(Self::GdeltActorCountryPair(left, right));
        }
        if raw == "gdelt_actor1_present" {
            return Ok(Self::GdeltActor1Present);
        }
        if raw == "gdelt_actor2_present" {
            return Ok(Self::GdeltActor2Present);
        }
        if let Some(value) = raw.strip_prefix("gdelt_event_code:") {
            return Ok(Self::GdeltEventCode(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_event_root:") {
            return Ok(Self::GdeltEventRoot(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_sqldate_prefix:") {
            return Ok(Self::GdeltSqlDatePrefix(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_source_host:") {
            return Ok(Self::GdeltSourceHost(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_source_host_contains:") {
            return Ok(Self::GdeltSourceHostContains(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_source_tld:") {
            return Ok(Self::GdeltSourceTld(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_goldstein_sign:") {
            return Ok(Self::GdeltGoldsteinSign(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_tone_sign:") {
            return Ok(Self::GdeltToneSign(value.to_string()));
        }
        if let Some(value) = raw.strip_prefix("gdelt_goldstein_bucket:") {
            return Ok(Self::GdeltGoldsteinBucket(parse_bucket(
                value,
                "gdelt_goldstein_bucket",
            )?));
        }
        if let Some(value) = raw.strip_prefix("gdelt_tone_bucket:") {
            return Ok(Self::GdeltToneBucket(parse_bucket(
                value,
                "gdelt_tone_bucket",
            )?));
        }
        if let Some(value) = raw.strip_prefix("source_domain_contains:") {
            return Ok(Self::SourceDomainContains(value.to_string()));
        }
        Err(CliError::usage(format!("unknown --derive-anchor {raw}")))
    }

    fn default_name(&self, target_class: usize) -> String {
        match self {
            Self::Label => format!("target_class_{target_class}"),
            Self::GdeltRoot(value) => format!("gdelt_root_{value}"),
            Self::GdeltQuad(value) => format!("gdelt_quad_{value}"),
            Self::GdeltActionGeo(value) => format!("gdelt_action_geo_{value}"),
            Self::GeoFullContains(value) => format!("gdelt_action_geo_fullname_contains_{value}"),
            Self::GdeltActor1Country(value) => format!("gdelt_actor1_country_{value}"),
            Self::GdeltActor2Country(value) => format!("gdelt_actor2_country_{value}"),
            Self::GdeltActorCountry(value) => format!("gdelt_actor_country_{value}"),
            Self::GdeltActorPair(left, right) => format!("gdelt_actor_pair_{left}_{right}"),
            Self::GdeltActorCountryPair(left, right) => {
                format!("gdelt_actor_country_pair_{left}_{right}")
            }
            Self::GdeltActor1Present => "gdelt_actor1_present".to_string(),
            Self::GdeltActor2Present => "gdelt_actor2_present".to_string(),
            Self::GdeltEventCode(value) => format!("gdelt_event_code_{value}"),
            Self::GdeltEventRoot(value) => format!("gdelt_event_root_{value}"),
            Self::GdeltSqlDatePrefix(value) => format!("gdelt_sqldate_prefix_{value}"),
            Self::GdeltSourceHost(value) => format!("gdelt_source_host_{value}"),
            Self::GdeltSourceHostContains(value) => {
                format!("gdelt_source_host_contains_{value}")
            }
            Self::GdeltSourceTld(value) => format!("gdelt_source_tld_{value}"),
            Self::GdeltGoldsteinSign(value) => format!("gdelt_goldstein_sign_{value}"),
            Self::GdeltToneSign(value) => format!("gdelt_tone_sign_{value}"),
            Self::GdeltGoldsteinBucket(value) => format!("gdelt_goldstein_bucket_{value}"),
            Self::GdeltToneBucket(value) => format!("gdelt_tone_bucket_{value}"),
            Self::SourceDomainContains(value) => format!("source_domain_contains_{value}"),
        }
    }

    fn value(&self, row: &RowJson, target_class: usize) -> Result<bool, String> {
        match self {
            Self::Label => row
                .label
                .map(|label| label == target_class)
                .ok_or_else(|| "label anchor requires row.label".to_string()),
            Self::GdeltRoot(expected) => {
                Ok(field_or_text(row.gdelt_event_root.as_deref(), row, "root")?
                    .eq_ignore_ascii_case(expected))
            }
            Self::GdeltQuad(expected) => {
                Ok(field_or_text(row.gdelt_quad_class.as_deref(), row, "quad")?
                    .eq_ignore_ascii_case(expected))
            }
            Self::GdeltActionGeo(expected) => Ok(row
                .gdelt_action_geo_country
                .as_deref()
                .unwrap_or_default()
                .eq_ignore_ascii_case(expected)),
            Self::GeoFullContains(expected) => Ok(row
                .gdelt_action_geo_fullname
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())),
            Self::GdeltActor1Country(expected) => {
                Ok(eq_ascii(row.gdelt_actor1_country.as_deref(), expected))
            }
            Self::GdeltActor2Country(expected) => {
                Ok(eq_ascii(row.gdelt_actor2_country.as_deref(), expected))
            }
            Self::GdeltActorCountry(expected) => {
                Ok(eq_ascii(row.gdelt_actor1_country.as_deref(), expected)
                    || eq_ascii(row.gdelt_actor2_country.as_deref(), expected))
            }
            Self::GdeltActorPair(left, right) => {
                let actor1 = field_or_marker(row.gdelt_actor1_name.as_deref(), row, "Actor1");
                let actor2 = field_or_marker(row.gdelt_actor2_name.as_deref(), row, "Actor2");
                Ok(eq_ascii(actor1.as_deref(), left) && eq_ascii(actor2.as_deref(), right))
            }
            Self::GdeltActorCountryPair(left, right) => {
                Ok(eq_ascii(row.gdelt_actor1_country.as_deref(), left)
                    && eq_ascii(row.gdelt_actor2_country.as_deref(), right))
            }
            Self::GdeltActor1Present => {
                Ok(!row.gdelt_actor1_country.as_deref().unwrap_or("").is_empty())
            }
            Self::GdeltActor2Present => {
                Ok(!row.gdelt_actor2_country.as_deref().unwrap_or("").is_empty())
            }
            Self::GdeltEventCode(expected) => Ok(eq_ascii(
                field_or_marker(row.gdelt_event_code.as_deref(), row, "EventCode").as_deref(),
                expected,
            )),
            Self::GdeltEventRoot(expected) => Ok(eq_ascii(
                field_or_marker(row.gdelt_event_root.as_deref(), row, "root").as_deref(),
                expected,
            )),
            Self::GdeltSqlDatePrefix(expected) => {
                Ok(
                    field_or_marker(row.gdelt_sql_date.as_deref(), row, "SQLDATE")
                        .as_deref()
                        .is_some_and(|value| value.starts_with(expected)),
                )
            }
            Self::GdeltSourceHost(expected) => Ok(source_host(row)
                .as_deref()
                .is_some_and(|host| host.eq_ignore_ascii_case(expected))),
            Self::GdeltSourceHostContains(expected) => Ok(source_host(row)
                .as_deref()
                .is_some_and(|host| host.contains(&expected.to_ascii_lowercase()))),
            Self::GdeltSourceTld(expected) => Ok(source_host(row)
                .as_deref()
                .and_then(|host| host.rsplit('.').next().map(str::to_string))
                .is_some_and(|tld| tld.eq_ignore_ascii_case(expected))),
            Self::GdeltGoldsteinSign(expected) => Ok(sign(
                field_or_marker(row.gdelt_goldstein.as_deref(), row, "Goldstein").as_deref(),
            ) == Some(expected.as_str())),
            Self::GdeltToneSign(expected) => Ok(sign(
                field_or_marker(row.gdelt_avg_tone.as_deref(), row, "tone").as_deref(),
            ) == Some(expected.as_str())),
            Self::GdeltGoldsteinBucket(expected) => Ok(numeric_bucket(
                field_or_marker(row.gdelt_goldstein.as_deref(), row, "Goldstein").as_deref(),
                -10.0,
                10.0,
                2.0,
            ) == Some(*expected)),
            Self::GdeltToneBucket(expected) => Ok(numeric_bucket(
                field_or_marker(row.gdelt_avg_tone.as_deref(), row, "tone").as_deref(),
                -100.0,
                100.0,
                10.0,
            ) == Some(*expected)),
            Self::SourceDomainContains(expected) => Ok(row
                .source_url
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())),
        }
    }
}

fn split_pair(value: &str, label: &str) -> CliResult<(String, String)> {
    let (left, right) = value
        .split_once("->")
        .ok_or_else(|| CliError::usage(format!("{label} requires LEFT->RIGHT")))?;
    if left.trim().is_empty() || right.trim().is_empty() {
        return Err(CliError::usage(format!("{label} requires non-empty sides")));
    }
    Ok((left.trim().to_string(), right.trim().to_string()))
}

fn parse_bucket(value: &str, label: &str) -> CliResult<i32> {
    value
        .parse::<i32>()
        .map_err(|err| CliError::usage(format!("{label} must be an integer: {err}")))
}

fn eq_ascii(actual: Option<&str>, expected: &str) -> bool {
    actual
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case(expected)
}

fn source_host(row: &RowJson) -> Option<String> {
    let raw = row.source_url.as_deref()?;
    let without_scheme = raw.split_once("://").map_or(raw, |(_, tail)| tail);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?;
    let host = authority
        .split(':')
        .next()?
        .trim_matches('.')
        .to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (!host.is_empty()).then(|| host.to_string())
}

fn sign(raw: Option<&str>) -> Option<&'static str> {
    let value = raw?.trim().parse::<f32>().ok()?;
    Some(if value < 0.0 {
        "neg"
    } else if value > 0.0 {
        "pos"
    } else {
        "zero"
    })
}

fn numeric_bucket(raw: Option<&str>, min: f32, max: f32, width: f32) -> Option<i32> {
    raw?.trim()
        .parse::<f32>()
        .ok()
        .map(|value| ((value.clamp(min, max) - min) / width).floor() as i32)
}

struct RowLimiter {
    limit: Option<usize>,
    counts: BTreeMap<usize, usize>,
}

impl RowLimiter {
    fn new(limit: Option<usize>) -> Self {
        Self {
            limit,
            counts: BTreeMap::new(),
        }
    }

    fn accept(&mut self, row: &RowJson) -> Result<bool, String> {
        let Some(limit) = self.limit else {
            return Ok(true);
        };
        let label = row
            .label
            .ok_or_else(|| "--limit-per-class requires row.label".to_string())?;
        let count = self.counts.get(&label).copied().unwrap_or(0);
        if count >= limit {
            return Ok(false);
        }
        self.counts.insert(label, count + 1);
        Ok(true)
    }
}

fn text_token(row: &RowJson, marker: &str) -> Result<String, String> {
    let text = row
        .text
        .as_deref()
        .ok_or_else(|| format!("{marker} anchor requires row.text"))?;
    let mut tokens = text.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == marker {
            return tokens
                .next()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{marker} marker has no value"));
        }
    }
    Err(format!("{marker} marker not found in row.text"))
}

fn field_or_text(field: Option<&str>, row: &RowJson, marker: &str) -> Result<String, String> {
    field_or_marker(field, row, marker).ok_or_else(|| format!("{marker} marker not found in row"))
}

fn field_or_marker(field: Option<&str>, row: &RowJson, marker: &str) -> Option<String> {
    if let Some(value) = field
        && !value.trim().is_empty()
    {
        return Some(value.trim().to_string());
    }
    text_token(row, marker).ok()
}
