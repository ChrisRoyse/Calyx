//! Vault-scoped legal-corpus readback verbs backing the research-chat tool
//! registry (issues #1962 / #1961).
//!
//! Each verb is a DB-native, read-only projection over a Calyx legal vault. They
//! close the live-CLI capability gaps the DeepSeek research-chat agent surfaced:
//!
//! - `readback citation-neighbors` — per-opinion citation fan-in/fan-out from the
//!   plain-graph Graph CF edge rows.
//! - `readback opinion-type-census` — server-side aggregation of `opinion_type`
//!   over Base metadata (majority / dissent / concurrence family totals).
//! - `cite-check` — DB-native case-name existence probe returning `found` cx_ids
//!   or `NOT_IN_CORPUS` as a first-class typed result (never an error).
//! - `readback case-text` — decoded retained opinion text (bounded window).
//! - `readback judge-census` — authored/panel opinion counts for an arbitrary
//!   judge over Base author metadata, with honest resolution-coverage reporting.
//!
//! Design: every verb splits a PURE parse (`parse_*_args`, vault-free, argv-only)
//! from a vault-touching `run_*`. The aggregation/projection cores are likewise
//! pure so they can be unit-tested on any OS with crafted, known-outcome data —
//! the real-vault FSV path only runs on the owner-only-FS (Unix) hosts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraphCsr, PlainGraphEdge};
use calyx_aster::retained_input::input_from_ref;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{CalyxError, CxId};
use serde_json::{Value, json};

use crate::cf_read::{hex_bytes, latest_cf_row, latest_cf_rows};
use crate::error::{CliError, CliResult};
use crate::output::print_line;

/// Default typed citation-graph collection (matches the chat `citation_graph`
/// default and the production `legal-citations-v2` overlay generation).
const DEFAULT_CITATION_COLLECTION: &str = "legal-citations-v2";
/// Default case-text window: a generous single-quote window without dumping a
/// whole long opinion into the model context.
const DEFAULT_CASE_TEXT_MAX_BYTES: usize = 32 * 1024;
/// Default citation-neighbor cap so a hub opinion cannot flood the caller.
const DEFAULT_NEIGHBOR_LIMIT: usize = 256;

/// Opinion-type codes with dedicated census aggregates (CourtListener vocab).
const DISSENT_TYPE: &str = "040dissent";
const CONCURRENCE_TYPE: &str = "030concurrence";
const CONCURRENCE_IN_PART_TYPE: &str = "035concurrenceinpart";

// ======================================================================
// shared arg helpers
// ======================================================================

fn value_of<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_cx(raw: &str) -> CliResult<CxId> {
    CxId::from_str(raw).map_err(|error| CliError::usage(format!("invalid --cx-id: {error}")))
}

fn parse_usize(raw: &str, flag: &str) -> CliResult<usize> {
    raw.parse::<usize>()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))
}

/// Harden the vault tree before any direct CF read (mirrors `readback cx-list`).
/// On an owner-only-FS host this migrates world-accessible trees to 0700/0600 and
/// fails closed on a symlink swap; on a non-Unix host it fails closed with
/// `CALYX_VAULT_PERMISSIONS_UNSAFE` (these verbs run on the Linux FSV hosts).
fn harden(vault: &Path) -> CliResult {
    calyx_aster::private_fs::harden_tree(vault)?;
    Ok(())
}

// ======================================================================
// 1. readback citation-neighbors
// ======================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    In,
    Out,
    Both,
}

