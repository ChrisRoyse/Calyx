//! The Calyx tool registry the agent drives.
//!
//! SECURITY MODEL: every tool is an ALLOWLISTED argv template. Model-supplied values
//! only ever land in *value* positions of a `Vec<String>` argv (never a flag
//! position, never a shell string), and each field is validated/clamped before it is
//! placed. There is no shell, no string interpolation of model output, and no
//! free-form binary/flag. A shell-metacharacter-laden query is therefore inert: it is
//! passed as ONE argv element to a process spawned without a shell.
//!
//! Every tool returns REAL JSON from the pinned `calyx` binary (or the binary's own
//! typed stderr envelope verbatim on failure). Tools are strictly READ-ONLY over the
//! production vault — none writes authority data.

use std::future::Future;
use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::config::Config;
use crate::error::{ApiError, ErrorCode};
use crate::live::{spawn_live_with_timeout, valid_answer_id};

/// A production search process deserializes the accepted cuVS CAGRA assets into GPU
/// memory. The resident service owns embeddings, not these per-process search assets,
/// so overlapping corpus_search subprocesses can exceed device memory. One
/// process-wide permit protects every episode/user while unrelated DB-native reads
/// still execute concurrently.
static CORPUS_SEARCH_GATE: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(1));

/// The read-only inputs a tool argv needs, projected from [`Config`] so argv building
/// is unit-testable without a full config.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub vault: String,
    pub resident_addr: String,
    pub d1_vault: String,
    pub d1_kernel: String,
    pub d2_vault: String,
    pub d2_kernel: String,
    pub search_k_max: u32,
}

impl ToolContext {
    pub fn from_config(cfg: &Config) -> Self {
        let search_k_max = cfg.chat.as_ref().map(|c| c.search_k_max).unwrap_or(20);
        Self {
            vault: cfg.vault.clone(),
            resident_addr: cfg.resident_addr.clone(),
            d1_vault: cfg.d1_vault.clone(),
            d1_kernel: cfg.d1_kernel.clone(),
            d2_vault: cfg.d2_vault.clone(),
            d2_kernel: cfg.d2_kernel.clone(),
            search_k_max,
        }
    }
}

/// The citation-overlay collection the tools default to. The accepted citation graph on
/// the production vault is materialized under `legal-citations-alias-v2` (the alias-aware
/// overlay: 28,834 edges / 9,158 nodes). The older `legal-citations-v1|v2` names are kept
/// as accepted values for back-compat, but they do NOT exist on the current prod vault —
/// defaulting to them is what made `citation_graph` read empty/mid-rebuild (#1977).
pub const DEFAULT_CITATION_COLLECTION: &str = "legal-citations-alias-v2";

/// The sentinel prefix the reshaped `citation_neighbors` payload puts on an out-of-corpus
/// (frontier) reference, so it can never be confused with a readable cx_id — and so
/// `case_read`/`case_text`/`case_segments` can reject it with an actionable error.
pub const FRONTIER_REF_PREFIX: &str = "frontier:";

/// The closed set of tool names the model may call.
pub const TOOL_NAMES: [&str; 15] = [
    "corpus_search",
    "case_read",
    "case_segments",
    "case_text",
    "kernel_answer",
    "kernel_inspect",
    "reproduce",
    "judge_profile",
    "judge_census",
    "citation_graph",
    "citation_neighbors",
    "cite_check",
    "corpus_stats",
    "ledger_verify",
    "ledger_tail",
];

/// Whether a tool is proven not to mutate authoritative state. This is an explicit
/// allowlist so a newly registered tool stays sequential until its storage behavior
/// is audited. `kernel_answer` is deliberately absent: it seals durable Answer/ledger
/// evidence and must retain deterministic ordering relative to every other call.
pub fn is_read_only(name: &str) -> bool {
    matches!(
        name,
        "corpus_search"
            | "case_read"
            | "case_segments"
            | "case_text"
            | "kernel_inspect"
            | "reproduce"
            | "judge_profile"
            | "judge_census"
            | "citation_graph"
            | "citation_neighbors"
            | "cite_check"
            | "corpus_stats"
            | "ledger_verify"
            | "ledger_tail"
    )
}

/// Tools whose otherwise read-only execution needs an exclusive hardware resource.
pub fn is_gpu_exclusive(name: &str) -> bool {
    name == "corpus_search"
}

/// Whether a successful result can establish a case/opinion as legal authority for
/// `[cx:...]` answer citations. Operational tools may contain nested `cx_id` fields
/// while describing prior work, but those identifiers are audit data—not authorities
/// retrieved for the current answer. Keep this as an explicit allowlist so a newly
/// registered tool cannot accidentally create a citation obligation.
pub fn returns_citable_authority(name: &str) -> bool {
    matches!(
        name,
        "corpus_search"
            | "case_read"
            | "case_segments"
            | "case_text"
            | "kernel_answer"
            | "judge_profile"
            | "judge_census"
            | "citation_graph"
            | "citation_neighbors"
            | "cite_check"
    )
}

