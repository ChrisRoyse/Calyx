//! Typed case-versus-opinion-part citation authority (#1853).
//!
//! Ordinary citation-map rows terminate at a synthetic case node. A relation
//! may terminate at an opinion-part constellation only when a separate explicit
//! evidence row names that part and is verified against the citing source text.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_core::{CalyxError, CxId, content_address};
use calyx_lodestar::AsterAssocNodeProps;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::opinion_alias_overlay::verify_idmap_physical;
use super::vault::home_dir;
use crate::durable_write::write_bytes_atomic_new;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod write;

pub(crate) const DEFAULT_COLLECTION: &str = "legal-citation-attribution-v1";
pub(crate) const SCHEMA: &str = "legal_citation_part_attribution_v1";
pub(crate) const CASE_NODE_TYPE: &str = "case";
pub(crate) const PART_NODE_TYPE: &str = "opinion_part";
pub(crate) const CONTAINS_EDGE: &str = "contains_opinion_part";
pub(crate) const CITES_CASE_EDGE: &str = "cites_case";
pub(crate) const CITES_PART_EDGE: &str = "cites_opinion_part";

#[derive(Clone, Debug)]
struct MaterializeArgs {
    vault: String,
    aliases: PathBuf,
    opinions: PathBuf,
    citations: PathBuf,
    explicit_parts: PathBuf,
    collection: String,
    report: PathBuf,
    home: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct TractionArgs {
    vault: String,
    opinion_id: String,
    collection: String,
    home: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(super) struct Node {
    pub id: CxId,
    pub node_type: &'static str,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct EdgeEvidence {
    pub source_row_id: String,
    pub citing_opinion_id: String,
    pub source_target_opinion_id: Option<String>,
    pub evidence_kind: String,
    pub evidence_sha256: Option<String>,
    pub source_text_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Edge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: &'static str,
    pub weight: f64,
    pub attribution: &'static str,
    pub evidence: Vec<EdgeEvidence>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Counts {
    pub accepted_opinion_rows: usize,
    pub physical_alias_rows_verified: usize,
    pub case_nodes: usize,
    pub opinion_part_nodes: usize,
    pub contains_opinion_part_edges: usize,
    pub citation_rows: usize,
    pub unique_cites_case_edges: usize,
    pub coalesced_cites_case_rows: usize,
    pub explicit_part_rows: usize,
    pub unique_cites_opinion_part_edges: usize,
    pub explicit_dissent_edges: usize,
    pub explicit_concurrence_edges: usize,
    pub explicit_lead_edges: usize,
    pub clusters_with_dissent: usize,
    pub ambiguous_case_edges_to_dissent_clusters: usize,
    pub no_case_to_sibling_smear: bool,
}

pub(super) struct Draft {
    pub nodes: BTreeMap<CxId, Node>,
    pub edges: BTreeMap<(CxId, &'static str, CxId), Edge>,
    pub counts: Counts,
}

#[derive(Clone, Debug)]
struct Part {
    opinion_id: String,
    cluster_id: u64,
    opinion_type: String,
    case_name: String,
    cx_id: CxId,
    source_text_sha256: String,
}

type LoadedParts = (BTreeMap<String, Part>, BTreeMap<String, String>, usize);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplicitPartRow {
    source_row_id: String,
    citing_opinion_id: String,
    case_target_opinion_id: String,
    part_target_opinion_id: String,
    attribution: String,
    evidence_text: String,
    evidence_sha256: String,
    source_text_sha256: String,
}

#[derive(Debug, Serialize)]
struct TractionReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    collection: String,
    opinion_id: String,
    opinion_part_node: String,
    physical_constellation_id: String,
    opinion_type: String,
    cluster_id: u64,
    case_node: String,
    membership_edge: Value,
    explicit_part_citations: usize,
    ambiguous_case_citations: usize,
    attribution_rule: &'static str,
    explicit_edges: Vec<Value>,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    match args.first().map(String::as_str) {
        Some("materialize-citation-attribution")
            if matches!(args.get(1).map(String::as_str), Some("--help" | "-h")) =>
        {
            Some(crate::usage::print_command_usage(
                "materialize-citation-attribution",
            ))
        }
        Some("materialize-citation-attribution") => {
            Some(parse_materialize(&args[1..]).and_then(materialize))
        }
        Some("citation-part-traction")
            if matches!(args.get(1).map(String::as_str), Some("--help" | "-h")) =>
        {
            Some(crate::usage::print_command_usage("citation-part-traction"))
        }
        Some("citation-part-traction") => Some(parse_traction(&args[1..]).and_then(traction)),
        _ => None,
    }
}

fn parse_materialize(rest: &[String]) -> CliResult<MaterializeArgs> {
    let vault = rest
        .first()
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CliError::usage("materialize-citation-attribution requires <vault>"))?;
    let mut aliases = None;
    let mut opinions = None;
    let mut citations = None;
    let mut explicit_parts = None;
    let mut collection = None;
    let mut report = None;
    let mut home = None;
    let mut index = 1;
    while index < rest.len() {
        let flag = &rest[index];
        let value = rest
            .get(index + 1)
            .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--aliases" => aliases = Some(value.into()),
            "--opinions" => opinions = Some(value.into()),
            "--citations" => citations = Some(value.into()),
            "--explicit-parts" => explicit_parts = Some(value.into()),
            "--collection" => collection = Some(value.clone()),
            "--report" => report = Some(value.into()),
            "--home" => home = Some(value.into()),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-citation-attribution flag {other}"
                )));
            }
        }
        index += 2;
    }
    Ok(MaterializeArgs {
        vault,
        aliases: aliases.ok_or_else(|| CliError::usage("--aliases <csv> is required"))?,
        opinions: opinions.ok_or_else(|| CliError::usage("--opinions <jsonl> is required"))?,
        citations: citations.ok_or_else(|| CliError::usage("--citations <csv> is required"))?,
        explicit_parts: explicit_parts
            .ok_or_else(|| CliError::usage("--explicit-parts <jsonl> is required"))?,
        collection: collection.unwrap_or_else(|| DEFAULT_COLLECTION.to_string()),
        report: report.ok_or_else(|| CliError::usage("--report <json> is required"))?,
        home,
    })
}

