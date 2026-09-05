//! DB-native source-opinion aliases for canonical legal constellations.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, VaultStore, content_address};
use calyx_lodestar::AsterAssocNodeProps;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod verify;
mod write;

pub(crate) use verify::verify_idmap_physical;

pub(crate) const DEFAULT_COLLECTION: &str = "legal-opinion-aliases-v1";
pub(crate) const SCHEMA: &str = "legal_opinion_alias_overlay_v1";
pub(crate) const EDGE_TYPE: &str = "aliases_to";
pub(crate) const SUMMARY_KEY: &str = "opinion_alias_overlay_summary";
pub(crate) const SOURCE_NODE_TYPE: &str = "source_opinion_alias";
pub(crate) const TARGET_NODE_TYPE: &str = "canonical_constellation";
const CSV_HEADER: [&str; 6] = [
    "opinion_id",
    "cx_id",
    "canonical_opinion_id",
    "content_sha256",
    "is_canonical",
    "source_url",
];

#[derive(Clone, Debug)]
pub(crate) struct MaterializeArgs {
    pub vault: String,
    pub aliases: PathBuf,
    pub collection: String,
    pub report: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AliasRow {
    pub opinion_id: String,
    pub cx_id: CxId,
    pub canonical_opinion_id: String,
    pub content_sha256: String,
    pub is_canonical: bool,
    pub source_url: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AliasInput {
    pub path: PathBuf,
    pub bytes: usize,
    pub sha256: String,
    pub rows: BTreeMap<String, AliasRow>,
    pub canonical_rows: usize,
}

#[derive(Debug, Serialize)]
struct ResolveReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    collection: String,
    opinion_id: String,
    alias_node_id: String,
    canonical_opinion_id: String,
    canonical_cx_id: String,
    content_sha256: String,
    source_url: String,
    alias_node_sha256: String,
    target_node_sha256: String,
    edge_sha256: String,
    base_metadata_exact: bool,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    match args.first().map(String::as_str) {
        Some("materialize-opinion-aliases") => Some(materialize_command(&args[1..])),
        Some("opinion-alias-resolve") => Some(resolve_command(&args[1..])),
        _ => None,
    }
}

fn materialize_command(rest: &[String]) -> CliResult {
    if matches!(rest, [flag] if matches!(flag.as_str(), "--help" | "-h")) {
        return crate::usage::print_command_usage("materialize-opinion-aliases");
    }
    let args = parse_materialize(rest)?;
    let input = load_aliases(&args.aliases)?;
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let report = write::materialize(&home, &args, &input)?;
    print_json(&report)
}

fn parse_materialize(rest: &[String]) -> CliResult<MaterializeArgs> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-opinion-aliases requires <vault>"))?
        .clone();
    let mut aliases = None;
    let mut collection = None;
    let mut report = None;
    let mut home = None;
    let mut index = 1;
    while index < rest.len() {
        let flag = &rest[index];
        index += 1;
        let value = rest.get(index).ok_or_else(|| {
            CliError::usage(format!(
                "materialize-opinion-aliases {flag} requires a value"
            ))
        })?;
        match flag.as_str() {
            "--aliases" => aliases = Some(value.into()),
            "--collection" => collection = Some(value.clone()),
            "--report" => report = Some(value.into()),
            "--home" => home = Some(value.into()),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-opinion-aliases flag {other}"
                )));
            }
        }
        index += 1;
    }
    Ok(MaterializeArgs {
        vault,
        aliases: aliases.ok_or_else(|| {
            CliError::usage("materialize-opinion-aliases requires --aliases <csv>")
        })?,
        collection: collection.unwrap_or_else(|| DEFAULT_COLLECTION.to_string()),
        report,
        home,
    })
}

pub(crate) fn alias_node_id(opinion_id: &str) -> CxId {
    CxId::from_bytes(content_address([
        b"calyx-legal-opinion-alias-v1".as_slice(),
        opinion_id.as_bytes(),
    ]))
}

