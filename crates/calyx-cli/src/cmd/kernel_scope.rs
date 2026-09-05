use std::collections::{BTreeMap, BTreeSet};

use super::kernel_generation::{
    KernelJurisdictionContract, KernelJurisdictionScope, jurisdiction_metadata_sha256,
};
use crate::error::{CliError, CliResult};

const LEGAL_DATASET_PREFIX: &str = "courtlistener-";
const GLOBAL_FIELDS: [(&str, &str); 3] = [
    ("jurisdiction_country", "country"),
    ("jurisdiction_court_system", "court_system"),
    ("jurisdiction_state", "state"),
];
const US_STATES: [&str; 51] = [
    "alabama",
    "alaska",
    "arizona",
    "arkansas",
    "california",
    "colorado",
    "connecticut",
    "delaware",
    "district of columbia",
    "florida",
    "georgia",
    "hawaii",
    "idaho",
    "illinois",
    "indiana",
    "iowa",
    "kansas",
    "kentucky",
    "louisiana",
    "maine",
    "maryland",
    "massachusetts",
    "michigan",
    "minnesota",
    "mississippi",
    "missouri",
    "montana",
    "nebraska",
    "nevada",
    "new hampshire",
    "new jersey",
    "new mexico",
    "new york",
    "north carolina",
    "north dakota",
    "ohio",
    "oklahoma",
    "oregon",
    "pennsylvania",
    "rhode island",
    "south carolina",
    "south dakota",
    "tennessee",
    "texas",
    "utah",
    "vermont",
    "virginia",
    "washington",
    "west virginia",
    "wisconsin",
    "wyoming",
];
const FEDERAL_SIGNALS: [&str; 6] = [
    "federal",
    "federal circuit",
    "united states court",
    "united states supreme court",
    "u.s. supreme court",
    "supreme court of the united states",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScopeConflict {
    pub kind: &'static str,
    pub detected: String,
}

pub(crate) fn derive_jurisdiction(
    rows: &[BTreeMap<String, String>],
) -> CliResult<Option<KernelJurisdictionContract>> {
    let legal_rows = rows
        .iter()
        .filter(|row| {
            row.get("source_dataset")
                .is_some_and(|value| value.starts_with(LEGAL_DATASET_PREFIX))
                || row.get("court_id").is_some_and(|value| !value.is_empty())
                || row
                    .get("source_url")
                    .is_some_and(|value| value.contains("courtlistener.com/"))
        })
        .count();
    if legal_rows == 0 {
        return Ok(None);
    }
    if legal_rows != rows.len() {
        return Err(CliError::runtime(format!(
            "kernel jurisdiction contract found mixed legal/non-legal rows: legal={legal_rows} total={}",
            rows.len()
        )));
    }
    if rows.iter().any(|row| {
        !row.get("source_dataset")
            .is_some_and(|value| value.starts_with(LEGAL_DATASET_PREFIX))
    }) {
        return Err(CliError::runtime(
            "kernel jurisdiction contract requires a CourtListener source_dataset on every legal row",
        ));
    }
    let mut global_values = BTreeMap::new();
    for (metadata_key, contract_key) in GLOBAL_FIELDS {
        let distinct = rows
            .iter()
            .map(|row| row.get(metadata_key).map(String::as_str).unwrap_or(""))
            .collect::<BTreeSet<_>>();
        if distinct.len() != 1
            || distinct.contains("")
            || distinct.iter().any(|value| value.trim() != *value)
        {
            return Err(CliError::runtime(format!(
                "kernel jurisdiction metadata {metadata_key} must be present and identical on every legal row; observed={distinct:?}"
            )));
        }
        global_values.insert(
            contract_key,
            distinct.into_iter().next().unwrap().to_string(),
        );
    }

    #[derive(Default)]
    struct ScopeAccumulator {
        court_ids: BTreeSet<String>,
        source_rows: usize,
    }

    type ScopeKey = (String, Option<String>, Option<String>);
    let mut scope_rows = BTreeMap::<ScopeKey, ScopeAccumulator>::new();
    for (index, row) in rows.iter().enumerate() {
        let source_dataset = required_nonempty(row, "source_dataset", index)?;
        if !source_dataset.starts_with(LEGAL_DATASET_PREFIX) {
            return Err(CliError::runtime(format!(
                "kernel jurisdiction metadata source_dataset must start with {LEGAL_DATASET_PREFIX:?} on every legal row; row={index} observed={source_dataset:?}"
            )));
        }
        let court_id = required_nonempty(row, "court_id", index)?;
        let county = optional_nonempty(row, "jurisdiction_county", index)?;
        let appellate_district = optional_nonempty(row, "jurisdiction_appellate_district", index)?;
        let entry = scope_rows
            .entry((source_dataset.to_string(), county, appellate_district))
            .or_default();
        entry.court_ids.insert(court_id.to_string());
        entry.source_rows += 1;
    }
    let scopes = scope_rows
        .into_iter()
        .map(
            |((source_dataset, county, appellate_district), accumulated)| KernelJurisdictionScope {
                source_dataset,
                court_ids: accumulated.court_ids.into_iter().collect(),
                county,
                appellate_district,
                source_rows: accumulated.source_rows,
            },
        )
        .collect::<Vec<_>>();

    let mut contract = KernelJurisdictionContract {
        schema_version: 2,
        country: global_values.remove("country").unwrap(),
        court_system: global_values.remove("court_system").unwrap(),
        state: global_values.remove("state").unwrap(),
        county: None,
        appellate_district: None,
        scopes,
        source_rows: rows.len(),
        metadata_contract_sha256: String::new(),
    };
    contract.metadata_contract_sha256 = jurisdiction_metadata_sha256(&contract)
        .ok_or_else(|| CliError::runtime("unsupported derived jurisdiction contract schema"))?;
    Ok(Some(contract))
}

fn required_nonempty<'a>(
    row: &'a BTreeMap<String, String>,
    key: &str,
    row_index: usize,
) -> CliResult<&'a str> {
    let value = row.get(key).map(String::as_str).unwrap_or("");
    if value.is_empty() || value.trim() != value {
        return Err(CliError::runtime(format!(
            "kernel jurisdiction metadata {key} must be present, non-empty, and canonical on every legal row; row={row_index} observed={value:?}"
        )));
    }
    Ok(value)
}