fn parse_traction(rest: &[String]) -> CliResult<TractionArgs> {
    let vault = rest
        .first()
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CliError::usage("citation-part-traction requires <vault>"))?;
    let opinion_id = rest
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CliError::usage("citation-part-traction requires <opinion-id>"))?;
    let mut collection = None;
    let mut home = None;
    let mut index = 2;
    while index < rest.len() {
        let flag = &rest[index];
        let value = rest
            .get(index + 1)
            .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--collection" => collection = Some(value.clone()),
            "--home" => home = Some(value.into()),
            other => {
                return Err(CliError::usage(format!(
                    "unexpected citation-part-traction flag {other}"
                )));
            }
        }
        index += 2;
    }
    Ok(TractionArgs {
        vault,
        opinion_id,
        collection: collection.unwrap_or_else(|| DEFAULT_COLLECTION.to_string()),
        home,
    })
}

fn materialize(args: MaterializeArgs) -> CliResult {
    require_new_output(&args.report)?;
    let explicit = load_explicit(&args.explicit_parts)?;
    if explicit.is_empty() {
        return Err(contract_error(
            "CALYX_CITATION_PART_EVIDENCE_EMPTY",
            "explicit part evidence is empty",
            "provide at least one source-text-bound part-specific citation; never infer one from case membership",
        ));
    }
    let aliases = load_aliases(&args.aliases)?;
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let verified = verify_idmap_physical(&home, &args.vault, &aliases)?;
    let (parts, explicit_texts, accepted_rows) = load_parts(&args.opinions, &aliases, &explicit)?;
    let mut draft = build_base(&args.citations, &parts, accepted_rows)?;
    draft.counts.physical_alias_rows_verified = verified;
    append_explicit(&explicit, &explicit_texts, &parts, &mut draft)?;
    validate_typed_edges(&draft)?;
    draft.counts.no_case_to_sibling_smear = true;
    let report = write::write_to_calyx(&home, &args, draft)?;
    print_json(&report)
}