impl Direction {
    fn parse(raw: &str) -> CliResult<Self> {
        match raw {
            "in" => Ok(Self::In),
            "out" => Ok(Self::Out),
            "both" => Ok(Self::Both),
            other => Err(CliError::usage(format!(
                "--direction must be in|out|both, got {other:?}"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Both => "both",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CitationNeighborsArgs {
    vault: PathBuf,
    cx_id: CxId,
    direction: Direction,
    collection: String,
    limit: usize,
}

pub(crate) fn parse_citation_neighbors_args(rest: &[String]) -> CliResult<CitationNeighborsArgs> {
    let mut vault = None;
    let mut cx_id = None;
    let mut direction = None;
    let mut collection = DEFAULT_CITATION_COLLECTION.to_string();
    let mut limit = DEFAULT_NEIGHBOR_LIMIT;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value_of(rest, idx, "--vault")?));
            }
            "--cx-id" => {
                idx += 1;
                cx_id = Some(parse_cx(value_of(rest, idx, "--cx-id")?)?);
            }
            "--direction" => {
                idx += 1;
                direction = Some(Direction::parse(value_of(rest, idx, "--direction")?)?);
            }
            "--collection" => {
                idx += 1;
                collection = value_of(rest, idx, "--collection")?.to_string();
            }
            "--limit" => {
                idx += 1;
                limit = parse_usize(value_of(rest, idx, "--limit")?, "--limit")?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected citation-neighbors flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(CitationNeighborsArgs {
        vault: vault.ok_or_else(|| CliError::usage("citation-neighbors requires --vault <dir>"))?,
        cx_id: cx_id.ok_or_else(|| CliError::usage("citation-neighbors requires --cx-id <cx>"))?,
        direction: direction.ok_or_else(|| {
            CliError::usage("citation-neighbors requires --direction in|out|both")
        })?,
        collection,
        limit,
    })
}

pub(crate) fn readback_citation_neighbors_args(rest: &[String]) -> CliResult {
    let args = parse_citation_neighbors_args(rest)?;
    harden(&args.vault)?;
    // Resolve the logical collection through the physically read-back accepted
    // lifecycle state. A generation under construction is never a citation
    // source, and callers cannot bypass publication by naming its storage key.
    // The committed CSR is the bounded topology source; exact edge values are
    // independently read from the routed physical Graph CF below.
    let physical = PhysicalPlainGraph::open_latest_isolated(&args.vault, &args.collection)?;
    let storage_collection = physical.storage_collection();
    let csr = physical.read_csr()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_GRAPH_CSR_REQUIRED",
            message: format!(
                "citation-neighbors requires a committed CSR for collection={} in {}",
                args.collection,
                args.vault.display()
            ),
            remediation: "materialize and physically verify the collection CSR, then retry; a row-level whole-graph scan is intentionally not available",
        })
    })?;
    let edges_scanned = csr.edges.len();
    let descriptors = collect_csr_neighbor_descriptors(&csr, args.cx_id, args.direction)?;
    let matched = descriptors.len();
    let mut neighbors = Vec::with_capacity(args.limit.min(matched));
    for descriptor in descriptors.into_iter().take(args.limit) {
        let value = physical
            .get_edge(descriptor.src, &descriptor.edge_type, descriptor.dst)?
            .ok_or_else(|| {
                CliError::from(CalyxError {
                    code: "CALYX_GRAPH_CSR_EDGE_MISSING",
                    message: format!(
                        "CSR edge {} -[{}]-> {} has no live physical Graph CF value in collection={}",
                        descriptor.src, descriptor.edge_type, descriptor.dst, args.collection
                    ),
                    remediation: "preserve the vault and rebuild the CSR from physically verified graph rows; do not serve a partial citation result",
                })
            })?;
        let edge = PlainGraphEdge {
            src: descriptor.src,
            dst: descriptor.dst,
            edge_type: descriptor.edge_type,
            value,
        };
        neighbors.push(neighbor_edge_json(
            &edge,
            descriptor.neighbor,
            descriptor.direction,
        ));
    }
    let doc = json!({
        "vault": args.vault.display().to_string(),
        "cx_id": args.cx_id.to_string(),
        "collection": args.collection,
        "storage_collection": storage_collection,
        "direction": args.direction.as_str(),
        "edges_scanned": edges_scanned,
        "csr_source_snapshot": csr.source_snapshot,
        "matched": matched,
        "returned": neighbors.len(),
        "truncated": matched > neighbors.len(),
        "limit": args.limit,
        "neighbors": neighbors,
    });
    print_line(&serialize(&doc, "citation-neighbors")?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CsrNeighborDescriptor {
    src: CxId,
    dst: CxId,
    edge_type: String,
    neighbor: CxId,
    direction: &'static str,
}

fn collect_csr_neighbor_descriptors(
    csr: &PlainGraphCsr,
    cx: CxId,
    direction: Direction,
) -> CliResult<Vec<CsrNeighborDescriptor>> {
    if csr.offsets.len() != csr.nodes.len().saturating_add(1)
        || csr.offsets.first().copied() != Some(0)
        || csr.offsets.last().copied() != Some(csr.edges.len())
        || csr.offsets.windows(2).any(|window| window[0] > window[1])
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_GRAPH_CSR_SHAPE_INVALID",
            message: format!(
                "citation CSR shape is invalid: nodes={} offsets={} edges={}",
                csr.nodes.len(),
                csr.offsets.len(),
                csr.edges.len()
            ),
            remediation: "preserve the vault and rebuild the CSR from verified graph rows before retrying",
        }));
    }
    let mut matched = Vec::new();
    for (source_index, src) in csr.nodes.iter().copied().enumerate() {
        let start = csr.offsets[source_index];
        let end = csr.offsets[source_index + 1];
        for edge in &csr.edges[start..end] {
            let out_hit = src == cx;
            let in_hit = edge.dst == cx;
            let neighbor_direction = match direction {
                Direction::Out if out_hit => "out",
                Direction::In if in_hit => "in",
                Direction::Both if out_hit => "out",
                Direction::Both if in_hit => "in",
                _ => continue,
            };
            let neighbor = if neighbor_direction == "out" {
                edge.dst
            } else {
                src
            };
            matched.push(CsrNeighborDescriptor {
                src,
                dst: edge.dst,
                edge_type: edge.edge_type.clone(),
                neighbor,
                direction: neighbor_direction,
            });
        }
    }
    matched.sort_by(|left, right| {
        left.edge_type
            .cmp(&right.edge_type)
            .then_with(|| left.neighbor.to_string().cmp(&right.neighbor.to_string()))
            .then_with(|| left.direction.cmp(right.direction))
            .then_with(|| left.src.cmp(&right.src))
            .then_with(|| left.dst.cmp(&right.dst))
    });
    Ok(matched)
}