/// The OpenAI-compatible `tools` array handed to the model. Descriptions are terse;
/// the deep teaching lives in the system prompt.
pub fn tool_specs() -> Vec<Value> {
    fn f(name: &str, desc: &str, params: Value) -> Value {
        json!({"type":"function","function":{"name":name,"description":desc,"parameters":params}})
    }
    let str_p = |d: &str| json!({"type":"string","description":d});
    vec![
        f(
            "corpus_search",
            "Semantic+structured panel search over all 109,133 canonical Ohio opinions (12,596 Cuyahoga Eighth District + 96,537 Ohio Supreme Court). Returns exact ranked cx_ids, scores, provenance, and authoritative Base metadata (case name, court, filed date, docket, dataset, opinion type). Fetch retained text only for selected authorities.",
            json!({"type":"object","properties":{"query":str_p("natural-language search text"),"k":{"type":"integer","description":"number of hits (1..=20)"}},"required":["query"]}),
        ),
        f(
            "case_read",
            "Metadata-only Base readback for one cx_id: case name, docket, date, opinion id, type, source URL, provenance. Never use it to learn a holding, and do not call it for a corpus_search hit whose authoritative metadata is already present; use case_text to read the opinion.",
            json!({"type":"object","properties":{"cx_id":str_p("exact cx_id from a prior tool result"),"max_bytes":{"type":"integer","description":"soft cap on returned size"}},"required":["cx_id"]}),
        ),
        f(
            "case_segments",
            "Like case_read but includes per-slot segmentation provenance (how a long opinion was measured as whole segments across the lenses).",
            json!({"type":"object","properties":{"cx_id":str_p("exact cx_id"),"max_bytes":{"type":"integer"}},"required":["cx_id"]}),
        ),
        f(
            "case_text",
            "Read the retained opinion text for one cx_id (a bounded byte window of the real source). Use to quote holdings verbatim. Returns case name, opinion type, original-source locator details, total bytes, and the text window.",
            json!({"type":"object","properties":{"cx_id":str_p("exact cx_id from a prior tool result"),"max_bytes":{"type":"integer","description":"window size in bytes (default 32768)"},"offset":{"type":"integer","description":"starting byte offset into the opinion (default 0)"}},"required":["cx_id"]}),
        ),
        f(
            "kernel_answer",
            "Primary doctrinal retriever: maps the query into the accepted full corpus, ranks the sealed resident kernel, derives the smallest grounded association path, verifies retained source support, and records a reproducible hop chain/answer_id. Use first for ordinary doctrinal legal questions. A failure is terminal and must be surfaced, never hidden by substituting another retrieval path.",
            json!({"type":"object","properties":{"query":str_p("the doctrinal question")},"required":["query"]}),
        ),
        f(
            "kernel_inspect",
            "Kernel-health readback for a scoped kernel: which='doctrine' (traffic-stop kernel) or 'judge' (Boyle kernel). Returns member count, recall vs 0.95 gate, grounded fraction.",
            json!({"type":"object","properties":{"which":{"type":"string","enum":["doctrine","judge"]}},"required":["which"]}),
        ),
        f(
            "reproduce",
            "Bit-for-bit cryptographic replay of a prior grounded answer_id (bit_parity, original_hash, reproduced_hash).",
            json!({"type":"object","properties":{"answer_id":str_p("an answer_id returned by kernel_answer")},"required":["answer_id"]}),
        ),
        f(
            "judge_profile",
            "The configured judge kernel (Mary J. Boyle) authorship census: authored count, dissents, kernel health. Descriptive, never predictive.",
            json!({"type":"object","properties":{}}),
        ),
        f(
            "judge_census",
            "DB-native authorship census for ANY judge (name or people-db person_id) over Base author metadata: authored_count, authored_dissents, panel_member_count, plus honest author-resolution coverage (~48.7%). Descriptive, never predictive.",
            json!({"type":"object","properties":{"judge":str_p("judge display name (substring, e.g. 'Boyle') or numeric people-db person_id")},"required":["judge"]}),
        ),
        f(
            "citation_graph",
            "The live typed citation-graph overlay generation (node/edge/frontier counts, provenance) with its lifecycle state. collection defaults to the accepted legal-citations-alias-v2 overlay.",
            json!({"type":"object","properties":{"collection":{"type":"string","enum":["legal-citations-v1","legal-citations-v2","legal-citations-alias-v2"]}}}),
        ),
        f(
            "citation_neighbors",
            "The citation fan-in/fan-out for ONE opinion. Each IN-CORPUS neighbor carries a readable_cx_id (safe for case_read/case_text). OUT-OF-CORPUS neighbors carry outside_corpus:true, a frontier_ref, and readable_cx_id:null — they have NO readable text; never pass a frontier_ref to case_read (use cite_check for existence). direction='in' (who cites it), 'out' (what it cites), or 'both'. collection defaults to legal-citations-alias-v2.",
            json!({"type":"object","properties":{"cx_id":str_p("exact cx_id"),"direction":{"type":"string","enum":["in","out","both"]},"limit":{"type":"integer","description":"max neighbors (default 256)"},"collection":{"type":"string","enum":["legal-citations-v1","legal-citations-v2","legal-citations-alias-v2"]}},"required":["cx_id","direction"]}),
        ),
        f(
            "cite_check",
            "DB-native existence probe: scan Base metadata for a case name (exact + normalized substring over case_name/case_name_full). Returns status='found' with found_cx_ids, or status='NOT_IN_CORPUS' as a first-class result (proven absence, NOT an error).",
            json!({"type":"object","properties":{"case_name":str_p("the case name to look for"),"proposition":str_p("optional proposition to note for follow-up search")},"required":["case_name"]}),
        ),
        f(
            "corpus_stats",
            "Live corpus-scale aggregates. dimension='production' independently reads the physical search/kernel manifests and serving resident; 'citation_overlay' reads graph totals; 'opinion_types' scans Base metadata; 'manifest' verifies the ledger chain.",
            json!({"type":"object","properties":{"dimension":{"type":"string","enum":["production","citation_overlay","opinion_types","manifest"]}},"required":["dimension"]}),
        ),
        f(
            "ledger_verify",
            "Full append-only chain verification over the production vault (status, break_at, entry count).",
            json!({"type":"object","properties":{}}),
        ),
        f(
            "ledger_tail",
            "The last N ledger entries ({verify, entries[]}). last clamps to 1..=50.",
            json!({"type":"object","properties":{"last":{"type":"integer"}}}),
        ),
    ]
}

// ---- field validators ------------------------------------------------------------

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::ChatToolArg, msg)
}

/// A free-text field (query / case name). Rejected only for emptiness, NUL bytes, or
/// absurd length; otherwise passed VERBATIM (it is a single argv element, so shell
/// metacharacters are inert).
fn validate_text(field: &str, raw: &str) -> Result<String, ApiError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(bad(format!("{field} must not be empty")));
    }
    if s.len() > 4000 {
        return Err(bad(format!("{field} exceeds 4000 bytes")));
    }
    if s.contains('\0') {
        return Err(bad(format!("{field} contains a NUL byte")));
    }
    Ok(s.to_owned())
}

/// A cx_id token: 8..=128 chars of `[A-Za-z0-9_-]`, never flag-like. Blocks argv
/// smuggling (`--flag`, paths) even though the array form already blocks the shell.
fn validate_cx_id(raw: &str) -> Result<String, ApiError> {
    let s = raw.trim();
    // An out-of-corpus (frontier) reference surfaced by `citation_neighbors` is NOT a
    // readable opinion: it has no cx text and no cx_id. The reshaped neighbor payload
    // carries it as `frontier:<...>` under `frontier_ref` (never as a `cx_id`), so a model
    // that mistakenly feeds it here gets a SPECIFIC, actionable correction rather than the
    // generic token-shape rejection — actionable errors are how the model self-corrects.
    if let Some(rest) = s.strip_prefix(FRONTIER_REF_PREFIX) {
        let shown: String = rest.chars().take(48).collect();
        return Err(bad(format!(
            "{shown:?} is an out-of-corpus (frontier) reference from citation_neighbors: it has \
             no readable opinion text and no cx_id. Do NOT pass it to case_read/case_text/case_segments. \
             Use cite_check for existence questions about out-of-corpus authorities."
        )));
    }
    let ok = (8..=128).contains(&s.len())
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(s.to_owned())
    } else {
        // Echo a bounded, safe slice of the offending value so the reasoning model can
        // self-correct (the message is fed back to the agent AND shown in the step pill).
        // The common failure mode: passing a frontier / `cites_outside_corpus` neighbor
        // ref (e.g. "frontier:rodriguez-v-us") or a decorated/truncated token — none of
        // which are cx_ids. cx_id is a model-supplied, length-bounded field: safe to echo.
        let shown: String = s.chars().take(48).collect();
        Err(bad(format!(
            "cx_id must be an 8..=128 char alphanumeric/_/- token \
             (only in-corpus opinions have a cx_id; frontier/out-of-corpus refs do not); got {shown:?}"
        )))
    }
}

/// The citation-neighbor direction: one of the three allowlisted tokens.
fn validate_direction(raw: &str) -> Result<String, ApiError> {
    match raw {
        "in" | "out" | "both" => Ok(raw.to_owned()),
        other => Err(bad(format!(
            "direction must be 'in', 'out', or 'both', got {other:?}"
        ))),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("missing required string arg `{key}`")))
}