fn build_base(
    citations: &Path,
    parts: &BTreeMap<String, Part>,
    accepted_rows: usize,
) -> CliResult<Draft> {
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut clusters = BTreeMap::<u64, (String, BTreeSet<String>, BTreeSet<String>)>::new();
    for part in parts.values() {
        let cluster = clusters
            .entry(part.cluster_id)
            .or_insert_with(|| (part.case_name.clone(), BTreeSet::new(), BTreeSet::new()));
        if cluster.0 != part.case_name {
            return Err(contract_error(
                "CALYX_CITATION_PART_CASE_NAME_CONFLICT",
                format!(
                    "cluster {} has case names {:?} and {:?}",
                    part.cluster_id, cluster.0, part.case_name
                ),
                "repair the source cluster identity before materializing case membership",
            ));
        }
        cluster.1.insert(part.opinion_id.clone());
        cluster.2.insert(part.opinion_type.clone());
    }
    for (cluster_id, (case_name, opinion_ids, _)) in &clusters {
        let case_id = case_node_id(*cluster_id);
        nodes.insert(case_id, case_node(case_id, *cluster_id, case_name));
        for opinion_id in opinion_ids {
            let part = parts
                .get(opinion_id)
                .ok_or_else(|| CliError::runtime("cluster references an absent opinion part"))?;
            let part_id = opinion_part_node_id(opinion_id);
            nodes.insert(part_id, part_node(part_id, part));
            add_edge(
                &mut edges,
                Edge {
                    src: case_id,
                    dst: part_id,
                    edge_type: CONTAINS_EDGE,
                    weight: 1.0,
                    attribution: "physical_cluster_membership",
                    evidence: vec![EdgeEvidence {
                        source_row_id: format!("membership:{cluster_id}:{opinion_id}"),
                        citing_opinion_id: cluster_id.to_string(),
                        source_target_opinion_id: Some(opinion_id.clone()),
                        evidence_kind: "courtlistener_cluster_id_and_physical_constellation"
                            .to_string(),
                        evidence_sha256: None,
                        source_text_sha256: Some(part.source_text_sha256.clone()),
                    }],
                },
            )?;
        }
    }
    let (citation_rows, coalesced) = append_case_citations(citations, parts, &mut edges)?;
    let dissent_clusters = clusters
        .iter()
        .filter(|(_, (_, _, types))| types.iter().any(|kind| is_dissent(kind)))
        .map(|(cluster_id, _)| *cluster_id)
        .collect::<BTreeSet<_>>();
    let dissent_case_nodes = dissent_clusters
        .iter()
        .map(|cluster_id| case_node_id(*cluster_id))
        .collect::<BTreeSet<_>>();
    let ambiguous = edges
        .values()
        .filter(|edge| edge.edge_type == CITES_CASE_EDGE && dissent_case_nodes.contains(&edge.dst))
        .count();
    let contains = edges
        .values()
        .filter(|edge| edge.edge_type == CONTAINS_EDGE)
        .count();
    let case_edges = edges
        .values()
        .filter(|edge| edge.edge_type == CITES_CASE_EDGE)
        .count();
    Ok(Draft {
        counts: Counts {
            accepted_opinion_rows: accepted_rows,
            case_nodes: clusters.len(),
            opinion_part_nodes: parts.len(),
            contains_opinion_part_edges: contains,
            citation_rows,
            unique_cites_case_edges: case_edges,
            coalesced_cites_case_rows: coalesced,
            clusters_with_dissent: dissent_clusters.len(),
            ambiguous_case_edges_to_dissent_clusters: ambiguous,
            ..Counts::default()
        },
        nodes,
        edges,
    })
}

