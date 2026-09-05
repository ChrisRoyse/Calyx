//! DB-native opinion-to-judge resolution, explicit refusal, and tenure relations.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use calyx_aster::base_page_index::visit_indexed_base_rows_for_keys;
use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::plain_graph::{
    GraphCollectionGenerationState, GraphCollectionGenerationStatus, GraphCollectionLifecycle,
    PhysicalGraphCollectionLifecycle, PhysicalPlainGraph, PlainGraph, PlainGraphCsr,
    PlainGraphCsrEdge, plain_graph_edge_raw_weight, plain_graph_normalized_edge_weight,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode::decode_constellation_base};
use calyx_core::{AnchorKind, CalyxError, CxId, VaultStore, content_address};
use calyx_lodestar::{AsterAssocNodeProps, encode_assoc_node_props};
use serde::Serialize;
use serde_json::{Value, json};

use super::opinion_alias_overlay::{contract_error, sha256_hex, verify_idmap_physical};
use super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use crate::durable_write::write_bytes_atomic_new;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_COLLECTION: &str = "legal-opinion-judges-v3";
const SCHEMA: &str = "legal_opinion_judge_associations_v1";
const MANIFEST_NAME: &str = "judge_manifest.json";
const MAPPING_NAME: &str = "opinion_judges.csv";
const JUDGES_NAME: &str = "judges_cuyahoga.jsonl";
const COVERAGE_NAME: &str = "judge_coverage.json";
const EXPECTED_FORMAT: &str = "calyx-cuyahoga-judges-generation-v3";
const EXPECTED_POLICY: &str = "signed-byline-tenure-aware-initials-unique-v3";
const SUMMARY_KEY: &str = "opinion_judge_association_summary";
const OPINION_NODE_TYPE: &str = "source_opinion_resolution";
const PERSON_NODE_TYPE: &str = "judge_person";
const POSITION_NODE_TYPE: &str = "court_position";
const REASON_NODE_TYPE: &str = "unresolved_reason";
const TARGET_NODE_TYPE: &str = "canonical_constellation";
const RESOLVED_EDGE: &str = "resolved_author";
const UNRESOLVED_EDGE: &str = "unresolved_because";
const BASE_EDGE: &str = "describes_constellation";
const POSITION_EDGE: &str = "holds_position";
const GRAPH_WAL_ROWS_PER_BATCH: usize = 10_000;
const ALIAS_HEADER: [&str; 6] = [
    "opinion_id",
    "cx_id",
    "canonical_opinion_id",
    "content_sha256",
    "is_canonical",
    "source_url",
];
const MAPPING_HEADER: [&str; 3] = ["opinion_id", "person_id", "method"];

#[derive(Clone, Debug)]
struct MaterializeArgs {
    vault: String,
    judge_generation: PathBuf,
    aliases: PathBuf,
    collection: String,
    report: Option<PathBuf>,
    home: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct AliasRow {
    opinion_id: String,
    cx_id: CxId,
    canonical_opinion_id: String,
    content_sha256: String,
    is_canonical: bool,
}

#[derive(Clone, Debug)]
struct ResolutionRow {
    opinion_id: String,
    person_id: Option<String>,
    method: String,
}

#[derive(Clone, Debug)]
struct PersonRow {
    person_id: String,
    name_first: String,
    name_middle: String,
    name_last: String,
    name_suffix: String,
    resolved_opinions: usize,
    courts: Vec<Value>,
    bytes_sha256: String,
}

#[derive(Clone, Debug)]
struct Input {
    generation: PathBuf,
    manifest_bytes: usize,
    manifest_sha256: String,
    mapping_bytes: usize,
    mapping_sha256: String,
    judges_bytes: usize,
    judges_sha256: String,
    coverage_bytes: usize,
    coverage_sha256: String,
    aliases_path: PathBuf,
    aliases_bytes: usize,
    aliases_sha256: String,
    source_extract_manifest_sha256: String,
    people_archive_sha256: String,
    positions_archive_sha256: String,
    resolutions: BTreeMap<String, ResolutionRow>,
    aliases: BTreeMap<String, AliasRow>,
    persons: BTreeMap<String, PersonRow>,
    resolved: usize,
    unresolved: usize,
}

#[derive(Clone, Debug)]
struct EdgeRow {
    src: CxId,
    edge_type: &'static str,
    dst: CxId,
    value: Vec<u8>,
}

type GraphNodeValues = BTreeMap<CxId, Vec<u8>>;
type ResolutionDimensionCounts = (usize, usize);
type ExpectedGraphRows = (GraphNodeValues, Vec<EdgeRow>, ResolutionDimensionCounts);

#[derive(Debug, Serialize)]
struct InputReport {
    judge_generation: String,
    judge_manifest_bytes: usize,
    judge_manifest_sha256: String,
    mapping_bytes: usize,
    mapping_sha256: String,
    judges_bytes: usize,
    judges_sha256: String,
    coverage_bytes: usize,
    coverage_sha256: String,
    aliases_path: String,
    aliases_bytes: usize,
    aliases_sha256: String,
    source_extract_manifest_sha256: String,
    people_archive_sha256: String,
    positions_archive_sha256: String,
}

#[derive(Debug, Serialize)]
struct Accounting {
    source_opinion_resolutions: usize,
    canonical_constellations: usize,
    resolved: usize,
    unresolved: usize,
    person_nodes: usize,
    position_nodes: usize,
    unresolved_reason_nodes: usize,
    graph_nodes: usize,
    graph_edges: usize,
}

#[derive(Debug, Serialize)]
struct Readback {
    physical_node_keys: usize,
    physical_edge_out_keys: usize,
    all_node_values_exact: bool,
    all_edge_values_exact: bool,
    base_rows_checked: usize,
    alias_rows_checked: usize,
    metadata_sha256: String,
    csr_nodes: usize,
    csr_edges: usize,
    csr_sha256: String,
    graph_wal_batches: usize,
    accepted_lifecycle_readback: bool,
}

#[derive(Debug, Serialize)]
struct MaterializeReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    collection: String,
    graph_generation: String,
    schema: &'static str,
    input: InputReport,
    accounting: Accounting,
    readback: Readback,
}

#[derive(Debug, Serialize)]
struct ResolveReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    collection: String,
    opinion_id: String,
    canonical_opinion_id: String,
    canonical_cx_id: String,
    resolution_status: String,
    resolution_method: String,
    person_id: Option<String>,
    unresolved_reason: Option<String>,
    opinion_node_sha256: String,
    resolution_edge_sha256: String,
    target_node_sha256: String,
    base_metadata_exact: bool,
}

