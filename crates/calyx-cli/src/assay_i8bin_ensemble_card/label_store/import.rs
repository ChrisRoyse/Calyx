use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{CliError, CliResult};

use super::{ImportedLabels, args::ImportArgs, error, hex_sha256, write};

pub(crate) fn run_import(raw: &[String]) -> CliResult {
    let args = ImportArgs::parse(raw)?;
    let anchor = AnchorSpec::parse(&args.derive_anchor)?;
    let imported = load_rows_jsonl(&args.rows_jsonl, args.target_class, &anchor)?;
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
    gdelt_action_geo_country: Option<String>,
    #[serde(default)]
    gdelt_actor1_country: Option<String>,
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
    GdeltActor1Present,
    GdeltActor2Present,
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
        if raw == "gdelt_actor1_present" {
            return Ok(Self::GdeltActor1Present);
        }
        if raw == "gdelt_actor2_present" {
            return Ok(Self::GdeltActor2Present);
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
            Self::GdeltActor1Present => "gdelt_actor1_present".to_string(),
            Self::GdeltActor2Present => "gdelt_actor2_present".to_string(),
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
                Ok(text_token(row, "root")?.eq_ignore_ascii_case(expected))
            }
            Self::GdeltQuad(expected) => {
                Ok(text_token(row, "quad")?.eq_ignore_ascii_case(expected))
            }
            Self::GdeltActionGeo(expected) => Ok(row
                .gdelt_action_geo_country
                .as_deref()
                .unwrap_or_default()
                .eq_ignore_ascii_case(expected)),
            Self::GdeltActor1Present => {
                Ok(!row.gdelt_actor1_country.as_deref().unwrap_or("").is_empty())
            }
            Self::GdeltActor2Present => {
                Ok(!row.gdelt_actor2_country.as_deref().unwrap_or("").is_empty())
            }
            Self::SourceDomainContains(expected) => Ok(row
                .source_url
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())),
        }
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