fn load_aliases(path: &Path) -> CliResult<AliasInput> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::io(format!(
            "inspect opinion-alias CSV {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_INPUT_NOT_PLAIN_FILE",
            format!("opinion-alias CSV {} is not a plain file", path.display()),
            "provide the sealed physical opinion_cx_aliases.csv member",
        ));
    }
    let file = fs::File::open(path).map_err(|error| {
        CliError::io(format!(
            "open opinion-alias CSV {}: {error}",
            path.display()
        ))
    })?;
    let mut lines = BufReader::new(file).lines();
    let header = lines
        .next()
        .transpose()
        .map_err(|error| CliError::io(format!("read opinion-alias header: {error}")))?
        .ok_or_else(|| CliError::usage("opinion-alias CSV is empty"))?;
    let actual_header = header.trim_end_matches('\r').split(',').collect::<Vec<_>>();
    if actual_header != CSV_HEADER {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_SCHEMA_MISMATCH",
            format!("opinion-alias CSV header is {actual_header:?}"),
            "rebuild the sealed six-column vault-alias generation",
        ));
    }
    let mut rows = BTreeMap::new();
    for (offset, line) in lines.enumerate() {
        let line_number = offset + 2;
        let line = line.map_err(|error| {
            CliError::io(format!("read opinion-alias row {line_number}: {error}"))
        })?;
        if line.trim().is_empty() {
            return Err(alias_row_error(line_number, "row is blank"));
        }
        let fields = line.trim_end_matches('\r').split(',').collect::<Vec<_>>();
        if fields.len() != CSV_HEADER.len() {
            return Err(alias_row_error(
                line_number,
                format!("row has {} fields", fields.len()),
            ));
        }
        let opinion_id = positive_id(fields[0], line_number, "opinion_id")?;
        let cx_id = fields[1]
            .parse::<CxId>()
            .map_err(|error| alias_row_error(line_number, format!("cx_id is invalid: {error}")))?;
        let canonical_opinion_id = positive_id(fields[2], line_number, "canonical_opinion_id")?;
        let content_sha256 = fields[3];
        if !is_sha256(content_sha256) {
            return Err(alias_row_error(
                line_number,
                "content_sha256 is not lowercase SHA-256",
            ));
        }
        let is_canonical = match fields[4] {
            "true" => true,
            "false" => false,
            _ => {
                return Err(alias_row_error(
                    line_number,
                    "is_canonical is not true/false",
                ));
            }
        };
        if is_canonical != (opinion_id == canonical_opinion_id) {
            return Err(alias_row_error(
                line_number,
                "is_canonical differs from the opinion/canonical identity relation",
            ));
        }
        let source_url = fields[5];
        if !source_url.starts_with("https://www.courtlistener.com/opinion/") {
            return Err(alias_row_error(
                line_number,
                "source_url is not a CourtListener opinion URL",
            ));
        }
        let row = AliasRow {
            opinion_id: opinion_id.clone(),
            cx_id,
            canonical_opinion_id,
            content_sha256: content_sha256.to_string(),
            is_canonical,
            source_url: source_url.to_string(),
        };
        if rows.insert(opinion_id.clone(), row).is_some() {
            return Err(alias_row_error(
                line_number,
                format!("opinion_id {opinion_id} is duplicated"),
            ));
        }
    }
    if rows.is_empty() {
        return Err(CliError::usage("opinion-alias CSV has no data rows"));
    }
    for row in rows.values() {
        let canonical = rows.get(&row.canonical_opinion_id).ok_or_else(|| {
            contract_error(
                "CALYX_OPINION_ALIAS_TARGET_MISSING",
                format!(
                    "opinion {} targets absent canonical opinion {}",
                    row.opinion_id, row.canonical_opinion_id
                ),
                "rebuild the complete alias relation from the canonical ingest generation",
            )
        })?;
        if !canonical.is_canonical
            || canonical.cx_id != row.cx_id
            || canonical.content_sha256 != row.content_sha256
        {
            return Err(contract_error(
                "CALYX_OPINION_ALIAS_TARGET_CONFLICT",
                format!(
                    "opinion {} disagrees with its canonical target",
                    row.opinion_id
                ),
                "repair the conflicting alias/canonical metadata and rebuild from source",
            ));
        }
    }
    let bytes = fs::read(path).map_err(|error| {
        CliError::io(format!(
            "reopen opinion-alias CSV {}: {error}",
            path.display()
        ))
    })?;
    Ok(AliasInput {
        path: path.to_path_buf(),
        bytes: bytes.len(),
        sha256: sha256_hex(&bytes),
        canonical_rows: rows.values().filter(|row| row.is_canonical).count(),
        rows,
    })
}