#[derive(Debug, Serialize)]
struct CensusReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    collection: String,
    canonical_base_rows: usize,
    resolved: usize,
    unresolved: usize,
    dissent_rows: usize,
    dissent_resolved: usize,
    dissent_unresolved: usize,
    per_person_resolved: BTreeMap<String, usize>,
    per_method: BTreeMap<String, usize>,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    match args.first().map(String::as_str) {
        Some("materialize-judge-associations") => Some(materialize_command(&args[1..])),
        Some("judge-association-resolve") => Some(resolve_command(&args[1..])),
        Some("judge-association-census") => Some(census_command(&args[1..])),
        _ => None,
    }
}

fn materialize_command(rest: &[String]) -> CliResult {
    if matches!(rest, [flag] if matches!(flag.as_str(), "--help" | "-h")) {
        return crate::usage::print_command_usage("materialize-judge-associations");
    }
    let args = parse_materialize(rest)?;
    let input = load_input(&args.judge_generation, &args.aliases)?;
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let report = materialize(&home, &args, &input)?;
    print_json(&report)
}

fn parse_materialize(rest: &[String]) -> CliResult<MaterializeArgs> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-judge-associations requires <vault>"))?
        .clone();
    let mut judge_generation = None;
    let mut aliases = None;
    let mut collection = None;
    let mut report = None;
    let mut home = None;
    let mut index = 1;
    while index < rest.len() {
        let flag = &rest[index];
        let value = rest.get(index + 1).ok_or_else(|| {
            CliError::usage(format!(
                "materialize-judge-associations {flag} requires a value"
            ))
        })?;
        match flag.as_str() {
            "--judge-generation" => judge_generation = Some(value.into()),
            "--aliases" => aliases = Some(value.into()),
            "--collection" => collection = Some(value.clone()),
            "--report" => report = Some(value.into()),
            "--home" => home = Some(value.into()),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-judge-associations flag {other}"
                )));
            }
        }
        index += 2;
    }
    Ok(MaterializeArgs {
        vault,
        judge_generation: judge_generation.ok_or_else(|| {
            CliError::usage("materialize-judge-associations requires --judge-generation <dir>")
        })?,
        aliases: aliases.ok_or_else(|| {
            CliError::usage("materialize-judge-associations requires --aliases <csv>")
        })?,
        collection: collection.unwrap_or_else(|| DEFAULT_COLLECTION.to_string()),
        report,
        home,
    })
}

fn load_input(generation: &Path, aliases_path: &Path) -> CliResult<Input> {
    let generation = generation.canonicalize().map_err(|error| {
        CliError::io(format!(
            "resolve judge generation {}: {error}",
            generation.display()
        ))
    })?;
    let manifest_path = generation.join(MANIFEST_NAME);
    let manifest_bytes = read_plain(&manifest_path, "judge manifest")?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        contract_error(
            "CALYX_JUDGE_ASSOC_MANIFEST_INVALID",
            format!("decode judge manifest {}: {error}", manifest_path.display()),
            "rebuild and independently verify the immutable judge generation",
        )
    })?;
    if manifest.get("format").and_then(Value::as_str) != Some(EXPECTED_FORMAT)
        || manifest.get("resolution_policy").and_then(Value::as_str) != Some(EXPECTED_POLICY)
        || manifest.get("source_of_truth").and_then(Value::as_str) != Some(MANIFEST_NAME)
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_MANIFEST_CONTRACT_MISMATCH",
            "judge manifest is not the accepted signed-byline/tenure-aware v3 contract",
            "provide the independently verified v3 judge generation",
        ));
    }
    let mapping_path = generation.join(MAPPING_NAME);
    let judges_path = generation.join(JUDGES_NAME);
    let coverage_path = generation.join(COVERAGE_NAME);
    let mapping_bytes = read_plain(&mapping_path, "opinion-to-judge mapping")?;
    let judges_bytes = read_plain(&judges_path, "judge roster")?;
    let coverage_bytes = read_plain(&coverage_path, "judge coverage")?;
    verify_member(&manifest, MAPPING_NAME, &mapping_bytes)?;
    verify_member(&manifest, JUDGES_NAME, &judges_bytes)?;
    verify_member(&manifest, COVERAGE_NAME, &coverage_bytes)?;

    let resolutions = parse_mapping(&mapping_bytes)?;
    let persons = parse_persons(&judges_bytes)?;
    let resolved = resolutions
        .values()
        .filter(|row| row.person_id.is_some())
        .count();
    let unresolved = resolutions.len() - resolved;
    let coverage = manifest
        .get("coverage")
        .ok_or_else(|| CliError::runtime("judge manifest has no coverage"))?;
    require_count(coverage, "opinions_total", resolutions.len())?;
    require_count(coverage, "resolved_total", resolved)?;
    require_count(coverage, "unresolved_total", unresolved)?;
    let per_person = resolutions
        .values()
        .filter_map(|row| row.person_id.as_ref())
        .fold(BTreeMap::<String, usize>::new(), |mut counts, person_id| {
            *counts.entry(person_id.clone()).or_default() += 1;
            counts
        });
    if per_person.len() != persons.len()
        || persons.iter().any(|(person_id, person)| {
            per_person.get(person_id).copied() != Some(person.resolved_opinions)
        })
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_ROSTER_MAPPING_MISMATCH",
            "judge roster counts differ from the complete opinion mapping",
            "quarantine the generation and recompute its roster and mapping together",
        ));
    }

    let aliases_bytes = read_plain(aliases_path, "opinion-Cx alias map")?;
    let aliases = parse_aliases(&aliases_bytes)?;
    if resolutions.keys().ne(aliases.keys()) {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_IDENTITY_SET_MISMATCH",
            "judge mapping and opinion-Cx alias identity sets differ",
            "rebuild both generations from the same accepted extract identity set",
        ));
    }
    for row in resolutions.values().filter(|row| row.person_id.is_some()) {
        if !persons.contains_key(row.person_id.as_deref().unwrap_or_default()) {
            return Err(contract_error(
                "CALYX_JUDGE_ASSOC_PERSON_MISSING",
                format!("opinion {} resolves to an absent person", row.opinion_id),
                "quarantine the generation and rebuild the complete person roster",
            ));
        }
    }

    let source_extract_manifest_sha256 =
        json_string(&manifest, &["source_extract_generation", "manifest_sha256"])?;
    let people_archive_sha256 = json_string(
        &manifest,
        &["source_tables", "archives", "people", "archive_sha256"],
    )?;
    let positions_archive_sha256 = json_string(
        &manifest,
        &["source_tables", "archives", "positions", "archive_sha256"],
    )?;
    Ok(Input {
        generation,
        manifest_bytes: manifest_bytes.len(),
        manifest_sha256: sha256_hex(&manifest_bytes),
        mapping_bytes: mapping_bytes.len(),
        mapping_sha256: sha256_hex(&mapping_bytes),
        judges_bytes: judges_bytes.len(),
        judges_sha256: sha256_hex(&judges_bytes),
        coverage_bytes: coverage_bytes.len(),
        coverage_sha256: sha256_hex(&coverage_bytes),
        aliases_path: aliases_path.to_path_buf(),
        aliases_bytes: aliases_bytes.len(),
        aliases_sha256: sha256_hex(&aliases_bytes),
        source_extract_manifest_sha256,
        people_archive_sha256,
        positions_archive_sha256,
        resolutions,
        aliases,
        persons,
        resolved,
        unresolved,
    })
}