fn append_case_citations(
    path: &Path,
    parts: &BTreeMap<String, Part>,
    edges: &mut BTreeMap<(CxId, &'static str, CxId), Edge>,
) -> CliResult<(usize, usize)> {
    let file = plain_file(path, "citation CSV")?;
    let mut lines = BufReader::new(file).lines();
    let header = lines
        .next()
        .transpose()
        .map_err(|error| CliError::io(format!("read citation header: {error}")))?
        .ok_or_else(|| CliError::usage("citation CSV is empty"))?;
    let columns = header.split(',').map(str::trim).collect::<Vec<_>>();
    let column = |name: &str| {
        columns
            .iter()
            .position(|value| *value == name)
            .ok_or_else(|| CliError::usage(format!("citation CSV requires {name}")))
    };
    let citing_col = column("citing_opinion_id")?;
    let cited_col = column("cited_opinion_id")?;
    let mut rows = 0;
    let mut coalesced = 0;
    for (offset, line) in lines.enumerate() {
        let row = offset + 2;
        let line =
            line.map_err(|error| CliError::io(format!("read citation row {row}: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        rows += 1;
        let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
        let citing = fields.get(citing_col).copied().unwrap_or("");
        let cited = fields.get(cited_col).copied().unwrap_or("");
        let source = parts
            .get(citing)
            .ok_or_else(|| unresolved_citation(row, "citing", citing))?;
        let target = parts
            .get(cited)
            .ok_or_else(|| unresolved_citation(row, "cited", cited))?;
        let key = (
            opinion_part_node_id(&source.opinion_id),
            CITES_CASE_EDGE,
            case_node_id(target.cluster_id),
        );
        let evidence = EdgeEvidence {
            source_row_id: format!("citation-row-{row}"),
            citing_opinion_id: citing.to_string(),
            source_target_opinion_id: Some(cited.to_string()),
            evidence_kind: "courtlistener_case_citation".to_string(),
            evidence_sha256: None,
            source_text_sha256: Some(source.source_text_sha256.clone()),
        };
        if let Some(edge) = edges.get_mut(&key) {
            edge.evidence.push(evidence);
            coalesced += 1;
        } else {
            edges.insert(
                key,
                Edge {
                    src: key.0,
                    dst: key.2,
                    edge_type: CITES_CASE_EDGE,
                    weight: 1.0,
                    attribution: "case_level_only",
                    evidence: vec![evidence],
                },
            );
        }
    }
    Ok((rows, coalesced))
}

fn append_explicit(
    explicit: &[ExplicitPartRow],
    texts: &BTreeMap<String, String>,
    parts: &BTreeMap<String, Part>,
    draft: &mut Draft,
) -> CliResult {
    let citation_pairs = draft
        .edges
        .values()
        .filter(|edge| edge.edge_type == CITES_CASE_EDGE)
        .flat_map(|edge| {
            edge.evidence.iter().filter_map(|evidence| {
                evidence
                    .source_target_opinion_id
                    .as_ref()
                    .map(|target| (evidence.citing_opinion_id.clone(), target.clone()))
            })
        })
        .collect::<BTreeSet<_>>();
    for row in explicit {
        let source = parts
            .get(&row.citing_opinion_id)
            .ok_or_else(|| explicit_error("citing opinion is absent"))?;
        let case_target = parts
            .get(&row.case_target_opinion_id)
            .ok_or_else(|| explicit_error("case target opinion is absent"))?;
        let part_target = parts
            .get(&row.part_target_opinion_id)
            .ok_or_else(|| explicit_error("part target opinion is absent"))?;
        if case_target.cluster_id != part_target.cluster_id {
            return Err(explicit_error(
                "case target and part target are not siblings in one cluster",
            ));
        }
        if !citation_pairs.contains(&(
            row.citing_opinion_id.clone(),
            row.case_target_opinion_id.clone(),
        )) {
            return Err(explicit_error(
                "explicit row has no underlying case-level citation-map row",
            ));
        }
        validate_attribution(&row.attribution, &part_target.opinion_type)?;
        let text = texts
            .get(&row.citing_opinion_id)
            .ok_or_else(|| explicit_error("citing source text was not retained"))?;
        if sha256(text.as_bytes()) != row.source_text_sha256
            || row.source_text_sha256 != source.source_text_sha256
        {
            return Err(explicit_error(
                "citing source text SHA-256 differs from explicit evidence",
            ));
        }
        if row.evidence_text.trim().is_empty() || !text.contains(&row.evidence_text) {
            return Err(explicit_error(
                "evidence_text is not an exact substring of the citing opinion",
            ));
        }
        if sha256(row.evidence_text.as_bytes()) != row.evidence_sha256 {
            return Err(explicit_error("evidence_text SHA-256 mismatch"));
        }
        add_edge(
            &mut draft.edges,
            Edge {
                src: opinion_part_node_id(&source.opinion_id),
                dst: opinion_part_node_id(&part_target.opinion_id),
                edge_type: CITES_PART_EDGE,
                weight: 1.0,
                attribution: match row.attribution.as_str() {
                    "explicit_dissent" => "explicit_dissent",
                    "explicit_concurrence" => "explicit_concurrence",
                    _ => "explicit_lead",
                },
                evidence: vec![EdgeEvidence {
                    source_row_id: row.source_row_id.clone(),
                    citing_opinion_id: row.citing_opinion_id.clone(),
                    source_target_opinion_id: Some(row.part_target_opinion_id.clone()),
                    evidence_kind: row.attribution.clone(),
                    evidence_sha256: Some(row.evidence_sha256.clone()),
                    source_text_sha256: Some(row.source_text_sha256.clone()),
                }],
            },
        )?;
        match row.attribution.as_str() {
            "explicit_dissent" => draft.counts.explicit_dissent_edges += 1,
            "explicit_concurrence" => draft.counts.explicit_concurrence_edges += 1,
            _ => draft.counts.explicit_lead_edges += 1,
        }
    }
    draft.counts.explicit_part_rows = explicit.len();
    draft.counts.unique_cites_opinion_part_edges = draft
        .edges
        .values()
        .filter(|edge| edge.edge_type == CITES_PART_EDGE)
        .count();
    Ok(())
}

fn validate_typed_edges(draft: &Draft) -> CliResult {
    for edge in draft.edges.values() {
        let source_type = draft.nodes.get(&edge.src).map(|node| node.node_type);
        let target_type = draft.nodes.get(&edge.dst).map(|node| node.node_type);
        let valid = match edge.edge_type {
            CONTAINS_EDGE => {
                source_type == Some(CASE_NODE_TYPE) && target_type == Some(PART_NODE_TYPE)
            }
            CITES_CASE_EDGE => {
                source_type == Some(PART_NODE_TYPE) && target_type == Some(CASE_NODE_TYPE)
            }
            CITES_PART_EDGE => {
                source_type == Some(PART_NODE_TYPE)
                    && target_type == Some(PART_NODE_TYPE)
                    && !edge.evidence.is_empty()
            }
            _ => false,
        };
        if !valid {
            return Err(contract_error(
                "CALYX_CITATION_ATTRIBUTION_TYPE_INVARIANT",
                format!(
                    "edge {} -{}-> {} has source type {:?} and target type {:?}",
                    edge.src, edge.edge_type, edge.dst, source_type, target_type
                ),
                "repair the typed case/opinion-part graph; never propagate a case citation through membership",
            ));
        }
    }
    Ok(())
}

fn load_aliases(path: &Path) -> CliResult<BTreeMap<String, CxId>> {
    let file = plain_file(path, "opinion alias CSV")?;
    let mut lines = BufReader::new(file).lines();
    let header = lines
        .next()
        .transpose()
        .map_err(|error| CliError::io(format!("read alias header: {error}")))?
        .ok_or_else(|| CliError::usage("opinion alias CSV is empty"))?;
    if header.trim_end_matches('\r')
        != "opinion_id,cx_id,canonical_opinion_id,content_sha256,is_canonical,source_url"
    {
        return Err(CliError::usage("opinion alias CSV has the wrong schema"));
    }
    let mut aliases = BTreeMap::new();
    for (offset, line) in lines.enumerate() {
        let row = offset + 2;
        let line = line.map_err(|error| CliError::io(format!("read alias row {row}: {error}")))?;
        let fields = line.trim_end_matches('\r').split(',').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(CliError::usage(format!(
                "alias row {row} has {} fields",
                fields.len()
            )));
        }
        let cx = fields[1]
            .parse::<CxId>()
            .map_err(|error| CliError::usage(format!("alias row {row} invalid cx_id: {error}")))?;
        if aliases.insert(fields[0].to_string(), cx).is_some() {
            return Err(CliError::usage(format!(
                "alias row {row} duplicates opinion {}",
                fields[0]
            )));
        }
    }
    if aliases.is_empty() {
        return Err(CliError::usage("opinion alias CSV is empty"));
    }
    Ok(aliases)
}

fn load_parts(
    path: &Path,
    aliases: &BTreeMap<String, CxId>,
    explicit: &[ExplicitPartRow],
) -> CliResult<LoadedParts> {
    let wanted_text = explicit
        .iter()
        .map(|row| row.citing_opinion_id.clone())
        .collect::<BTreeSet<_>>();
    let file = plain_file(path, "opinion JSONL")?;
    let mut parts = BTreeMap::new();
    let mut texts = BTreeMap::new();
    let mut accepted = 0;
    for (offset, line) in BufReader::new(file).lines().enumerate() {
        let row_number = offset + 1;
        let line =
            line.map_err(|error| CliError::io(format!("read opinion row {row_number}: {error}")))?;
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            CliError::usage(format!("decode opinion row {row_number}: {error}"))
        })?;
        if value.pointer("/selection/status").and_then(Value::as_str) != Some("accepted") {
            continue;
        }
        accepted += 1;
        let opinion_id = required_u64(&value, "opinion_id", row_number)?.to_string();
        let cluster_id = required_u64(&value, "cluster_id", row_number)?;
        let opinion_type = required_str(&value, "opinion_type", row_number)?.to_string();
        let case_name = required_str(&value, "case_name", row_number)?.to_string();
        let text = required_str(&value, "text", row_number)?;
        let cx_id = *aliases.get(&opinion_id).ok_or_else(|| {
            contract_error(
                "CALYX_CITATION_PART_ALIAS_MISSING",
                format!("accepted opinion {opinion_id} has no physical alias row"),
                "rebuild the exact accepted opinion/alias generation before attribution",
            )
        })?;
        let source_text_sha256 = sha256(text.as_bytes());
        if wanted_text.contains(&opinion_id) {
            texts.insert(opinion_id.clone(), text.to_string());
        }
        if parts
            .insert(
                opinion_id.clone(),
                Part {
                    opinion_id,
                    cluster_id,
                    opinion_type,
                    case_name,
                    cx_id,
                    source_text_sha256,
                },
            )
            .is_some()
        {
            return Err(CliError::usage(format!(
                "opinion JSONL duplicates opinion at row {row_number}"
            )));
        }
    }
    if parts.len() != aliases.len() {
        return Err(contract_error(
            "CALYX_CITATION_PART_ROWCOUNT_MISMATCH",
            format!(
                "accepted opinions={} aliases={}",
                parts.len(),
                aliases.len()
            ),
            "use the exact accepted extraction and physical alias generation",
        ));
    }
    Ok((parts, texts, accepted))
}

fn load_explicit(path: &Path) -> CliResult<Vec<ExplicitPartRow>> {
    let file = plain_file(path, "explicit part JSONL")?;
    let mut rows = Vec::new();
    let mut ids = BTreeSet::new();
    for (offset, line) in BufReader::new(file).lines().enumerate() {
        let row_number = offset + 1;
        let line =
            line.map_err(|error| CliError::io(format!("read explicit row {row_number}: {error}")))?;
        if line.trim().is_empty() {
            return Err(CliError::usage(format!(
                "explicit row {row_number} is blank"
            )));
        }
        let row: ExplicitPartRow = serde_json::from_str(&line).map_err(|error| {
            CliError::usage(format!("decode explicit row {row_number}: {error}"))
        })?;
        if !ids.insert(row.source_row_id.clone()) {
            return Err(CliError::usage(format!(
                "duplicate explicit source_row_id {}",
                row.source_row_id
            )));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn traction(args: TractionArgs) -> CliResult {
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let resolved = super::vault::resolve_vault_info(&home, &args.vault)?;
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &args.collection)?;
    let mut target = None;
    for (id, bytes) in physical.node_props()? {
        let props: AsterAssocNodeProps = serde_json::from_slice(&bytes)
            .map_err(|error| CliError::runtime(format!("decode attribution node {id}: {error}")))?;
        if props.metadata.get("node_type").map(String::as_str) == Some(PART_NODE_TYPE)
            && props
                .metadata
                .get("opinion_ids")
                .is_some_and(|ids| ids.split('|').any(|value| value == args.opinion_id))
            && target.replace((id, props)).is_some()
        {
            return Err(CliError::runtime(
                "opinion id resolves to multiple attribution nodes",
            ));
        }
    }
    let (part_id, props) = target.ok_or_else(|| {
        contract_error(
            "CALYX_CITATION_PART_NOT_FOUND",
            format!(
                "opinion {} is absent from attribution collection",
                args.opinion_id
            ),
            "use a physically retained opinion-part id from the accepted attribution generation",
        )
    })?;
    let cluster_id = props
        .metadata
        .get("cluster_id")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| CliError::runtime("opinion-part node has invalid cluster_id"))?;
    let opinion_type = props
        .metadata
        .get("opinion_types")
        .cloned()
        .unwrap_or_default();
    let physical_constellation_id = props
        .metadata
        .get("physical_constellation_id")
        .cloned()
        .ok_or_else(|| CliError::runtime("opinion-part node lacks physical constellation id"))?;
    let case_id = case_node_id(cluster_id);
    let mut explicit_edges = Vec::new();
    let mut ambiguous = 0;
    let mut membership_edges = Vec::new();
    for edge in physical.edge_out_props()? {
        if edge.edge_type == CITES_PART_EDGE && edge.dst == part_id {
            let value: Value = serde_json::from_slice(&edge.value).map_err(|error| {
                CliError::runtime(format!("decode explicit attribution edge: {error}"))
            })?;
            explicit_edges.push(json!({"src":edge.src,"dst":edge.dst,"value":value}));
        } else if edge.edge_type == CITES_CASE_EDGE && edge.dst == case_id {
            ambiguous += 1;
        } else if edge.edge_type == CONTAINS_EDGE && edge.src == case_id && edge.dst == part_id {
            let value: Value = serde_json::from_slice(&edge.value).map_err(|error| {
                CliError::runtime(format!("decode opinion-part membership edge: {error}"))
            })?;
            membership_edges.push(json!({"src":edge.src,"dst":edge.dst,"value":value}));
        }
    }
    if membership_edges.len() != 1 {
        return Err(contract_error(
            "CALYX_CITATION_PART_MEMBERSHIP_INVALID",
            format!(
                "opinion {} has {} physical contains_opinion_part edges",
                args.opinion_id,
                membership_edges.len()
            ),
            "rebuild the typed collection and require exactly one physical case-to-part membership edge",
        ));
    }
    let report = TractionReport {
        status: if explicit_edges.is_empty() {
            "insufficient_part_specific_evidence"
        } else {
            "complete"
        },
        source_of_truth: "physical Aster Graph CF typed edge and node bytes",
        vault: resolved.name,
        collection: args.collection,
        opinion_id: args.opinion_id,
        opinion_part_node: part_id.to_string(),
        physical_constellation_id,
        opinion_type,
        cluster_id,
        case_node: case_id.to_string(),
        membership_edge: membership_edges.remove(0),
        explicit_part_citations: explicit_edges.len(),
        ambiguous_case_citations: ambiguous,
        attribution_rule: "cites_case never propagates through contains_opinion_part; only cites_opinion_part counts as part traction",
        explicit_edges,
    };
    print_json(&report)?;
    if report.explicit_part_citations == 0 {
        return Err(contract_error(
            "CALYX_CITATION_PART_ATTRIBUTION_INSUFFICIENT",
            format!(
                "opinion {} has {} case-level citations but zero explicit part citations",
                report.opinion_id, report.ambiguous_case_citations
            ),
            "label the traction deficit or add source-text-bound explicit evidence; never smear case citations across sibling parts",
        ));
    }
    Ok(())
}

fn add_edge(edges: &mut BTreeMap<(CxId, &'static str, CxId), Edge>, edge: Edge) -> CliResult {
    let key = (edge.src, edge.edge_type, edge.dst);
    if let Some(existing) = edges.get_mut(&key) {
        if existing.attribution != edge.attribution {
            return Err(CliError::runtime(
                "citation attribution edge collision changes attribution class",
            ));
        }
        existing.evidence.extend(edge.evidence);
    } else {
        edges.insert(key, edge);
    }
    Ok(())
}

fn case_node(id: CxId, cluster_id: u64, case_name: &str) -> Node {
    Node {
        id,
        node_type: CASE_NODE_TYPE,
        metadata: BTreeMap::from([
            ("cluster_id".to_string(), cluster_id.to_string()),
            ("case_name".to_string(), case_name.to_string()),
        ]),
    }
}

fn part_node(id: CxId, part: &Part) -> Node {
    Node {
        id,
        node_type: PART_NODE_TYPE,
        metadata: BTreeMap::from([
            ("cluster_id".to_string(), part.cluster_id.to_string()),
            ("opinion_ids".to_string(), part.opinion_id.clone()),
            ("opinion_types".to_string(), part.opinion_type.clone()),
            (
                "physical_constellation_id".to_string(),
                part.cx_id.to_string(),
            ),
            (
                "source_text_sha256".to_string(),
                part.source_text_sha256.clone(),
            ),
        ]),
    }
}

pub(crate) fn case_node_id(cluster_id: u64) -> CxId {
    let cluster_id = cluster_id.to_string();
    CxId::from_bytes(content_address([
        b"calyx-legal-case-node-v1".as_slice(),
        cluster_id.as_bytes(),
    ]))
}

fn opinion_part_node_id(opinion_id: &str) -> CxId {
    CxId::from_bytes(content_address([
        b"calyx-legal-opinion-part-node-v1".as_slice(),
        opinion_id.as_bytes(),
    ]))
}

fn validate_attribution(attribution: &str, opinion_type: &str) -> CliResult {
    let valid = match attribution {
        "explicit_dissent" => is_dissent(opinion_type),
        "explicit_concurrence" => opinion_type.to_ascii_lowercase().contains("concurr"),
        "explicit_lead" => {
            let kind = opinion_type.to_ascii_lowercase();
            kind.contains("lead") || kind.contains("combined")
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(explicit_error(
            "attribution label does not match target opinion_type",
        ))
    }
}

fn is_dissent(value: &str) -> bool {
    value.to_ascii_lowercase().contains("dissent")
}

fn required_u64(value: &Value, field: &str, row: usize) -> CliResult<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::usage(format!("opinion row {row} requires positive {field}")))
}

fn required_str<'a>(value: &'a Value, field: &str, row: usize) -> CliResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::usage(format!("opinion row {row} requires nonempty {field}")))
}