/// Pure projection: decode the plain-graph out-edge rows into a deterministic,
/// direction-filtered neighbor list. Out-edges of a node are rows where `src`
/// equals the node; in-edges are rows where `dst` equals the node (the forward
/// out-edge rows carry the payload, so scanning out-edges also serves in-edges
/// without dereferencing the reverse index).
#[cfg(test)]
pub(crate) fn collect_neighbors(
    edges: &[PlainGraphEdge],
    cx: CxId,
    direction: Direction,
    limit: usize,
) -> (Vec<Value>, usize) {
    let mut matched: Vec<(String, String, Value)> = Vec::new();
    for edge in edges {
        let out_hit = edge.src == cx;
        let in_hit = edge.dst == cx;
        let neighbor_dir = match direction {
            Direction::Out if out_hit => "out",
            Direction::In if in_hit => "in",
            Direction::Both if out_hit => "out",
            Direction::Both if in_hit => "in",
            _ => continue,
        };
        // For a self-loop under Both, prefer the "out" facing (already chosen).
        let neighbor = if neighbor_dir == "out" {
            edge.dst
        } else {
            edge.src
        };
        matched.push((
            edge.edge_type.clone(),
            neighbor.to_string(),
            neighbor_edge_json(edge, neighbor, neighbor_dir),
        ));
    }
    // Deterministic order: edge_type, then neighbor cx_id hex.
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let total = matched.len();
    let neighbors = matched
        .into_iter()
        .take(limit)
        .map(|(_, _, value)| value)
        .collect::<Vec<_>>();
    (neighbors, total)
}