fn resolve_command(rest: &[String]) -> CliResult {
    if matches!(rest, [flag] if matches!(flag.as_str(), "--help" | "-h")) {
        return crate::usage::print_command_usage("opinion-alias-resolve");
    }
    let vault_name = rest
        .first()
        .ok_or_else(|| CliError::usage("opinion-alias-resolve requires <vault>"))?;
    let opinion_id = positive_id(
        rest.get(1)
            .ok_or_else(|| CliError::usage("opinion-alias-resolve requires <opinion-id>"))?,
        0,
        "opinion_id",
    )?;
    let mut collection = DEFAULT_COLLECTION.to_string();
    let mut home = None;
    let mut index = 2;
    while index < rest.len() {
        let flag = &rest[index];
        let value = rest
            .get(index + 1)
            .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--collection" => collection = value.clone(),
            "--home" => home = Some(PathBuf::from(value)),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected opinion-alias-resolve flag {other}"
                )));
            }
        }
        index += 2;
    }
    let home = home.map_or_else(home_dir, Ok)?;
    let resolved = resolve_vault_info(&home, vault_name)?;
    let physical = PhysicalPlainGraph::open_latest_isolated(&resolved.path, &collection)?;
    let alias_id = alias_node_id(&opinion_id);
    let alias_bytes = physical.get_node(alias_id)?.ok_or_else(|| {
        contract_error(
            "CALYX_OPINION_ALIAS_NOT_FOUND",
            format!("opinion alias {opinion_id} is absent from collection {collection}"),
            "materialize the complete accepted alias relation before resolving citations",
        )
    })?;
    let props: AsterAssocNodeProps = serde_json::from_slice(&alias_bytes).map_err(|error| {
        CliError::runtime(format!("decode physical opinion-alias node: {error}"))
    })?;
    let metadata = &props.metadata;
    if metadata.get("node_type").map(String::as_str) != Some(SOURCE_NODE_TYPE)
        || metadata.get("opinion_id") != Some(&opinion_id)
    {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_NODE_MISMATCH",
            format!("physical alias node {alias_id} has mismatched identity metadata"),
            "quarantine and rebuild the opinion-alias Graph collection",
        ));
    }
    let canonical_cx_id = metadata
        .get("canonical_cx_id")
        .ok_or_else(|| CliError::runtime("opinion-alias node lacks canonical_cx_id"))?
        .parse::<CxId>()
        .map_err(|error| CliError::runtime(format!("decode canonical_cx_id: {error}")))?;
    let target_bytes = physical
        .get_node(canonical_cx_id)?
        .ok_or_else(|| CliError::runtime("opinion-alias canonical target node is absent"))?;
    let edge_bytes = physical
        .get_edge(alias_id, EDGE_TYPE, canonical_cx_id)?
        .ok_or_else(|| CliError::runtime("opinion-alias aliases_to edge is absent"))?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            read_only: true,
            ..VaultOptions::default()
        },
    )?;
    let base = vault.get(canonical_cx_id, vault.snapshot())?;
    let canonical_opinion_id = metadata
        .get("canonical_opinion_id")
        .cloned()
        .unwrap_or_default();
    let content_sha256 = metadata.get("content_sha256").cloned().unwrap_or_default();
    let base_exact = base.metadata.get("opinion_id") == Some(&canonical_opinion_id)
        && base.metadata.get("canonical_opinion_id") == Some(&canonical_opinion_id)
        && base.metadata.get("ingest_text_sha256") == Some(&content_sha256);
    if !base_exact {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_BASE_MISMATCH",
            format!("alias {opinion_id} does not match canonical Base row {canonical_cx_id}"),
            "quarantine the alias collection and rebuild it from a fresh Base readback",
        ));
    }
    print_json(&ResolveReport {
        status: "verified",
        source_of_truth: "physical Aster Graph CF node/edge plus physical Base CF target",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        collection,
        opinion_id,
        alias_node_id: alias_id.to_string(),
        canonical_opinion_id,
        canonical_cx_id: canonical_cx_id.to_string(),
        content_sha256,
        source_url: metadata.get("source_url").cloned().unwrap_or_default(),
        alias_node_sha256: sha256_hex(&alias_bytes),
        target_node_sha256: sha256_hex(&target_bytes),
        edge_sha256: sha256_hex(&edge_bytes),
        base_metadata_exact: true,
    })
}

fn positive_id(value: &str, line: usize, field: &str) -> CliResult<String> {
    let parsed = value.parse::<u64>().map_err(|error| {
        alias_row_error(line, format!("{field} is not a positive integer: {error}"))
    })?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(alias_row_error(line, format!("{field} is not canonical")));
    }
    Ok(value.to_string())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn alias_row_error(line: usize, message: impl Into<String>) -> CliError {
    contract_error(
        "CALYX_OPINION_ALIAS_ROW_INVALID",
        format!("opinion-alias row {line}: {}", message.into()),
        "repair the sealed alias input and rerun before any Graph mutation",
    )
}

pub(crate) fn contract_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::from(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