fn read_plain(path: &Path, label: &str) -> CliResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CliError::io(format!("inspect {label} {}: {error}", path.display())))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_INPUT_NOT_PLAIN_FILE",
            format!("{label} {} is not a plain file", path.display()),
            "provide immutable, independently verified generation members",
        ));
    }
    fs::read(path).map_err(|error| CliError::io(format!("read {label}: {error}")))
}

fn verify_member(manifest: &Value, name: &str, bytes: &[u8]) -> CliResult {
    let fact = manifest
        .get("files")
        .and_then(|value| value.get(name))
        .ok_or_else(|| CliError::runtime(format!("judge manifest omits {name}")))?;
    if fact.get("bytes").and_then(Value::as_u64) != Some(bytes.len() as u64)
        || fact.get("sha256").and_then(Value::as_str) != Some(&sha256_hex(bytes))
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_MEMBER_HASH_MISMATCH",
            format!("judge member {name} differs from the manifest"),
            "quarantine the generation and restore the declared bytes",
        ));
    }
    Ok(())
}

fn parse_mapping(bytes: &[u8]) -> CliResult<BTreeMap<String, ResolutionRow>> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let header = lines.next().unwrap_or_default();
    if split_csv(header) != MAPPING_HEADER {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_MAPPING_SCHEMA_MISMATCH",
            "opinion-to-judge mapping header differs",
            "provide the sealed three-column opinion_judges.csv member",
        ));
    }
    let mut rows = BTreeMap::new();
    for (offset, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let fields = split_csv(line);
        if fields.len() != MAPPING_HEADER.len() {
            return Err(row_error(offset + 2, "mapping row width differs"));
        }
        let opinion_id = positive_id(fields[0], offset + 2, "opinion_id")?;
        let method = fields[2].to_string();
        let person_id = if fields[1] == "UNRESOLVED" {
            if !method.starts_with("UNRESOLVED_") {
                return Err(row_error(
                    offset + 2,
                    "unresolved row has a resolved method",
                ));
            }
            None
        } else {
            if method.starts_with("UNRESOLVED_") {
                return Err(row_error(
                    offset + 2,
                    "resolved row has an unresolved method",
                ));
            }
            Some(positive_id(fields[1], offset + 2, "person_id")?)
        };
        let row = ResolutionRow {
            opinion_id: opinion_id.clone(),
            person_id,
            method,
        };
        if rows.insert(opinion_id, row).is_some() {
            return Err(row_error(offset + 2, "opinion_id is duplicated"));
        }
    }
    if rows.is_empty() {
        return Err(CliError::usage("opinion-to-judge mapping has no rows"));
    }
    Ok(rows)
}

fn parse_aliases(bytes: &[u8]) -> CliResult<BTreeMap<String, AliasRow>> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    if split_csv(lines.next().unwrap_or_default()) != ALIAS_HEADER {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_ALIAS_SCHEMA_MISMATCH",
            "opinion-Cx alias header differs",
            "provide the sealed six-column opinion alias map",
        ));
    }
    let mut rows = BTreeMap::new();
    for (offset, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        let fields = split_csv(line);
        if fields.len() != ALIAS_HEADER.len() {
            return Err(row_error(offset + 2, "alias row width differs"));
        }
        let opinion_id = positive_id(fields[0], offset + 2, "opinion_id")?;
        let cx_id = fields[1]
            .parse::<CxId>()
            .map_err(|error| row_error(offset + 2, format!("cx_id is invalid: {error}")))?;
        let canonical_opinion_id = positive_id(fields[2], offset + 2, "canonical_opinion_id")?;
        let is_canonical = match fields[4] {
            "true" => true,
            "false" => false,
            _ => return Err(row_error(offset + 2, "is_canonical is invalid")),
        };
        if is_canonical != (opinion_id == canonical_opinion_id) {
            return Err(row_error(offset + 2, "canonical identity flag differs"));
        }
        if fields[3].len() != 64 {
            return Err(row_error(offset + 2, "content SHA-256 length differs"));
        }
        let row = AliasRow {
            opinion_id: opinion_id.clone(),
            cx_id,
            canonical_opinion_id,
            content_sha256: fields[3].to_string(),
            is_canonical,
        };
        if rows.insert(opinion_id, row).is_some() {
            return Err(row_error(offset + 2, "alias opinion_id is duplicated"));
        }
    }
    Ok(rows)
}

fn parse_persons(bytes: &[u8]) -> CliResult<BTreeMap<String, PersonRow>> {
    let reader = BufReader::new(bytes);
    let mut persons = BTreeMap::new();
    for (offset, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| CliError::io(format!("read judge row: {error}")))?;
        if line.is_empty() {
            return Err(row_error(offset + 1, "judge row is blank"));
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|error| row_error(offset + 1, format!("judge JSON is invalid: {error}")))?;
        let person_id = value
            .get("person_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| row_error(offset + 1, "judge person_id is invalid"))?
            .to_string();
        let resolved_opinions = value
            .get("resolved_opinions")
            .and_then(Value::as_u64)
            .ok_or_else(|| row_error(offset + 1, "resolved_opinions is invalid"))?
            as usize;
        let courts = value
            .get("courts")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| row_error(offset + 1, "judge courts are invalid"))?;
        let person = PersonRow {
            person_id: person_id.clone(),
            name_first: required_string(&value, "name_first", offset + 1)?,
            name_middle: required_string(&value, "name_middle", offset + 1)?,
            name_last: required_string(&value, "name_last", offset + 1)?,
            name_suffix: required_string(&value, "name_suffix", offset + 1)?,
            resolved_opinions,
            courts,
            bytes_sha256: sha256_hex(line.as_bytes()),
        };
        if persons.insert(person_id, person).is_some() {
            return Err(row_error(offset + 1, "judge person_id is duplicated"));
        }
    }
    Ok(persons)
}

fn required_string(value: &Value, field: &str, line: usize) -> CliResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| row_error(line, format!("judge {field} is invalid")))
}

fn json_string(value: &Value, path: &[&str]) -> CliResult<String> {
    let mut current = value;
    for key in path {
        current = current
            .get(key)
            .ok_or_else(|| CliError::runtime(format!("manifest path {path:?} is absent")))?;
    }
    current
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| CliError::runtime(format!("manifest path {path:?} is not a string")))
}

fn require_count(value: &Value, field: &str, expected: usize) -> CliResult {
    if value.get(field).and_then(Value::as_u64) != Some(expected as u64) {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_ACCOUNTING_MISMATCH",
            format!("coverage {field} differs from physical rows"),
            "quarantine the generation and recompute all members together",
        ));
    }
    Ok(())
}