fn neighbor_edge_json(edge: &PlainGraphEdge, neighbor: CxId, direction: &str) -> Value {
    // The edge VALUE is arbitrary JSON (citation overlay writes weight/depth/
    // opinion-id fields). Surface the parsed value verbatim so fused rank/lanes/
    // depth/opinion ids flow through when present; fall back to raw hex bytes.
    let (edge_value, weight) = match serde_json::from_slice::<Value>(&edge.value) {
        Ok(parsed) => {
            let weight = parsed.get("weight").and_then(Value::as_f64);
            (parsed, weight)
        }
        Err(_) => (json!({ "raw_hex": hex_bytes(&edge.value) }), None),
    };
    json!({
        "cx_id": neighbor.to_string(),
        "direction": direction,
        "edge_type": edge.edge_type,
        "weight": weight,
        "edge_value": edge_value,
    })
}

// ======================================================================
// 2. readback opinion-type-census
// ======================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SingleVaultArgs {
    vault: PathBuf,
}

fn parse_single_vault_args(rest: &[String], verb: &str) -> CliResult<SingleVaultArgs> {
    let mut vault = None;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value_of(rest, idx, "--vault")?));
            }
            other => return Err(CliError::usage(format!("unexpected {verb} flag {other}"))),
        }
        idx += 1;
    }
    Ok(SingleVaultArgs {
        vault: vault.ok_or_else(|| CliError::usage(format!("{verb} requires --vault <dir>")))?,
    })
}

pub(crate) fn parse_opinion_type_census_args(rest: &[String]) -> CliResult<SingleVaultArgs> {
    parse_single_vault_args(rest, "opinion-type-census")
}

/// Bounded aggregation state over Base metadata `opinion_type` values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OpinionTypeCensus {
    total: usize,
    tombstoned: usize,
    missing_type: usize,
    by_type: BTreeMap<String, usize>,
}

impl OpinionTypeCensus {
    fn observe_tombstone(&mut self) {
        self.tombstoned += 1;
    }

    fn observe_live(&mut self, metadata: &BTreeMap<String, String>) {
        self.total += 1;
        match metadata.get("opinion_type") {
            Some(t) if !t.trim().is_empty() => {
                *self.by_type.entry(t.clone()).or_default() += 1;
            }
            _ => self.missing_type += 1,
        }
    }

    fn count(&self, t: &str) -> usize {
        self.by_type.get(t).copied().unwrap_or(0)
    }

    fn to_json(&self, vault: &Path) -> Value {
        json!({
            "vault": vault.display().to_string(),
            "physical_opinions_n": self.total,
            "tombstoned_n": self.tombstoned,
            "missing_opinion_type_n": self.missing_type,
            "by_type": self.by_type,
            "dissents_n": self.count(DISSENT_TYPE),
            "concurrence_n": self.count(CONCURRENCE_TYPE),
            "concurrence_in_part_n": self.count(CONCURRENCE_IN_PART_TYPE),
            "concurrence_family_n": self.count(CONCURRENCE_TYPE) + self.count(CONCURRENCE_IN_PART_TYPE),
            "concurrence_family_types": [CONCURRENCE_TYPE, CONCURRENCE_IN_PART_TYPE],
            "aggregation": "server-side page fold over Base metadata.opinion_type (tombstones excluded from physical_opinions_n)",
        })
    }
}

pub(crate) fn readback_opinion_type_census_args(rest: &[String]) -> CliResult {
    let args = parse_opinion_type_census_args(rest)?;
    harden(&args.vault)?;
    let mut census = OpinionTypeCensus::default();
    scan_base_metadata(&args.vault, |row| {
        match row {
            BaseRow::Tombstone => census.observe_tombstone(),
            BaseRow::Live { metadata, .. } => census.observe_live(metadata),
        }
        Ok(())
    })?;
    print_line(&serialize(
        &census.to_json(&args.vault),
        "opinion-type-census",
    )?)
}

// ======================================================================
// 3. cite-check
// ======================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CiteCheckArgs {
    vault: PathBuf,
    case_name: String,
    proposition: Option<String>,
}

