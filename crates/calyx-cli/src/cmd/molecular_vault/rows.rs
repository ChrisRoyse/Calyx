use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use calyx_core::Modality;
use serde::Deserialize;

use crate::error::{CliError, CliResult};

#[derive(Debug, Deserialize)]
struct MolecularRow {
    id: String,
    domain: String,
    modality: Modality,
    text: String,
    #[serde(default)]
    input: Option<String>,
    bridge_terms: Vec<String>,
    #[serde(default)]
    binding_affinity_nm: Option<f64>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(super) struct PreparedRow {
    pub(super) id: String,
    pub(super) domain: String,
    pub(super) modality: Modality,
    pub(super) input: String,
    pub(super) text: String,
    pub(super) bridge_terms: Vec<String>,
    pub(super) binding_affinity_nm: Option<f64>,
    pub(super) metadata: BTreeMap<String, String>,
}

pub(super) fn read_rows(path: &Path) -> CliResult<Vec<PreparedRow>> {
    let file = fs::File::open(path)
        .map_err(|error| CliError::io(format!("open rows {}: {error}", path.display())))?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|error| CliError::io(format!("read row {}: {error}", index + 1)))?;
        if line.trim().is_empty() {
            continue;
        }
        let row: MolecularRow = serde_json::from_str(&line).map_err(|error| {
            CliError::usage(format!(
                "molecular vault row {} invalid JSON: {error}",
                index + 1
            ))
        })?;
        let row = prepare_row(index + 1, row)?;
        if !ids.insert(row.id.clone()) {
            return Err(CliError::usage(format!(
                "molecular vault row {} duplicates id {}",
                index + 1,
                row.id
            )));
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(CliError::usage("molecular vault rows file is empty"));
    }
    Ok(rows)
}

fn prepare_row(line: usize, mut row: MolecularRow) -> CliResult<PreparedRow> {
    row.id = row.id.trim().to_string();
    row.domain = row.domain.trim().to_lowercase();
    row.text = row.text.trim().to_string();
    let input = row
        .input
        .take()
        .unwrap_or_else(|| row.text.clone())
        .trim()
        .to_string();
    if row.id.is_empty() || row.domain.is_empty() || row.text.is_empty() || input.is_empty() {
        return Err(CliError::usage(format!(
            "molecular vault row {line} requires id, domain, text, and input"
        )));
    }
    if !matches!(
        row.modality,
        Modality::Text | Modality::Protein | Modality::Dna | Modality::Molecule
    ) {
        return Err(CliError::usage(format!(
            "molecular vault row {line} modality must be text, protein, dna, or molecule"
        )));
    }
    for required in ["source_dataset", "source_path", "source_sha256"] {
        if row
            .metadata
            .get(required)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(CliError::usage(format!(
                "molecular vault row {line} metadata requires {required}"
            )));
        }
    }
    if let Some(value) = row.binding_affinity_nm
        && (!value.is_finite() || value <= 0.0)
    {
        return Err(CliError::usage(format!(
            "molecular vault row {line} binding_affinity_nm must be finite and > 0"
        )));
    }
    let text = row.text.to_lowercase();
    let mut terms = BTreeSet::new();
    for term in &row.bridge_terms {
        let normalized = normalize_term(term);
        if normalized.is_empty() {
            return Err(CliError::usage(format!(
                "molecular vault row {line} has empty bridge term"
            )));
        }
        if !text.contains(&normalized) {
            return Err(CliError::usage(format!(
                "molecular vault row {line} text does not contain bridge term {normalized}"
            )));
        }
        terms.insert(normalized);
    }
    if terms.is_empty() {
        return Err(CliError::usage(format!(
            "molecular vault row {line} requires at least one bridge term"
        )));
    }
    Ok(PreparedRow {
        id: row.id,
        domain: row.domain,
        modality: row.modality,
        input,
        text: row.text,
        bridge_terms: terms.into_iter().collect(),
        binding_affinity_nm: row.binding_affinity_nm,
        metadata: row.metadata,
    })
}

pub(super) fn validate_row_set(rows: &[PreparedRow]) -> CliResult {
    for required in [
        Modality::Text,
        Modality::Protein,
        Modality::Dna,
        Modality::Molecule,
    ] {
        if !rows.iter().any(|row| row.modality == required) {
            return Err(CliError::usage(format!(
                "molecular vault rows require at least one {} row",
                modality_name(required)
            )));
        }
    }
    if !rows.iter().any(|row| row.binding_affinity_nm.is_some()) {
        return Err(CliError::usage(
            "molecular vault rows require at least one binding_affinity_nm anchor",
        ));
    }
    let mut term_domains: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in rows {
        for term in &row.bridge_terms {
            term_domains
                .entry(term)
                .or_default()
                .insert(row.domain.as_str());
        }
    }
    let bridged = term_domains
        .values()
        .any(|domains| domains.contains("clinical") && domains.len() > 1);
    if !bridged {
        return Err(CliError::usage(
            "molecular vault rows require a bridge term shared by clinical and molecular rows",
        ));
    }
    Ok(())
}

pub(super) fn modality_name(modality: Modality) -> &'static str {
    match modality {
        Modality::Text => "text",
        Modality::Protein => "protein",
        Modality::Dna => "dna",
        Modality::Molecule => "molecule",
        Modality::Code => "code",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Structured => "structured",
        Modality::Mixed => "mixed",
    }
}

fn normalize_term(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