fn split_csv(line: &[u8]) -> Vec<&str> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    std::str::from_utf8(line).unwrap_or("").split(',').collect()
}

fn positive_id(value: &str, line: usize, field: &str) -> CliResult<String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| row_error(line, format!("{field} is not a positive integer: {error}")))?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(row_error(line, format!("{field} is not canonical")));
    }
    Ok(value.to_string())
}

fn row_error(line: usize, message: impl Into<String>) -> CliError {
    contract_error(
        "CALYX_JUDGE_ASSOC_ROW_INVALID",
        format!("judge association row {line}: {}", message.into()),
        "repair the immutable generation before any Graph mutation",
    )
}

fn materialize(home: &Path, args: &MaterializeArgs, input: &Input) -> CliResult<MaterializeReport> {
    preflight_report(args.report.as_deref())?;
    let resolved = resolve_vault_info(home, &args.vault)?;
    reject_existing_collection(&resolved, &args.collection)?;
    let idmap = input
        .aliases
        .iter()
        .map(|(opinion_id, row)| (opinion_id.clone(), row.cx_id))
        .collect::<BTreeMap<_, _>>();
    let alias_rows_checked = verify_idmap_physical(home, &args.vault, &idmap)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    let base_rows_checked = validate_base(&vault, input)?;
    let generation = format!("materialize-judge-associations-{}", ulid::Ulid::new());
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            args.collection.clone(),
            generation.clone(),
            GraphCollectionGenerationStatus::Writing,
            "materialize-judge-associations",
        )
        .with_reason("opinion-to-judge association materialization started")
        .with_detail("schema", SCHEMA)
        .with_detail("judge_manifest_sha256", input.manifest_sha256.clone()),
    )?;
    let graph = PlainGraph::new(&vault, &args.collection)?;
    let (nodes, edges, counts) = expected_graph_rows(input)?;
    let mut graph_rows = Vec::with_capacity(nodes.len() + edges.len() * 2);
    for (id, value) in &nodes {
        graph_rows.push((ColumnFamily::Graph, graph.node_key(*id), value.clone()));
    }
    for edge in &edges {
        let out = graph.edge_out_key(edge.src, edge.edge_type, edge.dst)?;
        let reverse = graph.edge_in_key(edge.dst, edge.edge_type, edge.src)?;
        graph_rows.push((ColumnFamily::Graph, out.clone(), edge.value.clone()));
        graph_rows.push((ColumnFamily::Graph, reverse, out));
    }
    let graph_wal_batches = graph_rows.len().div_ceil(GRAPH_WAL_ROWS_PER_BATCH);
    for batch in graph_rows.chunks(GRAPH_WAL_ROWS_PER_BATCH) {
        vault.write_cf_batch(batch.to_vec())?;
    }
    let summary = serde_json::to_vec(&json!({
        "schema": SCHEMA,
        "collection": args.collection,
        "judge_generation": input.generation,
        "judge_manifest_sha256": input.manifest_sha256,
        "mapping_sha256": input.mapping_sha256,
        "judges_sha256": input.judges_sha256,
        "coverage_sha256": input.coverage_sha256,
        "aliases_sha256": input.aliases_sha256,
        "source_extract_manifest_sha256": input.source_extract_manifest_sha256,
        "people_archive_sha256": input.people_archive_sha256,
        "positions_archive_sha256": input.positions_archive_sha256,
        "source_opinion_resolutions": input.resolutions.len(),
        "resolved": input.resolved,
        "unresolved": input.unresolved,
        "nodes": nodes.len(),
        "edges": edges.len(),
    }))
    .map_err(|error| CliError::runtime(format!("serialize judge summary: {error}")))?;
    graph.put_metadata(SUMMARY_KEY, &summary)?;
    graph.write_csr_projection(build_csr(
        &args.collection,
        vault.snapshot(),
        &nodes,
        &edges,
    )?)?;
    vault.flush()?;
    drop(graph);
    drop(vault);

    let physical = PhysicalPlainGraph::open_latest_unchecked(&resolved.path, &args.collection)?;
    verify_graph(&physical, &nodes, &edges)?;
    let physical_summary = physical
        .get_metadata(SUMMARY_KEY)?
        .ok_or_else(|| CliError::runtime("physical judge summary is absent"))?;
    if physical_summary != summary {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_SUMMARY_READBACK_MISMATCH",
            "physical judge summary differs after flush",
            "quarantine the collection and inspect Graph CF persistence",
        ));
    }
    let csr_bytes = physical
        .read_csr_bytes()?
        .ok_or_else(|| CliError::runtime("physical judge CSR is absent"))?;
    let csr = physical
        .read_csr()?
        .ok_or_else(|| CliError::runtime("physical judge CSR does not decode"))?;
    if csr.nodes.len() != nodes.len() || csr.edges.len() != edges.len() {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_CSR_READBACK_MISMATCH",
            "physical judge CSR counts differ from Graph rows",
            "quarantine the collection and rebuild from accepted Graph rows",
        ));
    }
    let mut report = MaterializeReport {
        status: "verified",
        source_of_truth: "physical Aster Base CF plus Graph CF node/edge/metadata/CSR readback",
        vault: resolved.name.clone(),
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection: args.collection.clone(),
        graph_generation: generation.clone(),
        schema: SCHEMA,
        input: input_report(input),
        accounting: Accounting {
            source_opinion_resolutions: input.resolutions.len(),
            canonical_constellations: input
                .aliases
                .values()
                .filter(|row| row.is_canonical)
                .count(),
            resolved: input.resolved,
            unresolved: input.unresolved,
            person_nodes: input.persons.len(),
            position_nodes: counts.0,
            unresolved_reason_nodes: counts.1,
            graph_nodes: nodes.len(),
            graph_edges: edges.len(),
        },
        readback: Readback {
            physical_node_keys: physical.node_key_count()?,
            physical_edge_out_keys: physical.edge_out_key_count()?,
            all_node_values_exact: true,
            all_edge_values_exact: true,
            base_rows_checked,
            alias_rows_checked,
            metadata_sha256: sha256_hex(&physical_summary),
            csr_nodes: csr.nodes.len(),
            csr_edges: csr.edges.len(),
            csr_sha256: sha256_hex(&csr_bytes),
            graph_wal_batches,
            accepted_lifecycle_readback: false,
        },
    };
    accept_generation(&resolved, &args.collection, &generation, &report)?;
    report.readback.accepted_lifecycle_readback = true;
    write_report(args.report.as_deref(), &report)?;
    Ok(report)
}