pub(crate) fn parse_cite_check_args(rest: &[String]) -> CliResult<CiteCheckArgs> {
    let mut vault = None;
    let mut case_name = None;
    let mut proposition = None;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value_of(rest, idx, "--vault")?));
            }
            "--case-name" => {
                idx += 1;
                case_name = Some(value_of(rest, idx, "--case-name")?.to_string());
            }
            "--proposition" => {
                idx += 1;
                proposition = Some(value_of(rest, idx, "--proposition")?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected cite-check flag {other}"
                )));
            }
        }
        idx += 1;
    }
    let case_name =
        case_name.ok_or_else(|| CliError::usage("cite-check requires --case-name <name>"))?;
    if case_name.trim().is_empty() {
        return Err(CliError::usage("cite-check --case-name must not be empty"));
    }
    Ok(CiteCheckArgs {
        vault: vault.ok_or_else(|| CliError::usage("cite-check requires --vault <dir>"))?,
        case_name,
        proposition,
    })
}

/// Case-name normalization for the cite-check match rule: lowercase, strip
/// surrounding quotes, collapse internal whitespace runs to a single space, and
/// trim. Documented in the command output so the match is auditable.
pub(crate) fn normalize_case_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.extend(ch.to_lowercase());
            prev_space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CiteMatch {
    cx_id: String,
    case_name: Option<String>,
    case_name_full: Option<String>,
    kind: &'static str,
}

/// Pure existence scan: exact-or-normalized-substring over `case_name` and
/// `case_name_full`. Exact wins over substring for the reported `match_kind`.
pub(crate) struct CiteChecker {
    query_norm: String,
    matches: Vec<CiteMatch>,
}

impl CiteChecker {
    pub(crate) fn new(query: &str) -> Self {
        Self {
            query_norm: normalize_case_name(query),
            matches: Vec::new(),
        }
    }

    pub(crate) fn observe(&mut self, cx: CxId, metadata: &BTreeMap<String, String>) {
        let case_name = metadata.get("case_name").map(String::as_str);
        let case_name_full = metadata.get("case_name_full").map(String::as_str);
        let mut kind: Option<&'static str> = None;
        for candidate in [case_name, case_name_full].into_iter().flatten() {
            let cand_norm = normalize_case_name(candidate);
            if cand_norm.is_empty() || self.query_norm.is_empty() {
                continue;
            }
            if cand_norm == self.query_norm {
                kind = Some("exact");
                break;
            }
            if cand_norm.contains(&self.query_norm) {
                kind = Some("contains");
            }
        }
        if let Some(kind) = kind {
            self.matches.push(CiteMatch {
                cx_id: cx.to_string(),
                case_name: case_name.map(str::to_string),
                case_name_full: case_name_full.map(str::to_string),
                kind,
            });
        }
    }

    fn to_json(&self, args: &CiteCheckArgs) -> Value {
        let status = if self.matches.is_empty() {
            "NOT_IN_CORPUS"
        } else {
            "found"
        };
        let found: Vec<Value> = self
            .matches
            .iter()
            .map(|m| {
                json!({
                    "cx_id": m.cx_id,
                    "case_name": m.case_name,
                    "case_name_full": m.case_name_full,
                    "match_kind": m.kind,
                })
            })
            .collect();
        let found_cx_ids: Vec<&str> = self.matches.iter().map(|m| m.cx_id.as_str()).collect();
        let proposition = args.proposition.as_ref().map(|p| {
            json!({
                "text": p,
                "status": "not_evaluated",
                "note": "cite-check is a DB-native case-name existence probe; proposition support requires the resident search path",
                "remediation": "use corpus_search / kernel_answer against a matched cx_id to test the proposition",
            })
        });
        json!({
            "vault": args.vault.display().to_string(),
            "query_case_name": args.case_name,
            "normalized_query": self.query_norm,
            "status": status,
            "match_rule": "exact-or-normalized-substring over metadata.case_name and metadata.case_name_full (lowercased, surrounding quotes stripped, internal whitespace collapsed)",
            "match_count": self.matches.len(),
            "found": found,
            "found_cx_ids": found_cx_ids,
            "proposition": proposition,
        })
    }
}