fn arg_u32_opt(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

// ---- argv builders ---------------------------------------------------------------

/// Build the exact allowlisted argv for a tool call, or a typed arg/unknown-tool error
/// (which the loop feeds back to the model so it can adapt).
pub fn build_argv(ctx: &ToolContext, name: &str, args: &Value) -> Result<Vec<String>, ApiError> {
    match name {
        "corpus_search" => {
            let query = validate_text("query", arg_str(args, "query")?)?;
            let k = arg_u32_opt(args, "k")
                .unwrap_or(8)
                .clamp(1, ctx.search_k_max);
            Ok(vec![
                "search".into(),
                ctx.vault.clone(),
                query,
                "--k".into(),
                k.to_string(),
                "--provenance".into(),
                "--metadata".into(),
                "--resident-addr".into(),
                ctx.resident_addr.clone(),
            ])
        }
        "case_read" => {
            let cx = validate_cx_id(arg_str(args, "cx_id")?)?;
            Ok(vec![
                "readback".into(),
                "cx-list".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--cx-id".into(),
                cx,
            ])
        }
        "case_segments" => {
            let cx = validate_cx_id(arg_str(args, "cx_id")?)?;
            Ok(vec![
                "readback".into(),
                "cx-list".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--cx-id".into(),
                cx,
                "--include-slots".into(),
            ])
        }
        "case_text" => {
            let cx = validate_cx_id(arg_str(args, "cx_id")?)?;
            let mut argv = vec![
                "readback".into(),
                "case-text".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--cx-id".into(),
                cx,
            ];
            if let Some(n) = arg_u32_opt(args, "max_bytes") {
                argv.push("--max-bytes".into());
                argv.push(n.to_string());
            }
            if let Some(n) = arg_u32_opt(args, "offset") {
                argv.push("--offset".into());
                argv.push(n.to_string());
            }
            Ok(argv)
        }
        "kernel_answer" => {
            let query = validate_text("query", arg_str(args, "query")?)?;
            Ok(vec![
                "kernel-answer".into(),
                ctx.vault.clone(),
                query,
                "--resident-addr".into(),
                ctx.resident_addr.clone(),
            ])
        }
        "kernel_inspect" => {
            let which = arg_str(args, "which")?;
            let (root, kernel) = match which {
                "doctrine" => (&ctx.d1_vault, &ctx.d1_kernel),
                "judge" => (&ctx.d2_vault, &ctx.d2_kernel),
                other => {
                    return Err(bad(format!(
                        "which must be 'doctrine' or 'judge', got {other:?}"
                    )));
                }
            };
            Ok(vec![
                "readback".into(),
                "kernel-health".into(),
                "--root".into(),
                root.clone(),
                "--kernel-id".into(),
                kernel.clone(),
            ])
        }
        "reproduce" => {
            let id = arg_str(args, "answer_id")?.trim();
            if !valid_answer_id(id) {
                return Err(bad(
                    "answer_id must be an 8..=128 char alphanumeric/_/- token".to_owned(),
                ));
            }
            Ok(vec![
                "reproduce".into(),
                "--record".into(),
                "--resident-addr".into(),
                ctx.resident_addr.clone(),
                ctx.vault.clone(),
                id.to_owned(),
            ])
        }
        "judge_profile" => Ok(vec![
            "readback".into(),
            "kernel-health".into(),
            "--root".into(),
            ctx.d2_vault.clone(),
            "--kernel-id".into(),
            ctx.d2_kernel.clone(),
        ]),
        "judge_census" => {
            let judge = validate_text("judge", arg_str(args, "judge")?)?;
            Ok(vec![
                "readback".into(),
                "judge-census".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--judge".into(),
                judge,
            ])
        }
        "citation_graph" => {
            let collection = collection_arg(args)?;
            Ok(vec![
                "graph-collection-generations".into(),
                ctx.vault.clone(),
                "--collection".into(),
                collection,
            ])
        }
        "citation_neighbors" => {
            let cx = validate_cx_id(arg_str(args, "cx_id")?)?;
            let direction = validate_direction(arg_str(args, "direction")?)?;
            let collection = collection_arg(args)?;
            let mut argv = vec![
                "readback".into(),
                "citation-neighbors".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--cx-id".into(),
                cx,
                "--direction".into(),
                direction,
                "--collection".into(),
                collection,
            ];
            if let Some(n) = arg_u32_opt(args, "limit") {
                argv.push("--limit".into());
                argv.push(n.to_string());
            }
            Ok(argv)
        }
        "cite_check" => {
            let name = validate_text("case_name", arg_str(args, "case_name")?)?;
            let mut argv = vec![
                "cite-check".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--case-name".into(),
                name,
            ];
            if let Some(prop) = args.get("proposition").and_then(Value::as_str) {
                let prop = validate_text("proposition", prop)?;
                argv.push("--proposition".into());
                argv.push(prop);
            }
            Ok(argv)
        }
        "corpus_stats" => {
            let dim = arg_str(args, "dimension")?;
            match dim {
                "production" => Err(ApiError::new(
                    ErrorCode::Internal,
                    "production corpus status reached subprocess dispatch instead of the shared in-process verifier",
                )),
                "citation_overlay" => Ok(vec![
                    "graph-collection-generations".into(),
                    ctx.vault.clone(),
                    "--collection".into(),
                    DEFAULT_CITATION_COLLECTION.into(),
                ]),
                "opinion_types" => Ok(vec![
                    "readback".into(),
                    "opinion-type-census".into(),
                    "--vault".into(),
                    ctx.vault.clone(),
                ]),
                "manifest" => Ok(vec!["verify-chain".into(), ctx.vault.clone()]),
                other => Err(bad(format!(
                    "dimension must be 'production', 'citation_overlay', 'opinion_types', or 'manifest', got {other:?}"
                ))),
            }
        }
        "ledger_verify" => Ok(vec!["verify-chain".into(), ctx.vault.clone()]),
        "ledger_tail" => {
            let last = arg_u32_opt(args, "last").unwrap_or(5).clamp(1, 50);
            Ok(vec![
                "ledger-tail".into(),
                "--vault".into(),
                ctx.vault.clone(),
                "--last".into(),
                last.to_string(),
            ])
        }
        other => Err(ApiError::new(
            ErrorCode::ChatToolUnknown,
            format!("unknown tool {other:?}; not in the allowlisted registry"),
        )),
    }
}

fn collection_arg(args: &Value) -> Result<String, ApiError> {
    match args.get("collection").and_then(Value::as_str) {
        None => Ok(DEFAULT_CITATION_COLLECTION.to_owned()),
        Some(c @ ("legal-citations-v1" | "legal-citations-v2" | "legal-citations-alias-v2")) => {
            Ok(c.to_owned())
        }
        Some(other) => Err(bad(format!(
            "collection must be legal-citations-v1|legal-citations-v2|legal-citations-alias-v2, got {other:?}"
        ))),
    }
}

/// True for tools whose stdout is JSONL (only `ledger_tail`), vs a single JSON doc.
pub fn is_jsonl(name: &str) -> bool {
    name == "ledger_tail"
}

/// Every tool receives the same fail-closed live budget. `kernel_answer` and
/// `reproduce` rank through the verified resident and therefore must not depend
/// on an inflated cold-index timeout.
pub fn tool_timeout(cfg: &Config, _name: &str) -> std::time::Duration {
    cfg.live_timeout
}

// ---- corpus_search lens provenance (real stderr, not invented) --------------------
//
// `calyx search` emits two structured stderr line families the JSON stdout omits:
//
//   * ONE roster line, `CALYX_SEARCH_SLOTS resident_gpu=[<slot>:<key>,…] local_cpu=[…]
//     parked_excluded=[…] retired_excluded=[…] unregistered_excluded=[…]` — exactly which
//     learned lenses served this query and which were honestly excluded
//     (see `calyx-cli/src/cmd/search/roster.rs::emit_runtime_line`).
//   * ONE per-firing-slot line, `CALYX_SEARCH_RUNTIME phase=search_slot.done slot=<n>
//     elapsed_ms=<n> count=<n> detail=stage=<…> slot_elapsed_ms=<n> slot_backend=<…>
//     slot_compute_placement=<…> …` — the real backend, GPU/CPU placement, and latency
//     of each lens (`calyx-cli/src/cmd/search/engine.rs::emit_search_trace` +
//     `calyx-search/src/engine_slot_fanout.rs`).
//
// [`parse_search_lens_provenance`] lifts these REAL lines into structured JSON so the
// Research Console's "Lenses" tab can render the multi-lens fusion. Nothing here is
// fabricated: an absent line yields an absent field, never a placeholder.

/// One learned lens in the frozen serving roster: its slot id and slot key (name).
fn slot_ref(slot: u16, key: &str) -> Value {
    json!({ "slot": slot, "key": key })
}

/// Parse a `group=[<slot>:<key>,<slot>:<key>,…]` bracket list into `[{slot,key}]`.
/// Tolerant: a malformed entry is skipped rather than poisoning the whole roster.
fn parse_slot_group(inner: &str) -> Vec<Value> {
    inner
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (slot, key) = entry.split_once(':')?;
            let slot: u16 = slot.trim().parse().ok()?;
            Some(slot_ref(slot, key.trim()))
        })
        .collect()
}

