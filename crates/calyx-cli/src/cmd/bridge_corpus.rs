//! `calyx materialize-bridge-corpus` creates a small physical graph from real rows.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, VaultId, VaultStore};
use calyx_lodestar::{
    AssocStore, AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, PhysicalAsterAssocSnapshot,
    encode_assoc_node_props,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::vault::{home_dir, vault_salt};
use super::{Subcommand, validate_vault_name, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const PANEL_VERSION: u32 = 994;
const PANEL_TEMPLATE: &str = "bridge-corpus";
const DOMAIN_ANCHOR: &str = "bridge-corpus";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializeBridgeCorpusArgs {
    pub name: String,
    pub rows: PathBuf,
    pub home: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct BridgeCorpusRow {
    id: String,
    domain: String,
    text: String,
    bridge_terms: Vec<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Clone)]
struct MaterializedRow {
    id: String,
    domain: String,
    text: String,
    terms: Vec<String>,
    metadata: BTreeMap<String, String>,
    cx_id: CxId,
}

#[derive(Debug, Serialize)]
struct MaterializeBridgeCorpusReport {
    status: &'static str,
    name: String,
    vault_id: String,
    vault_dir: String,
    rows_jsonl: String,
    rows_jsonl_bytes: u64,
    rows_jsonl_sha256: String,
    row_count: usize,
    domain_counts: BTreeMap<String, usize>,
    bridge_term_count: usize,
    graph_nodes_written: usize,
    graph_edges_written: usize,
    csr_persisted: bool,
    readback: MaterializeBridgeCorpusReadback,
}

#[derive(Debug, Serialize)]
struct MaterializeBridgeCorpusReadback {
    index_contains_name: bool,
    node_count: usize,
    edge_count: usize,
}

pub(crate) fn parse_materialize_bridge_corpus(rest: &[String]) -> CliResult<Subcommand> {
    let name = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-bridge-corpus requires <name>"))?
        .clone();
    validate_vault_name(&name)?;
    let mut rows = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--rows" => {
                idx += 1;
                rows = Some(value(rest, idx, "--rows")?.into());
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-bridge-corpus flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::MaterializeBridgeCorpus(
        MaterializeBridgeCorpusArgs {
            name,
            rows: rows.ok_or_else(|| {
                CliError::usage("materialize-bridge-corpus requires --rows <jsonl>")
            })?,
            home,
        },
    ))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::MaterializeBridgeCorpus(args) = command else {
        unreachable!("non-materialize-bridge-corpus routed to bridge_corpus module");
    };
    let home = match args.home.clone() {
        Some(path) => path,
        None => home_dir()?,
    };
    let report = materialize(&home, args)?;
    print_json(&report)
}

fn materialize(
    home: &Path,
    args: MaterializeBridgeCorpusArgs,
) -> CliResult<MaterializeBridgeCorpusReport> {
    let rows_bytes = fs::read(&args.rows)?;
    let rows_sha256 = sha256_hex(&rows_bytes);
    let rows = read_rows(&args.rows)?;
    let mut domain_counts = BTreeMap::new();
    for row in &rows {
        *domain_counts.entry(row.domain.clone()).or_insert(0) += 1;
    }
    let vault_id = VaultId::from_ulid(Ulid::new());
    let relative = format!("vaults/{vault_id}");
    let vault_dir = home.join(&relative);
    if vault_dir.exists() {
        return Err(CliError::usage(format!(
            "vault directory for {vault_id} already exists"
        )));
    }
    ensure_index_can_add(home, &args.name)?;
    let salt = vault_salt(vault_id, &args.name);
    let rows = assign_ids(rows, &salt);
    let vault = AsterVault::new_durable(&vault_dir, vault_id, salt, VaultOptions::default())?;
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let mut term_to_rows: BTreeMap<String, Vec<CxId>> = BTreeMap::new();
    for row in &rows {
        let mut metadata = row.metadata.clone();
        metadata.insert("domain".to_string(), row.domain.clone());
        metadata.insert("text".to_string(), row.text.clone());
        metadata.insert("source_id".to_string(), row.id.clone());
        let props = AsterAssocNodeProps {
            anchors: vec![AnchorKind::Label(DOMAIN_ANCHOR.to_string())],
            metadata,
            ..Default::default()
        };
        graph.put_node(row.cx_id, &encode_assoc_node_props(&props)?)?;
        for term in &row.terms {
            term_to_rows
                .entry(term.clone())
                .or_default()
                .push(row.cx_id);
        }
    }
    let mut edges_written = 0usize;
    for (term, row_ids) in &term_to_rows {
        let term_id = term_id(term, vault_id);
        let props = AsterAssocNodeProps {
            metadata: BTreeMap::from([
                ("domain".to_string(), "bridge_term".to_string()),
                ("term".to_string(), term.clone()),
                ("source_dataset".to_string(), "bridge_terms".to_string()),
                ("source_id".to_string(), term.clone()),
                ("row_count".to_string(), row_ids.len().to_string()),
            ]),
            ..Default::default()
        };
        graph.put_node(term_id, &encode_assoc_node_props(&props)?)?;
        for row_id in row_ids {
            graph.put_edge(*row_id, "bridge_term", term_id, b"1")?;
            graph.put_edge(term_id, "bridge_term", *row_id, b"1")?;
            edges_written += 2;
        }
    }
    graph.rebuild_csr(vault.snapshot())?;
    vault.flush()?;
    add_index_entry(home, &args.name, vault_id, &relative)?;
    let readback = readback(home, &args.name, &vault_dir)?;
    Ok(MaterializeBridgeCorpusReport {
        status: "ok",
        name: args.name,
        vault_id: vault_id.to_string(),
        vault_dir: vault_dir.display().to_string(),
        rows_jsonl: args.rows.display().to_string(),
        rows_jsonl_bytes: rows_bytes.len() as u64,
        rows_jsonl_sha256: rows_sha256,
        row_count: rows.len(),
        domain_counts,
        bridge_term_count: term_to_rows.len(),
        graph_nodes_written: rows.len() + term_to_rows.len(),
        graph_edges_written: edges_written,
        csr_persisted: true,
        readback,
    })
}

fn read_rows(path: &Path) -> CliResult<Vec<BridgeCorpusRow>> {
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
        let mut row: BridgeCorpusRow = serde_json::from_str(&line).map_err(|error| {
            CliError::usage(format!(
                "bridge corpus row {} is invalid JSON: {error}",
                index + 1
            ))
        })?;
        validate_row(index + 1, &mut row)?;
        if !ids.insert(row.id.clone()) {
            return Err(CliError::usage(format!(
                "bridge corpus row {} duplicates id {}",
                index + 1,
                row.id
            )));
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(CliError::usage("bridge corpus rows file is empty"));
    }
    Ok(rows)
}

fn validate_row(line: usize, row: &mut BridgeCorpusRow) -> CliResult {
    row.id = row.id.trim().to_string();
    row.domain = row.domain.trim().to_string();
    if row.id.is_empty() || row.domain.is_empty() || row.text.trim().is_empty() {
        return Err(CliError::usage(format!(
            "bridge corpus row {line} requires non-empty id, domain, and text"
        )));
    }
    for required in ["source_dataset", "source_path", "source_sha256"] {
        if row
            .metadata
            .get(required)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(CliError::usage(format!(
                "bridge corpus row {line} metadata requires {required}"
            )));
        }
    }
    let text = row.text.to_lowercase();
    let mut terms = BTreeSet::new();
    for term in &row.bridge_terms {
        let normalized = normalize_term(term);
        if normalized.is_empty() {
            return Err(CliError::usage(format!(
                "bridge corpus row {line} has an empty bridge term"
            )));
        }
        if !text.contains(&normalized) {
            return Err(CliError::usage(format!(
                "bridge corpus row {line} text does not contain bridge term {normalized}"
            )));
        }
        terms.insert(normalized);
    }
    if terms.is_empty() {
        return Err(CliError::usage(format!(
            "bridge corpus row {line} requires at least one bridge term"
        )));
    }
    row.bridge_terms = terms.into_iter().collect();
    Ok(())
}