fn plain_file(path: &Path, label: &str) -> CliResult<fs::File> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| CliError::io(format!("inspect {label} {}: {error}", path.display())))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CliError::usage(format!(
            "{label} {} is not a plain file",
            path.display()
        )));
    }
    fs::File::open(path)
        .map_err(|error| CliError::io(format!("open {label} {}: {error}", path.display())))
}

fn require_new_output(path: &Path) -> CliResult {
    if fs::symlink_metadata(path).is_ok() {
        return Err(CliError::usage(format!(
            "report {} already exists; evidence is immutable",
            path.display()
        )));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::io(format!(
                "create report parent {}: {error}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

pub(super) fn write_report(path: &Path, value: &impl Serialize, label: &str) -> CliResult {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {label}: {error}")))?;
    bytes.push(b'\n');
    write_bytes_atomic_new(path, &bytes, label)?;
    let readback =
        fs::read(path).map_err(|error| CliError::io(format!("read back {label}: {error}")))?;
    if readback != bytes {
        return Err(CliError::runtime(format!("{label} byte readback mismatch")));
    }
    Ok(())
}

fn unresolved_citation(row: usize, endpoint: &str, opinion_id: &str) -> CliError {
    contract_error(
        "CALYX_CITATION_CASE_ENDPOINT_UNRESOLVED",
        format!("citation row {row} {endpoint} opinion {opinion_id} is absent"),
        "rebuild from the exact accepted opinion and alias generations; never silently skip a case endpoint",
    )
}

fn explicit_error(message: &'static str) -> CliError {
    contract_error(
        "CALYX_CITATION_PART_EVIDENCE_INVALID",
        message,
        "bind the exact citing text, underlying case citation, sibling cluster, target type, and hashes; never infer a part from case membership",
    )
}

fn contract_error(
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

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn source_contract(role: &str, path: &Path) -> CliResult<Value> {
    let mut file = plain_file(path, role)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    use std::io::Read;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| CliError::io(format!("hash {role}: {error}")))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        bytes += u64::try_from(count).unwrap_or(u64::MAX);
    }
    let digest = hasher.finalize();
    Ok(
        json!({"role":role,"path":path.display().to_string(),"bytes":bytes,"sha256":format!("{digest:x}")}),
    )
}