/// Split a stderr line into `key=value` tokens. Values never contain spaces in the
/// search runtime lines (bracket lists have no internal spaces), so a whitespace split
/// is exact.
fn kv_tokens(line: &str) -> std::collections::HashMap<&str, &str> {
    line.split_whitespace()
        .filter_map(|tok| tok.split_once('='))
        .collect()
}

/// Lift the REAL `CALYX_SEARCH_SLOTS` roster line and every `search_slot.done` per-slot
/// timing line out of a `corpus_search` subprocess's stderr into structured JSON, or
/// `None` when the stderr carried neither (e.g. a resident that emitted no roster).
pub fn parse_search_lens_provenance(stderr: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(stderr).ok()?;
    let mut roster: Option<Value> = None;
    let mut timings: Vec<Value> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("CALYX_SEARCH_SLOTS ") {
            let kv = kv_tokens(rest);
            let group = |name: &str| {
                kv.get(name)
                    .map(|raw| parse_slot_group(raw.trim_start_matches('[').trim_end_matches(']')))
                    .unwrap_or_default()
            };
            roster = Some(json!({
                "resident_gpu": group("resident_gpu"),
                "local_cpu": group("local_cpu"),
                "parked_excluded": group("parked_excluded"),
                "retired_excluded": group("retired_excluded"),
                "unregistered_excluded": group("unregistered_excluded"),
            }));
        } else if line.contains("phase=search_slot.done") {
            let kv = kv_tokens(line);
            let Some(slot) = kv.get("slot").and_then(|s| s.parse::<u16>().ok()) else {
                continue;
            };
            let mut row = serde_json::Map::new();
            row.insert("slot".into(), json!(slot));
            if let Some(count) = kv.get("count").and_then(|s| s.parse::<u64>().ok()) {
                row.insert("count".into(), json!(count));
            }
            if let Some(ms) = kv
                .get("slot_elapsed_ms")
                .and_then(|s| s.parse::<u64>().ok())
            {
                row.insert("slot_elapsed_ms".into(), json!(ms));
            }
            if let Some(backend) = kv.get("slot_backend") {
                row.insert("backend".into(), json!(backend));
            }
            if let Some(placement) = kv.get("slot_compute_placement") {
                row.insert("placement".into(), json!(placement));
            }
            timings.push(Value::Object(row));
        }
    }

    if roster.is_none() && timings.is_empty() {
        return None;
    }
    let mut out = roster.unwrap_or_else(|| json!({}));
    if let Value::Object(map) = &mut out {
        let resident = map
            .get("resident_gpu")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let cpu = map
            .get("local_cpu")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        map.insert("slot_timings".into(), Value::Array(timings));
        map.insert("lens_count".into(), json!(resident + cpu));
    }
    Some(out)
}

/// Attach parsed lens provenance to a successful `corpus_search` payload. The ranked
/// hits array (or existing `{hits}` object) is preserved under `hits`; the roster/timing
/// data is added as a sibling `lens_provenance`. When no roster line was captured the
/// payload is returned unchanged so the model contract is unperturbed.
pub fn enrich_search_payload(payload: Value, stderr: &[u8]) -> Value {
    let Some(lens) = parse_search_lens_provenance(stderr) else {
        return payload;
    };
    let hits = match payload {
        Value::Object(ref map) if map.contains_key("hits") => {
            map.get("hits").cloned().unwrap_or(Value::Array(vec![]))
        }
        other => other,
    };
    json!({ "hits": hits, "lens_provenance": lens })
}

// ---- citation_neighbors: disambiguate out-of-corpus (frontier) neighbors --------
//
// The `readback citation-neighbors` CLI emits ONE shape for every neighbor:
// `{cx_id, direction, edge_type, weight, edge_value}`. For a frontier neighbor the
// `edge_type` is `cites_outside_corpus` and the `cx_id` is a CONTENT-ADDRESSED synthetic
// frontier node id — a clean 32-hex token that is indistinguishable from a real,
// readable opinion cx_id. That is the exact trap behind #1977: the model saw a `cx_id`
// field on an out-of-corpus reference and fed it to `case_read` (which cannot read it).
//
// [`reshape_neighbors_payload`] makes the model-facing contract UNAMBIGUOUS and, just as
// importantly, keeps a frontier node id OUT of the grounding set (it renames the field
// off the `cx_id` allowlist key), so a frontier ref can never be cited as if it were a
// real opinion.

/// A neighbor row is a frontier (out-of-corpus) reference iff its edge type is the
/// citation-overlay frontier edge type, on the row or inside the decoded edge value.
fn neighbor_is_frontier(neighbor: &Value) -> bool {
    const FRONTIER_EDGE_TYPE: &str = "cites_outside_corpus";
    if neighbor.get("edge_type").and_then(Value::as_str) == Some(FRONTIER_EDGE_TYPE) {
        return true;
    }
    let edge_value = neighbor.get("edge_value");
    edge_value
        .and_then(|v| v.get("edge_type"))
        .and_then(Value::as_str)
        == Some(FRONTIER_EDGE_TYPE)
        || edge_value.and_then(|v| v.get("boundary_reason")).is_some()
}

/// Reshape one neighbor in place. In-corpus neighbors gain an explicit
/// `readable_cx_id` mirror and `outside_corpus:false`; frontier neighbors lose their
/// confusable `cx_id`, gain `outside_corpus:true`, `readable_cx_id:null`, a
/// `frontier_ref` (`frontier:<id>`), the external opinion id, and an actionable note.
fn reshape_one_neighbor(neighbor: &mut Value) {
    let frontier = neighbor_is_frontier(neighbor);
    let Some(map) = neighbor.as_object_mut() else {
        return;
    };
    if frontier {
        // Move the synthetic frontier node id off the `cx_id` allowlist key so it can
        // never enter the grounding set or be read back as an opinion.
        let raw = map
            .remove("cx_id")
            .and_then(|v| v.as_str().map(str::to_owned));
        map.insert("outside_corpus".into(), Value::Bool(true));
        map.insert("readable_cx_id".into(), Value::Null);
        if let Some(raw) = raw {
            map.insert(
                "frontier_ref".into(),
                Value::String(format!("{FRONTIER_REF_PREFIX}{raw}")),
            );
        }
        if let Some(ext) = map
            .get("edge_value")
            .and_then(|v| v.get("cited_opinion_id"))
            .cloned()
        {
            map.insert("external_opinion_id".into(), ext);
        }
        map.insert(
            "_note".into(),
            Value::String(
                "out-of-corpus citation: no readable opinion text and no cx_id. Use cite_check \
                 for existence; never case_read/case_text/case_segments."
                    .into(),
            ),
        );
    } else {
        if let Some(cx) = map.get("cx_id").cloned() {
            map.insert("readable_cx_id".into(), cx);
        }
        map.insert("outside_corpus".into(), Value::Bool(false));
    }
}

/// Rewrite a `citation_neighbors` payload so out-of-corpus (frontier) neighbors can never
/// be confused with readable in-corpus cx_ids. Applied to the real CLI JSON before it is
/// handed to the model, the grounding ledger, or the UI. A payload without a `neighbors`
/// array (e.g. a forwarded error envelope) is returned unchanged.
pub fn reshape_neighbors_payload(mut payload: Value) -> Value {
    if let Some(neighbors) = payload.get_mut("neighbors").and_then(Value::as_array_mut) {
        for neighbor in neighbors.iter_mut() {
            reshape_one_neighbor(neighbor);
        }
    }
    payload
}