fn assign_ids(rows: Vec<BridgeCorpusRow>, salt: &[u8]) -> Vec<MaterializedRow> {
    rows.into_iter()
        .map(|row| {
            let cx_id = CxId::from_input(
                format!("bridge-corpus-row:{}:{}", row.domain, row.id).as_bytes(),
                PANEL_VERSION,
                salt,
            );
            MaterializedRow {
                id: row.id,
                domain: row.domain,
                text: row.text,
                terms: row.bridge_terms,
                metadata: row.metadata,
                cx_id,
            }
        })
        .collect()
}

fn term_id(term: &str, vault_id: VaultId) -> CxId {
    CxId::from_input(
        format!("bridge-corpus-term:{term}").as_bytes(),
        PANEL_VERSION,
        vault_id.to_string().as_bytes(),
    )
}

fn normalize_term(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn ensure_index_can_add(home: &Path, name: &str) -> CliResult {
    let index = read_index_value(home)?;
    let Some(vaults) = index.get("vaults").and_then(Value::as_array) else {
        return Err(CliError::usage("vault index is missing a vaults array"));
    };
    if vaults
        .iter()
        .any(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
    {
        return Err(CliError::usage(format!("vault name {name} already exists")));
    }
    Ok(())
}

fn add_index_entry(home: &Path, name: &str, vault_id: VaultId, relative: &str) -> CliResult {
    let mut index = read_index_value(home)?;
    let Some(vaults) = index.get_mut("vaults").and_then(Value::as_array_mut) else {
        return Err(CliError::usage("vault index is missing a vaults array"));
    };
    vaults.push(json!({
        "name": name,
        "vault_id": vault_id,
        "path": relative,
        "panel_template": PANEL_TEMPLATE,
    }));
    vaults.sort_by(|left, right| {
        let left = left.get("name").and_then(Value::as_str).unwrap_or_default();
        let right = right
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        left.cmp(right)
    });
    let path = index_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::durable_write::write_json_value_atomic(&path, &index, "vault index")
}

fn read_index_value(home: &Path) -> CliResult<Value> {
    let path = index_path(home);
    if !path.exists() {
        return Ok(json!({ "vaults": [] }));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn index_path(home: &Path) -> PathBuf {
    home.join("vaults").join("index.json")
}

fn readback(
    home: &Path,
    name: &str,
    vault_dir: &Path,
) -> CliResult<MaterializeBridgeCorpusReadback> {
    let index = read_index_value(home)?;
    let index_contains_name = index
        .get("vaults")
        .and_then(Value::as_array)
        .is_some_and(|vaults| {
            vaults
                .iter()
                .any(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        });
    let snapshot = PhysicalAsterAssocSnapshot::latest(vault_dir, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let graph = snapshot.full_graph()?;
    Ok(MaterializeBridgeCorpusReadback {
        index_contains_name,
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_requires_rows() {
        let err = parse_materialize_bridge_corpus(&["demo".to_string()]).unwrap_err();
        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    }

    #[test]
    fn parser_accepts_rows_and_home() {
        let parsed = parse_materialize_bridge_corpus(&[
            "demo".to_string(),
            "--rows".to_string(),
            "rows.jsonl".to_string(),
            "--home".to_string(),
            "target/home".to_string(),
        ])
        .unwrap();
        assert_eq!(
            parsed,
            Subcommand::MaterializeBridgeCorpus(MaterializeBridgeCorpusArgs {
                name: "demo".to_string(),
                rows: "rows.jsonl".into(),
                home: Some("target/home".into()),
            })
        );
    }

    #[test]
    fn materializes_graph_and_reopens_readback() {
        let root = temp_root("happy");
        fs::create_dir_all(&root).unwrap();
        let rows = root.join("rows.jsonl");
        fs::write(
            &rows,
            concat!(
                r#"{"id":"clinical-1","domain":"clinical","text":"metformin treats diabetes in clinical reports","bridge_terms":["metformin","diabetes"],"metadata":{"source_dataset":"pubmedqa","source_path":"/source/pubmedqa.jsonl","source_sha256":"aaa"}}"#,
                "\n",
                r#"{"id":"drug-1","domain":"molecular","text":"metformin binding record for diabetes target context","bridge_terms":["metformin","diabetes"],"metadata":{"source_dataset":"bindingdb","source_path":"/source/bindingdb.tsv","source_sha256":"bbb"}}"#,
                "\n",
            ),
        )
        .unwrap();
        let report = materialize(
            &root,
            MaterializeBridgeCorpusArgs {
                name: "demo".to_string(),
                rows,
                home: None,
            },
        )
        .unwrap();
        assert_eq!(report.row_count, 2);
        assert_eq!(report.bridge_term_count, 2);
        assert!(report.readback.index_contains_name);
        assert_eq!(report.readback.node_count, 4);
        assert_eq!(report.readback.edge_count, 8);
    }

    #[test]
    fn rejects_unproven_bridge_term() {
        let root = temp_root("bad-term");
        fs::create_dir_all(&root).unwrap();
        let rows = root.join("rows.jsonl");
        fs::write(
            &rows,
            r#"{"id":"clinical-1","domain":"clinical","text":"source text","bridge_terms":["absent"],"metadata":{"source_dataset":"pubmedqa","source_path":"/source/pubmedqa.jsonl","source_sha256":"aaa"}}"#,
        )
        .unwrap();
        let err = materialize(
            &root,
            MaterializeBridgeCorpusArgs {
                name: "demo".to_string(),
                rows,
                home: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains("does not contain bridge term"));
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("calyx-bridge-corpus-{name}-{}", std::process::id()))
    }
}