fn input_report(input: &Input) -> InputReport {
    InputReport {
        judge_generation: input.generation.display().to_string(),
        judge_manifest_bytes: input.manifest_bytes,
        judge_manifest_sha256: input.manifest_sha256.clone(),
        mapping_bytes: input.mapping_bytes,
        mapping_sha256: input.mapping_sha256.clone(),
        judges_bytes: input.judges_bytes,
        judges_sha256: input.judges_sha256.clone(),
        coverage_bytes: input.coverage_bytes,
        coverage_sha256: input.coverage_sha256.clone(),
        aliases_path: input.aliases_path.display().to_string(),
        aliases_bytes: input.aliases_bytes,
        aliases_sha256: input.aliases_sha256.clone(),
        source_extract_manifest_sha256: input.source_extract_manifest_sha256.clone(),
        people_archive_sha256: input.people_archive_sha256.clone(),
        positions_archive_sha256: input.positions_archive_sha256.clone(),
    }
}

fn validate_base(vault: &AsterVault, input: &Input) -> CliResult<usize> {
    let snapshot = vault.snapshot();
    let mut count = 0;
    for row in input.aliases.values().filter(|row| row.is_canonical) {
        let base = vault.get(row.cx_id, snapshot).map_err(|error| {
            contract_error(
                "CALYX_JUDGE_ASSOC_BASE_MISSING",
                format!(
                    "canonical opinion {} Base read failed: {error}",
                    row.opinion_id
                ),
                "rebuild the association against the exact accepted Base generation",
            )
        })?;
        if base.metadata.get("opinion_id") != Some(&row.opinion_id)
            || base.metadata.get("canonical_opinion_id") != Some(&row.canonical_opinion_id)
            || base.metadata.get("ingest_text_sha256") != Some(&row.content_sha256)
        {
            return Err(contract_error(
                "CALYX_JUDGE_ASSOC_BASE_MISMATCH",
                format!("canonical opinion {} differs from Base", row.opinion_id),
                "quarantine the association and bind it to matching Base bytes",
            ));
        }
        count += 1;
    }
    Ok(count)
}

fn expected_graph_rows(input: &Input) -> CliResult<ExpectedGraphRows> {
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::new();
    for alias in input.aliases.values().filter(|row| row.is_canonical) {
        insert_node(&mut nodes, alias.cx_id, target_props(alias)?)?;
    }
    for person in input.persons.values() {
        insert_node(
            &mut nodes,
            person_node_id(&person.person_id),
            person_props(person)?,
        )?;
    }
    let mut position_count = 0;
    for person in input.persons.values() {
        for position in &person.courts {
            let position_id = position
                .get("position_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| CliError::runtime("judge court position_id is invalid"))?
                .to_string();
            let position_node = position_node_id(&position_id);
            insert_node(
                &mut nodes,
                position_node,
                position_props(&person.person_id, &position_id, position)?,
            )?;
            edges.push(edge(
                person_node_id(&person.person_id),
                POSITION_EDGE,
                position_node,
                json!({
                    "schema": SCHEMA,
                    "edge_type": POSITION_EDGE,
                    "weight": 1.0,
                    "person_id": person.person_id,
                    "position_id": position_id,
                }),
            )?);
            position_count += 1;
        }
    }
    let reasons = input
        .resolutions
        .values()
        .filter(|row| row.person_id.is_none())
        .map(|row| row.method.clone())
        .collect::<BTreeSet<_>>();
    for reason in &reasons {
        insert_node(&mut nodes, reason_node_id(reason), reason_props(reason)?)?;
    }
    for (opinion_id, resolution) in &input.resolutions {
        let alias = &input.aliases[opinion_id];
        let opinion_node = opinion_node_id(opinion_id);
        insert_node(&mut nodes, opinion_node, opinion_props(alias, resolution)?)?;
        edges.push(edge(
            opinion_node,
            BASE_EDGE,
            alias.cx_id,
            json!({
                "schema": SCHEMA,
                "edge_type": BASE_EDGE,
                "weight": 1.0,
                "opinion_id": opinion_id,
                "canonical_opinion_id": alias.canonical_opinion_id,
                "canonical_cx_id": alias.cx_id,
            }),
        )?);
        let (edge_type, target) = if let Some(person_id) = &resolution.person_id {
            (RESOLVED_EDGE, person_node_id(person_id))
        } else {
            (UNRESOLVED_EDGE, reason_node_id(&resolution.method))
        };
        edges.push(edge(
            opinion_node,
            edge_type,
            target,
            json!({
                "schema": SCHEMA,
                "edge_type": edge_type,
                "weight": 1.0,
                "opinion_id": opinion_id,
                "person_id": resolution.person_id,
                "method": resolution.method,
            }),
        )?);
    }
    Ok((nodes, edges, (position_count, reasons.len())))
}

fn insert_node(nodes: &mut BTreeMap<CxId, Vec<u8>>, id: CxId, value: Vec<u8>) -> CliResult {
    if nodes.insert(id, value).is_some() {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_NODE_COLLISION",
            format!("judge association node {id} is duplicated"),
            "change the versioned node identity domain or repair duplicate inputs",
        ));
    }
    Ok(())
}

fn props(node_type: &str, metadata: BTreeMap<String, String>) -> CliResult<Vec<u8>> {
    encode_assoc_node_props(&AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("legal_opinion_judge_association".into()),
            AnchorKind::Label(format!("legal_opinion_judge_association:{node_type}")),
        ],
        metadata,
        ..Default::default()
    })
    .map_err(CliError::from)
}

fn target_props(row: &AliasRow) -> CliResult<Vec<u8>> {
    let mut metadata = base_metadata(TARGET_NODE_TYPE);
    metadata.insert(
        "canonical_opinion_id".into(),
        row.canonical_opinion_id.clone(),
    );
    metadata.insert("canonical_cx_id".into(), row.cx_id.to_string());
    metadata.insert("content_sha256".into(), row.content_sha256.clone());
    props(TARGET_NODE_TYPE, metadata)
}

fn opinion_props(alias: &AliasRow, resolution: &ResolutionRow) -> CliResult<Vec<u8>> {
    let mut metadata = base_metadata(OPINION_NODE_TYPE);
    metadata.insert("opinion_id".into(), alias.opinion_id.clone());
    metadata.insert(
        "canonical_opinion_id".into(),
        alias.canonical_opinion_id.clone(),
    );
    metadata.insert("canonical_cx_id".into(), alias.cx_id.to_string());
    metadata.insert("content_sha256".into(), alias.content_sha256.clone());
    metadata.insert("is_canonical".into(), alias.is_canonical.to_string());
    metadata.insert("resolution_method".into(), resolution.method.clone());
    metadata.insert(
        "resolution_status".into(),
        if resolution.person_id.is_some() {
            "resolved"
        } else {
            "unresolved"
        }
        .into(),
    );
    if let Some(person_id) = &resolution.person_id {
        metadata.insert("person_id".into(), person_id.clone());
    } else {
        metadata.insert("unresolved_reason".into(), resolution.method.clone());
    }
    props(OPINION_NODE_TYPE, metadata)
}