/// A human sentence describing what a tool call is about to do.
///
/// `opinion_label` is authoritative display metadata retained from a prior real tool
/// result. Raw content-addressed cx_ids are intentionally never exposed in UI copy.
pub fn verb(name: &str, args: &Value, opinion_label: Option<&str>) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("");
    match name {
        "corpus_search" => format!("Searching Ohio court opinions for “{}”…", s("query")),
        "case_read" => opinion_label
            .map(|label| format!("Reading {label}…"))
            .unwrap_or_else(|| "Reading a court opinion…".into()),
        "case_segments" => opinion_label
            .map(|label| format!("Reading segmentation details for {label}…"))
            .unwrap_or_else(|| "Reading court-opinion segmentation details…".into()),
        "case_text" => opinion_label
            .map(|label| format!("Reading the opinion text for {label}…"))
            .unwrap_or_else(|| "Reading a court opinion…".into()),
        "kernel_answer" => format!("Measuring the sealed doctrine kernel for “{}”…", s("query")),
        "kernel_inspect" => format!("Inspecting the {} kernel health…", s("which")),
        "reproduce" => "Cryptographically replaying a recorded answer…".into(),
        "judge_profile" => "Reading the Mary J. Boyle authorship census…".into(),
        "judge_census" => format!("Counting authored opinions for judge “{}”…", s("judge")),
        "citation_graph" => "Reading the typed citation-graph overlay…".into(),
        "citation_neighbors" => match (s("direction"), opinion_label) {
            ("in", Some(label)) => format!("Reading opinions that cite {label}…"),
            ("out", Some(label)) => format!("Reading opinions cited by {label}…"),
            ("both", Some(label)) => format!("Reading citations to and from {label}…"),
            ("in", None) => "Reading opinions that cite a court opinion…".into(),
            ("out", None) => "Reading opinions cited by a court opinion…".into(),
            _ => "Reading citations to and from a court opinion…".into(),
        },
        "cite_check" => format!(
            "Checking whether “{}” exists in the corpus…",
            s("case_name")
        ),
        "corpus_stats" => format!("Reading corpus statistics ({})…", s("dimension")),
        "ledger_verify" => "Verifying the full append-only ledger chain…".into(),
        "ledger_tail" => "Reading the most recent ledger entries…".into(),
        other => format!("Running {other}…"),
    }
}

/// A one-line headline summarising a REAL result payload (no invented numbers).
pub fn headline(name: &str, ok: bool, payload: &Value) -> String {
    if !ok {
        let code = payload
            .get("code")
            .or_else(|| payload.get("upstream").and_then(|u| u.get("code")))
            .and_then(Value::as_str)
            .unwrap_or("error");
        return format!("tool error: {code}");
    }
    match name {
        "corpus_search" => {
            let arr = payload
                .as_array()
                .or_else(|| payload.get("hits").and_then(Value::as_array));
            match arr {
                Some(hits) => {
                    let top = hits
                        .first()
                        .and_then(|h| h.get("score"))
                        .and_then(Value::as_f64);
                    match top {
                        Some(s) => format!("{} hits (top score {s:.4})", hits.len()),
                        None => format!("{} hits", hits.len()),
                    }
                }
                None => "search complete".into(),
            }
        }
        "cite_check" => {
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("?");
            let n = payload
                .get("match_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            match status {
                "found" => format!("found ({n} matching cx_id(s))"),
                other => other.to_owned(),
            }
        }
        "case_read" | "case_segments" => {
            let row = first_row(payload);
            let name = row
                .and_then(|r| r.get("metadata"))
                .and_then(|m| m.get("case_name"))
                .and_then(Value::as_str)
                .unwrap_or("(unnamed)");
            name.to_owned()
        }
        "case_text" => {
            let name = payload
                .get("case_name")
                .and_then(Value::as_str)
                .unwrap_or("(unnamed)");
            let bytes = payload
                .get("returned_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            format!("{name} ({bytes} bytes of text)")
        }
        "citation_neighbors" => {
            let matched = payload.get("matched").and_then(Value::as_u64).unwrap_or(0);
            let dir = payload
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("{matched} {dir} citation neighbor(s)")
        }
        "judge_census" => {
            let authored = payload.get("authored_count").and_then(Value::as_u64);
            let panel = payload.get("panel_member_count").and_then(Value::as_u64);
            match (authored, panel) {
                (Some(a), Some(p)) => format!("{a} authored, {p} panels"),
                (Some(a), None) => format!("{a} authored"),
                _ => "judge census".into(),
            }
        }
        "kernel_answer" => {
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("?");
            let hops = payload.get("hops").and_then(Value::as_array).map(Vec::len);
            match hops {
                Some(h) => format!("status={status}, {h} hops"),
                None => format!("status={status}"),
            }
        }
        "kernel_inspect" | "judge_profile" => {
            let size = payload.get("size").and_then(Value::as_u64);
            let recall = payload
                .get("recall")
                .and_then(|r| r.get("ratio"))
                .and_then(Value::as_f64);
            match (size, recall) {
                (Some(s), Some(r)) => format!("{s} members, recall {r}"),
                (Some(s), None) => format!("{s} members"),
                _ => "kernel health".into(),
            }
        }
        "reproduce" => {
            let parity = payload.get("bit_parity").and_then(Value::as_bool);
            format!(
                "bit_parity={}",
                parity.map(|b| b.to_string()).unwrap_or_else(|| "?".into())
            )
        }
        "citation_graph" | "corpus_stats" => {
            if let Some(ready) = payload.get("ready").and_then(Value::as_bool) {
                let opinions = payload
                    .pointer("/scope/physical_opinions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let lenses = payload
                    .pointer("/resident/warmed_lens_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                return format!(
                    "production ready={ready}, {opinions} opinions, {lenses} warmed lenses"
                );
            }
            // The opinion-type census dimension has no nodes/edges; report its totals.
            if let Some(total) = payload.get("physical_opinions_n").and_then(Value::as_u64) {
                let dissents = payload
                    .get("dissents_n")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                return format!("{total} opinions, {dissents} dissents");
            }
            let edges = find_u64(payload, "edges").or_else(|| find_u64(payload, "edge_n"));
            let nodes = find_u64(payload, "nodes").or_else(|| find_u64(payload, "node_n"));
            match (nodes, edges) {
                (Some(n), Some(e)) => format!("{n} nodes, {e} edges"),
                _ => {
                    let status = payload.get("status").and_then(Value::as_str);
                    status
                        .map(|s| format!("status={s}"))
                        .unwrap_or_else(|| "ok".into())
                }
            }
        }
        "ledger_verify" => {
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("?");
            format!("chain status={status}")
        }
        "ledger_tail" => {
            let n = payload
                .get("entries")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            format!("{n} recent entries")
        }
        _ => "ok".into(),
    }
}

fn first_row(payload: &Value) -> Option<&Value> {
    match payload {
        Value::Array(a) => a.first(),
        other => Some(other),
    }
}

/// Find the first u64 under any key equal to `key`, recursively.
fn find_u64(payload: &Value, key: &str) -> Option<u64> {
    match payload {
        Value::Object(m) => {
            if let Some(v) = m.get(key).and_then(Value::as_u64) {
                return Some(v);
            }
            m.values().find_map(|v| find_u64(v, key))
        }
        Value::Array(a) => a.iter().find_map(|v| find_u64(v, key)),
        _ => None,
    }
}

/// For `case_read`/`case_segments`, drop the bulky raw `base_hex` (opinion bytes as
/// hex — noise to the model) so the readback the agent sees is metadata + provenance.
/// `max_bytes` is advisory; when the shrunk payload still exceeds it we keep only
/// metadata/provenance. Applied to every row in a cx-list array.
pub fn shrink_case_payload(mut payload: Value, name: &str, max_bytes: Option<usize>) -> Value {
    if name != "case_read" && name != "case_segments" {
        return payload;
    }
    strip_base_hex(&mut payload);
    if let Some(cap) = max_bytes {
        let len = serde_json::to_string(&payload)
            .map(|s| s.len())
            .unwrap_or(0);
        if len > cap {
            payload = keep_metadata_only(&payload);
        }
    }
    payload
}

fn strip_base_hex(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("base_hex");
            for (_, child) in m.iter_mut() {
                strip_base_hex(child);
            }
        }
        Value::Array(a) => {
            for child in a.iter_mut() {
                strip_base_hex(child);
            }
        }
        _ => {}
    }
}