pub(crate) fn cite_check_args(rest: &[String]) -> CliResult {
    let args = parse_cite_check_args(rest)?;
    harden(&args.vault)?;
    let mut checker = CiteChecker::new(&args.case_name);
    scan_base_metadata(&args.vault, |row| {
        if let BaseRow::Live { cx_id, metadata } = row {
            checker.observe(cx_id, metadata);
        }
        Ok(())
    })?;
    print_line(&serialize(&checker.to_json(&args), "cite-check")?)
}

// ======================================================================
// 4. readback case-text
// ======================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaseTextArgs {
    vault: PathBuf,
    cx_id: CxId,
    max_bytes: usize,
    offset: usize,
}

pub(crate) fn parse_case_text_args(rest: &[String]) -> CliResult<CaseTextArgs> {
    let mut vault = None;
    let mut cx_id = None;
    let mut max_bytes = DEFAULT_CASE_TEXT_MAX_BYTES;
    let mut offset = 0usize;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value_of(rest, idx, "--vault")?));
            }
            "--cx-id" => {
                idx += 1;
                cx_id = Some(parse_cx(value_of(rest, idx, "--cx-id")?)?);
            }
            "--max-bytes" => {
                idx += 1;
                max_bytes = parse_usize(value_of(rest, idx, "--max-bytes")?, "--max-bytes")?;
            }
            "--offset" => {
                idx += 1;
                offset = parse_usize(value_of(rest, idx, "--offset")?, "--offset")?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected case-text flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if max_bytes == 0 {
        return Err(CliError::usage("case-text --max-bytes must be > 0"));
    }
    Ok(CaseTextArgs {
        vault: vault.ok_or_else(|| CliError::usage("case-text requires --vault <dir>"))?,
        cx_id: cx_id.ok_or_else(|| CliError::usage("case-text requires --cx-id <cx>"))?,
        max_bytes,
        offset,
    })
}