fn person_props(person: &PersonRow) -> CliResult<Vec<u8>> {
    let mut metadata = base_metadata(PERSON_NODE_TYPE);
    metadata.insert("person_id".into(), person.person_id.clone());
    metadata.insert("name_first".into(), person.name_first.clone());
    metadata.insert("name_middle".into(), person.name_middle.clone());
    metadata.insert("name_last".into(), person.name_last.clone());
    metadata.insert("name_suffix".into(), person.name_suffix.clone());
    metadata.insert(
        "resolved_opinions".into(),
        person.resolved_opinions.to_string(),
    );
    metadata.insert("source_row_sha256".into(), person.bytes_sha256.clone());
    props(PERSON_NODE_TYPE, metadata)
}

fn position_props(person_id: &str, position_id: &str, value: &Value) -> CliResult<Vec<u8>> {
    let mut metadata = base_metadata(POSITION_NODE_TYPE);
    metadata.insert("person_id".into(), person_id.into());
    metadata.insert("position_id".into(), position_id.into());
    for key in [
        "court_id",
        "date_start",
        "date_termination",
        "position_type",
        "job_title",
        "organization_name",
    ] {
        metadata.insert(
            key.into(),
            value.get(key).and_then(Value::as_str).unwrap_or("").into(),
        );
    }
    metadata.insert(
        "source_person_id".into(),
        value
            .get("source_person_id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .unwrap_or_default(),
    );
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CliError::runtime(format!("serialize court position: {error}")))?;
    metadata.insert("source_row_sha256".into(), sha256_hex(&bytes));
    props(POSITION_NODE_TYPE, metadata)
}

fn reason_props(reason: &str) -> CliResult<Vec<u8>> {
    let mut metadata = base_metadata(REASON_NODE_TYPE);
    metadata.insert("reason".into(), reason.into());
    props(REASON_NODE_TYPE, metadata)
}

fn base_metadata(node_type: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("schema".into(), SCHEMA.into()),
        ("node_type".into(), node_type.into()),
    ])
}

fn edge(src: CxId, edge_type: &'static str, dst: CxId, value: Value) -> CliResult<EdgeRow> {
    Ok(EdgeRow {
        src,
        edge_type,
        dst,
        value: serde_json::to_vec(&value)
            .map_err(|error| CliError::runtime(format!("serialize judge edge: {error}")))?,
    })
}

fn opinion_node_id(opinion_id: &str) -> CxId {
    domain_id(b"calyx-legal-opinion-resolution-v1", opinion_id)
}

fn person_node_id(person_id: &str) -> CxId {
    domain_id(b"calyx-legal-judge-person-v1", person_id)
}

fn position_node_id(position_id: &str) -> CxId {
    domain_id(b"calyx-legal-court-position-v1", position_id)
}

fn reason_node_id(reason: &str) -> CxId {
    domain_id(b"calyx-legal-judge-unresolved-reason-v1", reason)
}

fn domain_id(domain: &[u8], value: &str) -> CxId {
    CxId::from_bytes(content_address([domain, value.as_bytes()]))
}

fn build_csr(
    collection: &str,
    snapshot: u64,
    nodes: &BTreeMap<CxId, Vec<u8>>,
    edges: &[EdgeRow],
) -> CliResult<PlainGraphCsr> {
    let node_ids = nodes.keys().copied().collect::<Vec<_>>();
    let indexes = node_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<BTreeMap<_, _>>();
    let mut max_weight = 0.0_f32;
    let mut drafts = Vec::new();
    let mut associations = BTreeSet::new();
    for edge in edges {
        let source = *indexes
            .get(&edge.src)
            .ok_or_else(|| CliError::runtime("judge CSR source node is absent"))?;
        if !indexes.contains_key(&edge.dst) {
            return Err(CliError::runtime("judge CSR target node is absent"));
        }
        let weight = plain_graph_edge_raw_weight(&edge.value)?;
        max_weight = max_weight.max(weight);
        drafts.push((source, edge.dst, edge.edge_type, weight));
        associations.insert((edge.src, edge.dst, edge.edge_type));
    }
    let mut by_source = vec![Vec::<PlainGraphCsrEdge>::new(); node_ids.len()];
    for (source, dst, edge_type, weight) in drafts {
        by_source[source].push(PlainGraphCsrEdge {
            dst,
            edge_type: edge_type.into(),
            weight: plain_graph_normalized_edge_weight(weight, max_weight)?,
        });
    }
    let mut offsets = vec![0];
    let mut csr_edges = Vec::new();
    for mut list in by_source {
        list.sort_by(|left, right| {
            left.edge_type
                .cmp(&right.edge_type)
                .then(left.dst.cmp(&right.dst))
        });
        csr_edges.extend(list);
        offsets.push(csr_edges.len());
    }
    Ok(PlainGraphCsr {
        collection: collection.into(),
        source_snapshot: snapshot,
        nodes: node_ids,
        offsets,
        edges: csr_edges,
        association_edge_count: associations.len(),
    })
}

fn verify_graph(
    physical: &PhysicalPlainGraph,
    nodes: &BTreeMap<CxId, Vec<u8>>,
    edges: &[EdgeRow],
) -> CliResult {
    let actual_nodes = physical
        .node_props()?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    if actual_nodes != *nodes {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_NODE_READBACK_MISMATCH",
            format!(
                "physical judge node map has {} rows; expected {}",
                actual_nodes.len(),
                nodes.len()
            ),
            "quarantine the collection and inspect Graph CF node persistence",
        ));
    }
    let expected_edges = edges
        .iter()
        .map(|edge| {
            (
                (edge.src, edge.edge_type.to_string(), edge.dst),
                edge.value.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let actual_edges = physical
        .edge_out_props()?
        .into_iter()
        .map(|edge| ((edge.src, edge.edge_type, edge.dst), edge.value))
        .collect::<BTreeMap<_, _>>();
    if actual_edges != expected_edges {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_EDGE_READBACK_MISMATCH",
            format!(
                "physical judge edge map has {} rows; expected {}",
                actual_edges.len(),
                expected_edges.len()
            ),
            "quarantine the collection and inspect Graph CF edge persistence",
        ));
    }
    Ok(())
}