fn keep_metadata_only(v: &Value) -> Value {
    let reduce = |row: &Value| {
        json!({
            "cx_id": row.get("cx_id").cloned().unwrap_or(Value::Null),
            "metadata": row.get("metadata").cloned().unwrap_or(Value::Null),
            "provenance": row.get("provenance").cloned().unwrap_or(Value::Null),
            "_note": "payload trimmed to metadata to honor max_bytes",
        })
    };
    match v {
        Value::Array(a) => Value::Array(a.iter().map(reduce).collect()),
        other => reduce(other),
    }
}

/// The outcome of executing one tool call.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub ok: bool,
    pub payload: Value,
    pub latency_ms: u64,
}

/// The execution seam. Production runs the pinned binary; tests supply scripted
/// outcomes so the loop is deterministic and needs no resident.
pub trait ToolRunner: Send + Sync {
    fn run(&self, name: String, args: Value) -> impl Future<Output = ToolOutcome> + Send;
}

/// Production runner: validate → build allowlisted argv → spawn the pinned binary →
/// interpret real stdout JSON (or forward the typed stderr envelope verbatim).
pub struct CalyxToolRunner {
    cfg: std::sync::Arc<Config>,
}

impl CalyxToolRunner {
    pub fn new(cfg: std::sync::Arc<Config>) -> Self {
        Self { cfg }
    }
}

impl ToolRunner for CalyxToolRunner {
    async fn run(&self, name: String, args: Value) -> ToolOutcome {
        if name == "corpus_stats"
            && args.get("dimension").and_then(Value::as_str) == Some("production")
        {
            let started = std::time::Instant::now();
            return ToolOutcome {
                ok: true,
                payload: crate::production::production_status(&self.cfg).await,
                latency_ms: started.elapsed().as_millis() as u64,
            };
        }
        let ctx = ToolContext::from_config(&self.cfg);
        let argv = match build_argv(&ctx, &name, &args) {
            Ok(a) => a,
            // A typed arg/unknown-tool error IS a real tool result the model adapts to.
            Err(err) => {
                return ToolOutcome {
                    ok: false,
                    payload: err.body(),
                    latency_ms: 0,
                };
            }
        };
        let started = std::time::Instant::now();
        let timeout = tool_timeout(&self.cfg, &name);
        let _gpu_search_permit = if is_gpu_exclusive(&name) {
            let waiting = std::time::Instant::now();
            let permit = CORPUS_SEARCH_GATE
                .acquire()
                .await
                .expect("process-owned corpus search semaphore is never closed");
            tracing::info!(
                waited_ms = waiting.elapsed().as_millis() as u64,
                "acquired exclusive GPU search permit"
            );
            Some(permit)
        } else {
            None
        };
        match spawn_live_with_timeout(&self.cfg, &argv, timeout).await {
            Err(err) => ToolOutcome {
                ok: false,
                payload: err.body(),
                latency_ms: started.elapsed().as_millis() as u64,
            },
            Ok(output) => {
                let ok = output.status.success();
                let parsed = if is_jsonl(&name) {
                    crate::live::interpret_ledger_tail(ok, &output.stdout, &output.stderr)
                } else {
                    crate::live::interpret(&argv, ok, &output.stdout, &output.stderr)
                };
                match parsed {
                    Ok(mut payload) => {
                        let max_bytes = args
                            .get("max_bytes")
                            .and_then(Value::as_u64)
                            .map(|n| n as usize);
                        payload = shrink_case_payload(payload, &name, max_bytes);
                        // corpus_search: capture the REAL per-lens roster + timing lines
                        // the search emits on stderr (the "N learned lenses voted" story).
                        if name == "corpus_search" {
                            payload = enrich_search_payload(payload, &output.stderr);
                        }
                        // citation_neighbors: disambiguate out-of-corpus (frontier) refs so
                        // they can never be read back or cited as real opinions (#1977).
                        if name == "citation_neighbors" {
                            payload = reshape_neighbors_payload(payload);
                        }
                        log_runtime_telemetry(&name, &output.stderr);
                        ToolOutcome {
                            ok: true,
                            payload,
                            latency_ms: started.elapsed().as_millis() as u64,
                        }
                    }
                    Err(err) => ToolOutcome {
                        ok: false,
                        payload: err.body(),
                        latency_ms: started.elapsed().as_millis() as u64,
                    },
                }
            }
        }
    }
}