/// Char-boundary-safe UTF-8 window over decoded opinion text. Returns the window
/// text plus `(total_bytes, returned_bytes, truncated)`. Snapping to char
/// boundaries keeps a byte offset/window from splitting a multibyte codepoint.
pub(crate) fn text_window(
    text: &str,
    offset: usize,
    max_bytes: usize,
) -> (String, usize, usize, bool) {
    let total = text.len();
    if offset >= total {
        return (String::new(), total, 0, false);
    }
    let start = floor_char_boundary(text, offset);
    let raw_end = start.saturating_add(max_bytes).min(total);
    let mut end = floor_char_boundary(text, raw_end);
    if end <= start {
        // The window is smaller than one codepoint; return that whole codepoint.
        end = next_char_boundary(text, start);
    }
    let window = &text[start..end];
    let truncated = end < total;
    (window.to_string(), total, window.len(), truncated)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    index += 1;
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub(crate) fn readback_case_text_args(rest: &[String]) -> CliResult {
    let args = parse_case_text_args(rest)?;
    harden(&args.vault)?;
    let key = base_key(args.cx_id);
    let value = latest_cf_row(&args.vault, ColumnFamily::Base, &key)?.ok_or_else(|| {
        CliError::usage(format!(
            "case-text --cx-id {} was not found in {}",
            args.cx_id,
            args.vault.display()
        ))
    })?;
    if is_tombstone_value(&value) {
        return Err(CliError::usage(format!(
            "case-text --cx-id {} is a tombstoned (erased) constellation",
            args.cx_id
        )));
    }
    let cx = decode_constellation_base(&value)?;
    // Follow the retained input reference to the vault-owned blob and verify its
    // bytes against the stored hash (fails closed on redacted / missing / mismatch).
    let input = input_from_ref(&args.vault, cx.modality, &cx.input_ref)?;
    let text = String::from_utf8(input.bytes).map_err(|error| {
        CliError::runtime(format!(
            "retained input for cx {} is not valid UTF-8: {error}",
            args.cx_id
        ))
    })?;
    let (window, total, returned, truncated) = text_window(&text, args.offset, args.max_bytes);
    let doc = json!({
        "vault": args.vault.display().to_string(),
        "cx_id": args.cx_id.to_string(),
        "case_name": cx.metadata.get("case_name"),
        "opinion_type": cx.metadata.get("opinion_type"),
        "source": {
            "url": cx.metadata.get("source_url"),
            "download_url": cx.metadata.get("courtlistener_download_url"),
            "court_id": cx.metadata.get("court_id"),
            "court_name": cx.metadata.get("jurisdiction_court"),
            "appellate_district": cx.metadata.get("jurisdiction_appellate_district"),
            "state": cx.metadata.get("jurisdiction_state"),
            "date_filed": cx.metadata.get("date_filed"),
            "docket_number": cx.metadata.get("docket_number"),
            "opinion_id": cx.metadata.get("opinion_id"),
            "cluster_id": cx.metadata.get("cluster_id"),
            "cluster_slug": cx.metadata.get("cluster_slug"),
            "source_dataset": cx.metadata.get("source_dataset"),
            "precedential_status": cx.metadata.get("precedential_status"),
            "text_source": cx.metadata.get("text_source"),
            "license": cx.metadata.get("license"),
        },
        "total_bytes": total,
        "offset": args.offset,
        "returned_bytes": returned,
        "truncated": truncated,
        "input_ref": {
            "hash": hex_bytes(&cx.input_ref.hash),
            "pointer": cx.input_ref.pointer.as_deref(),
            "redacted": cx.input_ref.redacted,
        },
        "text": window,
    });
    print_line(&serialize(&doc, "case-text")?)
}

// ======================================================================
// 5. readback judge-census
// ======================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JudgeCensusArgs {
    vault: PathBuf,
    judge: String,
}

pub(crate) fn parse_judge_census_args(rest: &[String]) -> CliResult<JudgeCensusArgs> {
    let mut vault = None;
    let mut judge = None;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                vault = Some(PathBuf::from(value_of(rest, idx, "--vault")?));
            }
            "--judge" => {
                idx += 1;
                judge = Some(value_of(rest, idx, "--judge")?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected judge-census flag {other}"
                )));
            }
        }
        idx += 1;
    }
    let judge =
        judge.ok_or_else(|| CliError::usage("judge-census requires --judge <name-or-id>"))?;
    if judge.trim().is_empty() {
        return Err(CliError::usage("judge-census --judge must not be empty"));
    }
    Ok(JudgeCensusArgs {
        vault: vault.ok_or_else(|| CliError::usage("judge-census requires --vault <dir>"))?,
        judge,
    })
}

/// Documented, honest author-resolution coverage over this corpus (issue #1477 /
/// docs/lawsearch/05_judges.md): only ~48.7% of opinions carry a resolved author.
const DOCUMENTED_AUTHOR_COVERAGE: &str = "~48.7% author resolution (original demo-2: 9,028/18,528 = 0.4872); corrected against current physical authority ~49.6% (6,254/12,605). CAP-era rows carry no CourtListener author_id.";

/// Pure authorship aggregation for one judge query. A numeric query is treated as
/// a people-db person_id (matched against `author_id` and the `judges` panel
/// field); a non-numeric query is normalized-substring matched against
/// `author_str` (and the `judges` panel field).
pub(crate) struct JudgeCensus {
    query: String,
    query_norm: String,
    query_is_numeric: bool,
    total: usize,
    with_author_id: usize,
    authored: usize,
    authored_dissents: usize,
    panel_member: usize,
}

impl JudgeCensus {
    pub(crate) fn new(judge: &str) -> Self {
        let query = judge.trim().to_string();
        Self {
            query_norm: normalize_case_name(&query),
            query_is_numeric: !query.is_empty() && query.chars().all(|c| c.is_ascii_digit()),
            query,
            total: 0,
            with_author_id: 0,
            authored: 0,
            authored_dissents: 0,
            panel_member: 0,
        }
    }