fn resolve_command(rest: &[String]) -> CliResult {
    if matches!(rest, [flag] if matches!(flag.as_str(), "--help" | "-h")) {
        return crate::usage::print_command_usage("judge-association-resolve");
    }
    let (vault, opinion_id, collection, home) = parse_read_args(rest, "judge-association-resolve")?;
    let home = home.map_or_else(home_dir, Ok)?;
    let resolved = resolve_vault_info(&home, &vault)?;
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &collection)?;
    let opinion_node = opinion_node_id(&opinion_id);
    let opinion_bytes = physical.get_node(opinion_node)?.ok_or_else(|| {
        contract_error(
            "CALYX_JUDGE_ASSOC_OPINION_NOT_FOUND",
            format!("opinion {opinion_id} is absent from {collection}"),
            "materialize the complete accepted judge relation",
        )
    })?;
    let opinion: AsterAssocNodeProps = serde_json::from_slice(&opinion_bytes)
        .map_err(|error| CliError::runtime(format!("decode opinion resolution: {error}")))?;
    validate_opinion_metadata(&opinion.metadata, &opinion_id)?;
    let canonical_cx_id = parse_metadata_cx(&opinion.metadata, "canonical_cx_id")?;
    let target_bytes = physical
        .get_node(canonical_cx_id)?
        .ok_or_else(|| CliError::runtime("judge association Base target node is absent"))?;
    let status = opinion.metadata["resolution_status"].clone();
    let method = opinion.metadata["resolution_method"].clone();
    let (person_id, reason, edge_type, edge_target) = if status == "resolved" {
        let person = opinion
            .metadata
            .get("person_id")
            .cloned()
            .ok_or_else(|| CliError::runtime("resolved opinion node has no person_id"))?;
        (
            Some(person.clone()),
            None,
            RESOLVED_EDGE,
            person_node_id(&person),
        )
    } else {
        let reason = opinion
            .metadata
            .get("unresolved_reason")
            .cloned()
            .ok_or_else(|| CliError::runtime("unresolved opinion node has no reason"))?;
        (
            None,
            Some(reason.clone()),
            UNRESOLVED_EDGE,
            reason_node_id(&reason),
        )
    };
    let edge_bytes = physical
        .get_edge(opinion_node, edge_type, edge_target)?
        .ok_or_else(|| CliError::runtime("judge resolution edge is absent"))?;
    let base = read_physical_base_point(&resolved, canonical_cx_id)?;
    let canonical_opinion_id = opinion.metadata["canonical_opinion_id"].clone();
    let base_exact = base.metadata.get("opinion_id") == Some(&canonical_opinion_id)
        && base.metadata.get("canonical_opinion_id") == Some(&canonical_opinion_id)
        && base.metadata.get("ingest_text_sha256") == opinion.metadata.get("content_sha256");
    if !base_exact {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_BASE_READBACK_MISMATCH",
            format!("opinion {opinion_id} differs from physical Base"),
            "quarantine and rebuild the judge association collection",
        ));
    }
    print_json(&ResolveReport {
        status: "verified",
        source_of_truth: "physical Aster Graph CF resolution edge plus physical Base CF target",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        collection,
        opinion_id,
        canonical_opinion_id,
        canonical_cx_id: canonical_cx_id.to_string(),
        resolution_status: status,
        resolution_method: method,
        person_id,
        unresolved_reason: reason,
        opinion_node_sha256: sha256_hex(&opinion_bytes),
        resolution_edge_sha256: sha256_hex(&edge_bytes),
        target_node_sha256: sha256_hex(&target_bytes),
        base_metadata_exact: true,
    })
}

fn census_command(rest: &[String]) -> CliResult {
    if matches!(rest, [flag] if matches!(flag.as_str(), "--help" | "-h")) {
        return crate::usage::print_command_usage("judge-association-census");
    }
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("judge-association-census requires <vault>"))?
        .clone();
    let mut collection = DEFAULT_COLLECTION.to_string();
    let mut home = None;
    let mut index = 1;
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
                    "unexpected judge-association-census flag {other}"
                )));
            }
        }
        index += 2;
    }
    let home = home.map_or_else(home_dir, Ok)?;
    let resolved = resolve_vault_info(&home, &vault)?;
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &collection)?;
    let vault_store = open_read_only(&resolved)?;
    let mut resolved_count = 0;
    let mut unresolved_count = 0;
    let mut dissent_rows = 0;
    let mut dissent_resolved = 0;
    let mut per_person = BTreeMap::<String, usize>::new();
    let mut per_method = BTreeMap::<String, usize>::new();
    for (node_id, bytes) in physical.node_props()? {
        let props: AsterAssocNodeProps = serde_json::from_slice(&bytes)
            .map_err(|error| CliError::runtime(format!("decode judge graph node: {error}")))?;
        let metadata = props.metadata;
        if metadata.get("node_type").map(String::as_str) != Some(OPINION_NODE_TYPE)
            || metadata.get("is_canonical").map(String::as_str) != Some("true")
        {
            continue;
        }
        let opinion_id = metadata
            .get("opinion_id")
            .cloned()
            .ok_or_else(|| CliError::runtime("canonical opinion node has no opinion_id"))?;
        if node_id != opinion_node_id(&opinion_id) {
            return Err(CliError::runtime("canonical opinion node key differs"));
        }
        let cx_id = parse_metadata_cx(&metadata, "canonical_cx_id")?;
        let base = vault_store.get(cx_id, vault_store.snapshot())?;
        if base.metadata.get("opinion_id") != Some(&opinion_id) {
            return Err(CliError::runtime("judge census Base identity differs"));
        }
        let dissent = base.metadata.get("opinion_type").map(String::as_str) == Some("040dissent");
        if dissent {
            dissent_rows += 1;
        }
        let status = metadata.get("resolution_status").map(String::as_str);
        let method = metadata
            .get("resolution_method")
            .cloned()
            .ok_or_else(|| CliError::runtime("opinion node has no resolution method"))?;
        *per_method.entry(method).or_default() += 1;
        if status == Some("resolved") {
            let person_id = metadata
                .get("person_id")
                .cloned()
                .ok_or_else(|| CliError::runtime("resolved node has no person_id"))?;
            if physical
                .get_edge(node_id, RESOLVED_EDGE, person_node_id(&person_id))?
                .is_none()
            {
                return Err(CliError::runtime("resolved-author edge is absent"));
            }
            resolved_count += 1;
            *per_person.entry(person_id).or_default() += 1;
            if dissent {
                dissent_resolved += 1;
            }
        } else if status == Some("unresolved") {
            let reason = metadata
                .get("unresolved_reason")
                .cloned()
                .ok_or_else(|| CliError::runtime("unresolved node has no reason"))?;
            if physical
                .get_edge(node_id, UNRESOLVED_EDGE, reason_node_id(&reason))?
                .is_none()
            {
                return Err(CliError::runtime("unresolved-reason edge is absent"));
            }
            unresolved_count += 1;
        } else {
            return Err(CliError::runtime("opinion node status is invalid"));
        }
    }
    let canonical = resolved_count + unresolved_count;
    if canonical == 0 || dissent_resolved > dissent_rows {
        return Err(CliError::runtime("judge census accounting is invalid"));
    }
    print_json(&CensusReport {
        status: "verified",
        source_of_truth: "physical Aster Graph CF canonical resolution rows joined to physical Base CF",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        collection,
        canonical_base_rows: canonical,
        resolved: resolved_count,
        unresolved: unresolved_count,
        dissent_rows,
        dissent_resolved,
        dissent_unresolved: dissent_rows - dissent_resolved,
        per_person_resolved: per_person,
        per_method,
    })
}