/// Preserve successful CLI diagnostics in the service journal. The subprocess stderr
/// was previously discarded after JSON parsing, which made production phase profiling
/// impossible even though the CLI already emitted structured runtime records.
///
/// Only known, secret-free Calyx runtime prefixes are admitted and every line is
/// length/count bounded. Failure stderr continues through the typed error envelope.
fn log_runtime_telemetry(tool: &str, stderr: &[u8]) {
    const PREFIXES: [&str; 5] = [
        "CALYX_KERNEL_ANSWER_RUNTIME ",
        "CALYX_SEARCH_RUNTIME ",
        "CALYX_SEARCH_SLOTS ",
        "search_slot.done ",
        "CALYX_PANEL_RESIDENT_RUNTIME ",
    ];
    let Ok(text) = std::str::from_utf8(stderr) else {
        return;
    };
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| PREFIXES.iter().any(|prefix| line.starts_with(prefix)))
        .take(128)
    {
        let bounded: String = line.chars().take(4096).collect();
        tracing::info!(tool, runtime = %bounded, "lawdemo tool runtime");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            vault: "/zfs/PROD".into(),
            resident_addr: "127.0.0.1:18401".into(),
            d1_vault: "/zfs/D1".into(),
            d1_kernel: "kd1".repeat(11), // 33 chars, valid-ish id shape
            d2_vault: "/zfs/D2".into(),
            d2_kernel: "kd2".repeat(11),
            search_k_max: 20,
        }
    }

    #[test]
    fn every_registered_tool_has_a_spec_and_builds_or_needs_args() {
        let specs = tool_specs();
        assert_eq!(specs.len(), TOOL_NAMES.len());
        for spec in &specs {
            let n = spec["function"]["name"].as_str().unwrap();
            assert!(TOOL_NAMES.contains(&n), "spec for unregistered tool {n}");
        }
    }

    #[test]
    fn corpus_search_argv_is_exact_and_clamps_k() {
        let argv = build_argv(
            &ctx(),
            "corpus_search",
            &json!({"query":"traffic stop","k":9999}),
        )
        .unwrap();
        assert_eq!(argv[0], "search");
        assert_eq!(argv[1], "/zfs/PROD");
        assert_eq!(argv[2], "traffic stop");
        assert_eq!(argv[3], "--k");
        assert_eq!(argv[4], "20"); // clamped to search_k_max
        assert!(argv.contains(&"--provenance".to_string()));
        assert_eq!(argv.last().unwrap(), "127.0.0.1:18401");
    }

    #[test]
    fn injection_query_is_a_single_argv_element() {
        // Shell metacharacters, flags, and a fake path must all stay INSIDE one argv
        // element (position 2) — never split, never a new flag, never a shell.
        let evil = "'; rm -rf / #$(reboot) --resident-addr 6.6.6.6:1 && curl evil";
        let argv = build_argv(&ctx(), "corpus_search", &json!({"query":evil})).unwrap();
        assert_eq!(argv[2], evil, "the whole payload is one argv element");
        // Exactly one --resident-addr (the trusted one), never the injected one.
        assert_eq!(argv.iter().filter(|a| *a == "--resident-addr").count(), 1);
        assert_eq!(argv.last().unwrap(), "127.0.0.1:18401");
        // The vault position is the trusted vault, untouched by the payload.
        assert_eq!(argv[1], "/zfs/PROD");
    }

    #[test]
    fn case_read_rejects_flaglike_and_path_cx_ids() {
        assert!(build_argv(&ctx(), "case_read", &json!({"cx_id":"--record"})).is_err());
        assert!(build_argv(&ctx(), "case_read", &json!({"cx_id":"../../etc/passwd"})).is_err());
        assert!(build_argv(&ctx(), "case_read", &json!({"cx_id":"x"})).is_err()); // too short
        let ok = build_argv(
            &ctx(),
            "case_read",
            &json!({"cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b"}),
        )
        .unwrap();
        assert_eq!(
            ok,
            vec![
                "readback",
                "cx-list",
                "--vault",
                "/zfs/PROD",
                "--cx-id",
                "8f9b8ff1e2ed614e732c0e0f4e1ac62b"
            ]
        );
    }

    #[test]
    fn kernel_inspect_maps_which_to_scoped_vaults() {
        let d = build_argv(&ctx(), "kernel_inspect", &json!({"which":"doctrine"})).unwrap();
        assert_eq!(d[3], "/zfs/D1");
        let j = build_argv(&ctx(), "kernel_inspect", &json!({"which":"judge"})).unwrap();
        assert_eq!(j[3], "/zfs/D2");
        assert!(build_argv(&ctx(), "kernel_inspect", &json!({"which":"delaware"})).is_err());
    }

    #[test]
    fn citation_graph_defaults_and_allowlists_collection() {
        // #1977: the accepted prod overlay is legal-citations-alias-v2, so the tool must
        // default there (the old legal-citations-v2 default read empty on the new vault).
        let def = build_argv(&ctx(), "citation_graph", &json!({})).unwrap();
        assert_eq!(def.last().unwrap(), "legal-citations-alias-v2");
        assert_eq!(def.last().unwrap(), DEFAULT_CITATION_COLLECTION);
        // The alias overlay and the legacy names are all allowlisted; anything else fails.
        for c in [
            "legal-citations-v1",
            "legal-citations-v2",
            "legal-citations-alias-v2",
        ] {
            let argv = build_argv(&ctx(), "citation_graph", &json!({ "collection": c })).unwrap();
            assert_eq!(argv.last().unwrap(), c);
        }
        assert!(build_argv(&ctx(), "citation_graph", &json!({"collection":"evil"})).is_err());
        // corpus_stats citation_overlay uses the same accepted collection.
        let stats = build_argv(
            &ctx(),
            "corpus_stats",
            &json!({"dimension":"citation_overlay"}),
        )
        .unwrap();
        assert_eq!(stats.last().unwrap(), DEFAULT_CITATION_COLLECTION);
    }

    #[test]
    fn corpus_stats_dimension_is_allowlisted() {
        assert_eq!(
            build_argv(&ctx(), "corpus_stats", &json!({"dimension":"manifest"})).unwrap(),
            vec!["verify-chain", "/zfs/PROD"]
        );
        // opinion_types is now a live census verb.
        assert_eq!(
            build_argv(
                &ctx(),
                "corpus_stats",
                &json!({"dimension":"opinion_types"})
            )
            .unwrap(),
            vec!["readback", "opinion-type-census", "--vault", "/zfs/PROD"]
        );
        assert!(build_argv(&ctx(), "corpus_stats", &json!({"dimension":"hallucinated"})).is_err());
    }

    #[test]
    fn case_text_argv_is_exact_with_optional_window() {
        let cx = "8f9b8ff1e2ed614e732c0e0f4e1ac62b";
        let argv = build_argv(&ctx(), "case_text", &json!({"cx_id":cx})).unwrap();
        assert_eq!(
            argv,
            vec![
                "readback",
                "case-text",
                "--vault",
                "/zfs/PROD",
                "--cx-id",
                cx
            ]
        );
        let argv = build_argv(
            &ctx(),
            "case_text",
            &json!({"cx_id":cx,"max_bytes":64,"offset":10}),
        )
        .unwrap();
        assert!(argv.contains(&"--max-bytes".to_string()) && argv.contains(&"64".to_string()));
        assert!(argv.contains(&"--offset".to_string()) && argv.contains(&"10".to_string()));
        // A flag-like cx_id is still rejected.
        assert!(build_argv(&ctx(), "case_text", &json!({"cx_id":"--record"})).is_err());
    }

    #[test]
    fn citation_neighbors_argv_validates_direction_and_defaults_collection() {
        let cx = "8f9b8ff1e2ed614e732c0e0f4e1ac62b";
        let argv = build_argv(
            &ctx(),
            "citation_neighbors",
            &json!({"cx_id":cx,"direction":"both"}),
        )
        .unwrap();
        assert_eq!(argv[0], "readback");
        assert_eq!(argv[1], "citation-neighbors");
        assert!(argv.contains(&"--direction".to_string()) && argv.contains(&"both".to_string()));
        // collection defaults to the accepted alias overlay.
        assert!(argv.contains(&"legal-citations-alias-v2".to_string()));
        // Bad direction / bad collection are typed errors.
        assert!(
            build_argv(
                &ctx(),
                "citation_neighbors",
                &json!({"cx_id":cx,"direction":"sideways"})
            )
            .is_err()
        );
        assert!(
            build_argv(
                &ctx(),
                "citation_neighbors",
                &json!({"cx_id":cx,"direction":"in","collection":"evil"})
            )
            .is_err()
        );
    }

    #[test]
    fn cite_check_argv_is_db_native_with_optional_proposition() {
        let argv =
            build_argv(&ctx(), "cite_check", &json!({"case_name":"State v. Smith"})).unwrap();
        assert_eq!(
            argv,
            vec![
                "cite-check",
                "--vault",
                "/zfs/PROD",
                "--case-name",
                "State v. Smith"
            ]
        );
        // No resident-addr is used — it is a DB-native probe now.
        assert!(!argv.iter().any(|a| a == "--resident-addr"));
        let argv = build_argv(
            &ctx(),
            "cite_check",
            &json!({"case_name":"State v. Smith","proposition":"holds X"}),
        )
        .unwrap();
        assert!(
            argv.contains(&"--proposition".to_string()) && argv.contains(&"holds X".to_string())
        );
    }

    #[test]
    fn judge_census_argv_passes_free_text_judge() {
        let argv = build_argv(&ctx(), "judge_census", &json!({"judge":"Boyle"})).unwrap();
        assert_eq!(
            argv,
            vec![
                "readback",
                "judge-census",
                "--vault",
                "/zfs/PROD",
                "--judge",
                "Boyle"
            ]
        );
        assert!(build_argv(&ctx(), "judge_census", &json!({"judge":""})).is_err());
    }

    #[test]
    fn all_tools_use_the_same_live_budget() {
        let mut map = crate::config::base_env_map();
        map.insert("LAWDEMO_WEB_LIVE_TIMEOUT_SECS".into(), "120".into());
        let cfg = crate::config::Config::from_map(&map).unwrap();

        for tool in [
            "kernel_answer",
            "reproduce",
            "kernel_inspect",
            "judge_profile",
            "corpus_search",
            "case_read",
            "ledger_tail",
            "cite_check",
        ] {
            assert_eq!(
                tool_timeout(&cfg, tool),
                std::time::Duration::from_secs(120)
            );
        }
    }

    #[test]
    fn unknown_tool_is_typed_error() {
        let err = build_argv(&ctx(), "hallucinated_tool", &json!({})).unwrap_err();
        assert_eq!(err.code(), ErrorCode::ChatToolUnknown);
    }

    #[test]
    fn shrink_case_payload_drops_base_hex() {
        let payload = json!([{"cx_id":"abc","base_hex":"deadbeef".repeat(1000),"metadata":{"case_name":"X"}}]);
        let out = shrink_case_payload(payload, "case_read", None);
        assert!(out[0].get("base_hex").is_none());
        assert_eq!(out[0]["metadata"]["case_name"], "X");
    }

    /// A REAL recorded `calyx search` stderr transcript (the exact line families emitted
    /// by `roster.rs::emit_runtime_line` + `engine.rs::emit_search_trace`), captured to a
    /// fixture so the lens-parsing enrichment is tested against real bytes, not a mock.
    const SEARCH_STDERR: &str =
        include_str!("../../tests/fixtures/search/corpus_search_stderr.txt");

    #[test]
    fn parse_search_lens_provenance_reads_real_roster_and_timings() {
        let lens = parse_search_lens_provenance(SEARCH_STDERR.as_bytes())
            .expect("roster + timing lines present in the real fixture");
        // Ten learned lenses served on the GPU; the ColBERT lens is honestly excluded.
        let resident = lens["resident_gpu"].as_array().unwrap();
        assert_eq!(resident.len(), 10);
        assert_eq!(lens["lens_count"], 10);
        assert_eq!(resident[0]["slot"], 1);
        assert_eq!(
            resident[0]["key"],
            "semantic_multilingual_e5_base_fastembed"
        );
        let parked = lens["parked_excluded"].as_array().unwrap();
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0]["slot"], 4);
        assert_eq!(parked[0]["key"], "a38_bgem3_colbert");
        assert!(lens["local_cpu"].as_array().unwrap().is_empty());
        // Per-slot timings carry the real backend, GPU/CPU placement, and latency.
        let timings = lens["slot_timings"].as_array().unwrap();
        assert_eq!(timings.len(), 10);
        let sparse = timings
            .iter()
            .find(|t| t["slot"] == 3)
            .expect("slot 3 timing");
        assert_eq!(sparse["backend"], "cuda-sparse-bm25");
        assert_eq!(sparse["placement"], "cuda:0");
        assert_eq!(sparse["slot_elapsed_ms"], 18);
        assert_eq!(sparse["count"], 8);
    }

    #[test]
    fn enrich_search_payload_wraps_hits_and_attaches_lens_provenance() {
        let payload = json!([{"rank":1,"cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b","score":0.84}]);
        let out = enrich_search_payload(payload, SEARCH_STDERR.as_bytes());
        // The ranked hits are preserved (headline + citation collection stay correct).
        assert_eq!(out["hits"][0]["cx_id"], "8f9b8ff1e2ed614e732c0e0f4e1ac62b");
        assert_eq!(out["lens_provenance"]["lens_count"], 10);
        // The wrapped shape still yields the real headline.
        let h = headline("corpus_search", true, &out);
        assert!(h.contains("1 hits"), "got: {h}");
        // cx_ids are still collectable from the wrapped payload (grounding intact).
        let mut set = std::collections::BTreeSet::new();
        crate::chat::citations::collect_cx_ids(&out, &mut set);
        assert!(set.contains("8f9b8ff1e2ed614e732c0e0f4e1ac62b"));
    }

    #[test]
    fn enrich_search_payload_is_identity_without_roster_lines() {
        // No CALYX_SEARCH_SLOTS / search_slot.done lines → payload unchanged (fail-open
        // to the model's existing array contract, never a fabricated roster).
        let payload = json!([{"rank":1,"cx_id":"abcdef01","score":0.5}]);
        let out = enrich_search_payload(payload.clone(), b"some unrelated stderr noise\n");
        assert_eq!(out, payload);
        assert!(parse_search_lens_provenance(b"nothing here").is_none());
    }

    /// A REAL `readback citation-neighbors` document (in-corpus `cites` rows captured
    /// live from prod for State v. Jackson, plus a `cites_outside_corpus` frontier row in
    /// the exact CLI shape). The reshape must make the frontier ref unconfusable.
    const NEIGHBORS_FIXTURE: &str =
        include_str!("../../tests/fixtures/citation/neighbors_frontier.json");

    #[test]
    fn reshape_neighbors_makes_frontier_refs_unconfusable_with_readable_cx_ids() {
        let raw: Value = serde_json::from_str(NEIGHBORS_FIXTURE).unwrap();
        // The frontier node id present in the RAW payload — a clean 32-hex that looks
        // exactly like a readable opinion cx_id (the #1977 trap).
        let frontier_id = "9f7b1c3ad2e04455aa8f0b6612cd7788";
        let mut raw_ids = std::collections::BTreeSet::new();
        crate::chat::citations::collect_cx_ids(&raw, &mut raw_ids);
        assert!(
            raw_ids.contains(frontier_id),
            "precondition: the raw CLI payload DOES leak the frontier id as a citable cx_id"
        );

        let out = reshape_neighbors_payload(raw);
        let neighbors = out["neighbors"].as_array().unwrap();
        assert_eq!(neighbors.len(), 3);

        // The model-facing payload must carry NO confusable field for the frontier ref:
        // no `cx_id`, an explicit outside_corpus:true, readable_cx_id:null, and the ref
        // under the differently-named `frontier_ref` (with the sentinel prefix).
        let frontier = neighbors
            .iter()
            .find(|n| n["edge_type"] == "cites_outside_corpus")
            .expect("frontier neighbor present");
        assert!(
            frontier.get("cx_id").is_none(),
            "frontier must not carry a cx_id"
        );
        assert_eq!(frontier["outside_corpus"], true);
        assert_eq!(frontier["readable_cx_id"], Value::Null);
        assert_eq!(
            frontier["frontier_ref"],
            format!("{FRONTIER_REF_PREFIX}{frontier_id}")
        );
        assert_eq!(frontier["external_opinion_id"], "118143");
        assert!(frontier["_note"].as_str().unwrap().contains("cite_check"));

        // In-corpus neighbors stay readable: cx_id kept, mirrored, outside_corpus:false.
        for n in neighbors.iter().filter(|n| n["edge_type"] == "cites") {
            assert!(n.get("cx_id").is_some());
            assert_eq!(n["readable_cx_id"], n["cx_id"]);
            assert_eq!(n["outside_corpus"], false);
        }

        // The grounding set collected from the reshaped payload must NOT include the
        // frontier id (it can never be cited as a real opinion), but MUST still include
        // the readable in-corpus neighbors.
        let mut ids = std::collections::BTreeSet::new();
        crate::chat::citations::collect_cx_ids(&out, &mut ids);
        assert!(
            !ids.contains(frontier_id),
            "the frontier id must NOT be a citable cx_id after reshape"
        );
        assert!(ids.contains("20277925748567bbbfba6c8104ac7776"));
        assert!(ids.contains("e11cd8a4f6b34a7290c5d0e3b7188a90"));
    }

    #[test]
    fn reshape_neighbors_is_identity_without_neighbors_array() {
        // A forwarded error envelope (no `neighbors`) passes through untouched.
        let err = json!({"code":"CALYX_GRAPH_COLLECTION_REBUILD_STALE","message":"x"});
        assert_eq!(reshape_neighbors_payload(err.clone()), err);
    }

    #[test]
    fn case_read_frontier_ref_yields_actionable_error_not_generic_shape_error() {
        // A model that mistakenly feeds a frontier_ref to case_read gets a SPECIFIC
        // correction (existence → cite_check), never the generic token-shape rejection.
        let frontier_ref = format!("{FRONTIER_REF_PREFIX}9f7b1c3ad2e04455aa8f0b6612cd7788");
        for tool in ["case_read", "case_text", "case_segments"] {
            let err = build_argv(&ctx(), tool, &json!({ "cx_id": frontier_ref })).unwrap_err();
            assert_eq!(err.code(), ErrorCode::ChatToolArg);
            let msg = err.body()["message"].as_str().unwrap().to_owned();
            assert!(msg.contains("out-of-corpus"), "{tool}: {msg}");
            assert!(msg.contains("cite_check"), "{tool}: {msg}");
        }
    }

    #[test]
    fn headline_reports_real_numbers_only() {
        let h = headline("corpus_search", true, &json!([{"cx_id":"a","score":0.84}]));
        assert!(h.contains("1 hits"));
        let e = headline(
            "kernel_answer",
            false,
            &json!({"code":"CALYX_KERNEL_QUERY_OUT_OF_SCOPE"}),
        );
        assert!(e.contains("CALYX_KERNEL_QUERY_OUT_OF_SCOPE"));
    }
}