    pub(crate) fn observe(&mut self, metadata: &BTreeMap<String, String>) {
        self.total += 1;
        let author_id = metadata
            .get("author_id")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if author_id.is_some() {
            self.with_author_id += 1;
        }
        let authored = if self.query_is_numeric {
            author_id == Some(self.query.as_str())
        } else {
            metadata
                .get("author_str")
                .map(|s| normalize_case_name(s))
                .is_some_and(|s| !s.is_empty() && s.contains(&self.query_norm))
        };
        if authored {
            self.authored += 1;
            if metadata.get("opinion_type").map(String::as_str) == Some(DISSENT_TYPE) {
                self.authored_dissents += 1;
            }
        }
        // Panel membership: the cluster-level `judges` field carries resolved
        // person_ids (and/or names). Match the raw query token defensively.
        if let Some(judges) = metadata.get("judges").map(|s| s.as_str()) {
            let hit = if self.query_is_numeric {
                judges
                    .split(|c: char| !c.is_ascii_digit())
                    .any(|tok| tok == self.query)
            } else {
                !self.query_norm.is_empty()
                    && normalize_case_name(judges).contains(&self.query_norm)
            };
            if hit {
                self.panel_member += 1;
            }
        }
    }

    fn to_json(&self, vault: &Path) -> Value {
        let coverage = if self.total == 0 {
            0.0
        } else {
            self.with_author_id as f64 / self.total as f64
        };
        json!({
            "vault": vault.display().to_string(),
            "judge_query": self.query,
            "resolved_as": if self.query_is_numeric { "person_id (author_id / judges panel)" } else { "author_str normalized-substring (and judges panel)" },
            "total_opinions_scanned": self.total,
            "opinions_with_author_id_n": self.with_author_id,
            "authored_count": self.authored,
            "authored_dissents_n": self.authored_dissents,
            "panel_member_count": self.panel_member,
            "live_author_resolution_coverage": coverage,
            "documented_author_resolution_coverage": DOCUMENTED_AUTHOR_COVERAGE,
            "match_rule": "numeric query -> author_id equality + judges person_id token; text query -> author_str/judges normalized-substring (lowercased, whitespace-collapsed)",
        })
    }
}

pub(crate) fn readback_judge_census_args(rest: &[String]) -> CliResult {
    let args = parse_judge_census_args(rest)?;
    harden(&args.vault)?;
    let mut census = JudgeCensus::new(&args.judge);
    scan_base_metadata(&args.vault, |row| {
        if let BaseRow::Live { metadata, .. } = row {
            census.observe(metadata);
        }
        Ok(())
    })?;
    print_line(&serialize(&census.to_json(&args.vault), "judge-census")?)
}

// ======================================================================
// shared Base scan + serialize
// ======================================================================

pub(crate) enum BaseRow<'a> {
    Tombstone,
    Live {
        cx_id: CxId,
        metadata: &'a BTreeMap<String, String>,
    },
}

/// Bounded page fold over live Base rows. Each row is decoded and handed to
/// `visit` one at a time; only the caller's running aggregates persist, so peak
/// decoded memory stays O(1) in the row count.
fn scan_base_metadata<F>(vault: &Path, mut visit: F) -> CliResult
where
    F: FnMut(BaseRow) -> CliResult,
{
    let rows = latest_cf_rows(vault, ColumnFamily::Base)?;
    for (_key, value) in rows {
        if is_tombstone_value(&value) {
            visit(BaseRow::Tombstone)?;
            continue;
        }
        let cx = decode_constellation_base(&value)?;
        visit(BaseRow::Live {
            cx_id: cx.cx_id,
            metadata: &cx.metadata,
        })?;
    }
    Ok(())
}

fn serialize(value: &Value, verb: &str) -> CliResult<String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {verb} output: {error}")))
}

#[cfg(test)]
mod tests;