fn parse_read_args(
    rest: &[String],
    command: &str,
) -> CliResult<(String, String, String, Option<PathBuf>)> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage(format!("{command} requires <vault>")))?
        .clone();
    let opinion_id = positive_id(
        rest.get(1)
            .ok_or_else(|| CliError::usage(format!("{command} requires <opinion-id>")))?,
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
                    "unexpected {command} flag {other}"
                )));
            }
        }
        index += 2;
    }
    Ok((vault, opinion_id, collection, home))
}

fn validate_opinion_metadata(metadata: &BTreeMap<String, String>, opinion_id: &str) -> CliResult {
    if metadata.get("node_type").map(String::as_str) != Some(OPINION_NODE_TYPE)
        || metadata.get("opinion_id").map(String::as_str) != Some(opinion_id)
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_OPINION_NODE_MISMATCH",
            format!("physical opinion node differs for {opinion_id}"),
            "quarantine and rebuild the judge association collection",
        ));
    }
    Ok(())
}

fn parse_metadata_cx(metadata: &BTreeMap<String, String>, key: &str) -> CliResult<CxId> {
    metadata
        .get(key)
        .ok_or_else(|| CliError::runtime(format!("node metadata omits {key}")))?
        .parse::<CxId>()
        .map_err(|error| CliError::runtime(format!("decode {key}: {error}")))
}

fn open_read_only(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            read_only: true,
            ..VaultOptions::default()
        },
    )
    .map_err(CliError::from)
}

fn read_physical_base_point(
    resolved: &ResolvedVault,
    canonical_cx_id: CxId,
) -> CliResult<calyx_core::Constellation> {
    let key = base_key(canonical_cx_id);
    let mut value = None;
    let stats = visit_indexed_base_rows_for_keys(
        &resolved.path,
        std::slice::from_ref(&key),
        |observed_key, observed_value| {
            if observed_key != key {
                return Err(contract_error(
                    "CALYX_JUDGE_ASSOC_BASE_KEY_MISMATCH",
                    format!(
                        "physical Base point read returned a different key for {canonical_cx_id}"
                    ),
                    "rebuild the Base page index before resolving judge associations",
                ));
            }
            value = observed_value;
            Ok(())
        },
    )?;
    if stats.unique_keys != 1
        || stats.touched_pages != 1
        || stats.source_files != 1
        || stats.live_rows != 1
        || stats.missing_rows != 0
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_BASE_POINT_INCOMPLETE",
            format!(
                "physical Base point read for {canonical_cx_id} visited {} keys, {} pages, {} source files, {} live rows, and {} missing rows",
                stats.unique_keys,
                stats.touched_pages,
                stats.source_files,
                stats.live_rows,
                stats.missing_rows,
            ),
            "rebuild the Base page index and retry the physical association read",
        ));
    }
    let bytes = value.ok_or_else(|| {
        contract_error(
            "CALYX_JUDGE_ASSOC_BASE_POINT_MISSING",
            format!("physical Base point row is absent for {canonical_cx_id}"),
            "restore the canonical Base row before resolving judge associations",
        )
    })?;
    let base = decode_constellation_base(&bytes)?;
    if base.cx_id != canonical_cx_id {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_BASE_ID_MISMATCH",
            format!("physical Base point row identity differs for {canonical_cx_id}"),
            "quarantine the Base page index and rebuild it from the canonical vault",
        ));
    }
    Ok(base)
}

fn preflight_report(path: Option<&Path>) -> CliResult {
    let Some(path) = path else { return Ok(()) };
    if fs::symlink_metadata(path).is_ok() {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_REPORT_EXISTS",
            format!("judge association report {} already exists", path.display()),
            "choose a new immutable report destination",
        ));
    }
    Ok(())
}

fn reject_existing_collection(resolved: &ResolvedVault, collection: &str) -> CliResult {
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    if lifecycle
        .list_states()?
        .iter()
        .any(|row| row.state.collection == collection)
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_COLLECTION_EXISTS",
            format!("judge association collection {collection} already has lifecycle state"),
            "use a new versioned collection name; never overwrite accepted association truth",
        ));
    }
    let physical = PhysicalPlainGraph::open_latest_unchecked(&resolved.path, collection)?;
    if physical.node_key_count()? != 0
        || physical.edge_out_key_count()? != 0
        || physical.get_metadata(SUMMARY_KEY)?.is_some()
        || physical.read_csr_bytes()?.is_some()
    {
        return Err(contract_error(
            "CALYX_JUDGE_ASSOC_COLLECTION_PHYSICAL_ROWS_EXIST",
            format!("judge association collection {collection} has orphaned Graph rows"),
            "quarantine the orphaned collection and use a new versioned name",
        ));
    }
    Ok(())
}

fn accept_generation(
    resolved: &ResolvedVault,
    collection: &str,
    generation: &str,
    report: &MaterializeReport,
) -> CliResult {
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    GraphCollectionLifecycle::new(&vault)?.put_state(
        &GraphCollectionGenerationState::new(
            collection,
            generation,
            GraphCollectionGenerationStatus::Accepted,
            "materialize-judge-associations",
        )
        .with_reason("physical Base and judge Graph readback passed")
        .with_detail("schema", SCHEMA)
        .with_detail(
            "judge_manifest_sha256",
            report.input.judge_manifest_sha256.clone(),
        )
        .with_detail(
            "source_opinion_resolutions",
            report.accounting.source_opinion_resolutions.to_string(),
        )
        .with_detail("resolved", report.accounting.resolved.to_string())
        .with_detail("unresolved", report.accounting.unresolved.to_string())
        .with_detail("csr_sha256", report.readback.csr_sha256.clone()),
    )?;
    vault.flush()?;
    drop(vault);
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    if !lifecycle.list_states()?.iter().any(|row| {
        row.state.collection == collection
            && row.state.generation == generation
            && row.state.status == GraphCollectionGenerationStatus::Accepted
    }) {
        return Err(CliError::from(CalyxError {
            code: "CALYX_JUDGE_ASSOC_LIFECYCLE_READBACK_MISSING",
            message: format!("accepted lifecycle row is absent for {collection}/{generation}"),
            remediation: "do not consume the collection until accepted state reads back",
        }));
    }
    Ok(())
}

fn write_report(path: Option<&Path>, report: &MaterializeReport) -> CliResult {
    let Some(path) = path else { return Ok(()) };
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize judge report: {error}")))?;
    write_bytes_atomic_new(path, &bytes, "judge association materialization report")?;
    if fs::read(path).map_err(|error| CliError::io(format!("read judge report: {error}")))? != bytes
    {
        return Err(CliError::runtime("judge report byte readback mismatch"));
    }
    Ok(())
}