fn optional_nonempty(
    row: &BTreeMap<String, String>,
    key: &str,
    row_index: usize,
) -> CliResult<Option<String>> {
    let Some(value) = row.get(key) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.trim() != value {
        return Err(CliError::runtime(format!(
            "kernel jurisdiction metadata {key} must be absent, empty, or canonical; row={row_index} observed={value:?}"
        )));
    }
    Ok(Some(value.clone()))
}

pub(crate) fn allowed_counties(contract: &KernelJurisdictionContract) -> Vec<String> {
    if contract.schema_version == 1 {
        return contract.county.iter().cloned().collect();
    }
    contract
        .scopes
        .iter()
        .filter_map(|scope| scope.county.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn allowed_appellate_districts(contract: &KernelJurisdictionContract) -> Vec<String> {
    if contract.schema_version == 1 {
        return contract.appellate_district.iter().cloned().collect();
    }
    contract
        .scopes
        .iter()
        .filter_map(|scope| scope.appellate_district.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn includes_statewide_scope(contract: &KernelJurisdictionContract) -> bool {
    contract.schema_version == 2
        && contract
            .scopes
            .iter()
            .any(|scope| scope.county.is_none() && scope.appellate_district.is_none())
}

pub(crate) fn explicit_scope_conflict(
    query: &str,
    contract: &KernelJurisdictionContract,
) -> Option<ScopeConflict> {
    let normalized = query.to_ascii_lowercase();
    let expected_state = contract.state.to_ascii_lowercase();
    if let Some(state) = US_STATES
        .iter()
        .find(|state| phrase_present(&normalized, state) && **state != expected_state)
    {
        return Some(ScopeConflict {
            kind: "state",
            detected: title_case(state),
        });
    }
    if !contract.court_system.eq_ignore_ascii_case("federal")
        && let Some(signal) = FEDERAL_SIGNALS
            .iter()
            .find(|signal| phrase_present(&normalized, signal))
    {
        return Some(ScopeConflict {
            kind: "court_system",
            detected: signal.to_string(),
        });
    }
    None
}

fn phrase_present(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(start, found)| {
        let end = start + found.len();
        let left = text[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        let right = text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        left && right
    })
}

fn title_case(value: &str) -> String {
    value
        .split_ascii_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
