//! Grounding: the fail-closed differentiator.
//!
//! The agent must cite sources as `[cx:<cx_id>]` markers. The server permits only
//! cx_ids from opinion text successfully read in this conversation. Search hits and
//! case-name matches are research candidates, not citable authority. A cited id
//! outside the retained-text set is ungrounded — the loop asks for one revision and,
//! failing that, refuses with a typed error rather than rendering the answer.

use std::collections::BTreeMap;

use serde_json::Value;

/// Keys in a tool-result payload whose STRING values are real cx_ids the answer may
/// cite. This is deliberately a closed allowlist of cx-bearing fields (not "every
/// string in the payload"), so a case name or date can never be mistaken for a
/// grounded citation token.
const CX_KEYS: [&str; 5] = [
    "cx_id",
    "to_cx_id",
    "from_cx_id",
    "nearest_cx_id",
    "anchor_kernel_node_id",
];

/// Recursively collect every cx_id under a [`CX_KEYS`] field anywhere in `payload`.
pub fn collect_cx_ids(payload: &Value, out: &mut std::collections::BTreeSet<String>) {
    match payload {
        Value::Object(map) => {
            for (k, v) in map {
                if CX_KEYS.contains(&k.as_str())
                    && let Some(s) = v.as_str()
                    && !s.is_empty()
                {
                    out.insert(s.to_owned());
                }
                collect_cx_ids(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_cx_ids(v, out);
            }
        }
        _ => {}
    }
}

/// Best-effort display provenance for a single grounded cx_id, all fields sourced ONLY
/// from real tool payloads (never invented).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CxProvenance {
    /// The `tool_call_id` of the tool call whose result first surfaced this cx_id.
    pub tool_call_id: Option<String>,
    /// The tool that produced it (e.g. `case_read`, `corpus_search`).
    pub tool_name: Option<String>,
    /// Case name if a `case_read`/row payload carried it.
    pub case_name: Option<String>,
    /// Four-digit filing year parsed from a `date_filed`, if present.
    pub year: Option<String>,
    /// Canonical filing date and public identifiers a lawyer can use to retrieve
    /// the original authority independently.
    pub date_filed: Option<String>,
    pub court_id: Option<String>,
    pub court_name: Option<String>,
    pub appellate_district: Option<String>,
    pub state: Option<String>,
    pub docket_number: Option<String>,
    pub opinion_id: Option<String>,
    pub source_url: Option<String>,
    pub source_download_url: Option<String>,
    pub source_dataset: Option<String>,
    pub precedential_status: Option<String>,
    /// Ledger sequence if a search `provenance.ledger_seq` or hop `ledger_ref.seq`
    /// carried it.
    pub ledger_seq: Option<u64>,
}

/// Accumulates the grounding set and per-cx display provenance across a conversation.
#[derive(Debug, Default)]
pub struct GroundingLedger {
    seen: std::collections::BTreeSet<String>,
    citable_text: std::collections::BTreeSet<String>,
    provenance: BTreeMap<String, CxProvenance>,
}

impl GroundingLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once a successful `case_text` read has returned an exact cx_id.
    pub fn has_any(&self) -> bool {
        !self.citable_text.is_empty()
    }

    pub fn seen(&self) -> &std::collections::BTreeSet<String> {
        &self.seen
    }

    /// Exact ids whose retained opinion text was read in this episode. Search hits,
    /// metadata reads, and graph hops are candidates, not citable legal authority.
    pub fn citable_text(&self) -> &std::collections::BTreeSet<String> {
        &self.citable_text
    }

    pub fn provenance_of(&self, cx_id: &str) -> Option<&CxProvenance> {
        self.provenance.get(cx_id)
    }

    /// Ingest one tool result: record every cx_id it surfaced plus any display
    /// provenance we can read directly from the payload.
    pub fn ingest(&mut self, tool_call_id: &str, tool_name: &str, payload: &Value) {
        let mut ids = std::collections::BTreeSet::new();
        collect_cx_ids(payload, &mut ids);
        for id in &ids {
            self.seen.insert(id.clone());
            if tool_name == "case_text" {
                self.citable_text.insert(id.clone());
            }
            let entry = self.provenance.entry(id.clone()).or_default();
            if entry.tool_call_id.is_none() {
                entry.tool_call_id = Some(tool_call_id.to_owned());
                entry.tool_name = Some(tool_name.to_owned());
            }
        }
        // Row-level provenance: walk objects that carry a cx_id alongside metadata /
        // provenance, and enrich that cx's display record.
        self.enrich(payload);
    }

    fn enrich(&mut self, payload: &Value) {
        match payload {
            Value::Object(map) => {
                if let Some(cx) = map.get("cx_id").and_then(Value::as_str)
                    && let Some(entry) = self.provenance.get_mut(cx)
                {
                    enrich_source_fields(entry, map);
                    if let Some(meta) = map.get("metadata").and_then(Value::as_object) {
                        enrich_source_fields(entry, meta);
                    }
                    if let Some(source) = map.get("source").and_then(Value::as_object) {
                        enrich_source_fields(entry, source);
                    }
                    if entry.ledger_seq.is_none() {
                        entry.ledger_seq = map
                            .get("provenance")
                            .and_then(|p| p.get("seq").or_else(|| p.get("ledger_seq")))
                            .and_then(Value::as_u64);
                    }
                }
                // Hop rows: {to_cx_id, ledger_ref:{seq}}
                if let Some(cx) = map.get("to_cx_id").and_then(Value::as_str)
                    && let Some(entry) = self.provenance.get_mut(cx)
                    && entry.ledger_seq.is_none()
                {
                    entry.ledger_seq = map
                        .get("ledger_ref")
                        .and_then(|r| r.get("seq"))
                        .and_then(Value::as_u64);
                }
                for v in map.values() {
                    self.enrich(v);
                }
            }
            Value::Array(items) => {
                for v in items {
                    self.enrich(v);
                }
            }
            _ => {}
        }
    }
}

fn enrich_source_fields(entry: &mut CxProvenance, fields: &serde_json::Map<String, Value>) {
    fn fill(target: &mut Option<String>, fields: &serde_json::Map<String, Value>, key: &str) {
        if target.is_none() {
            *target = fields
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned);
        }
    }
    fill(&mut entry.case_name, fields, "case_name");
    fill(&mut entry.date_filed, fields, "date_filed");
    fill(&mut entry.court_id, fields, "court_id");
    fill(&mut entry.court_name, fields, "jurisdiction_court");
    fill(&mut entry.court_name, fields, "court_name");
    fill(
        &mut entry.appellate_district,
        fields,
        "jurisdiction_appellate_district",
    );
    fill(&mut entry.appellate_district, fields, "appellate_district");
    fill(&mut entry.state, fields, "jurisdiction_state");
    fill(&mut entry.state, fields, "state");
    fill(&mut entry.docket_number, fields, "docket_number");
    fill(&mut entry.opinion_id, fields, "opinion_id");
    fill(&mut entry.source_url, fields, "source_url");
    fill(&mut entry.source_url, fields, "url");
    fill(
        &mut entry.source_download_url,
        fields,
        "courtlistener_download_url",
    );
    fill(&mut entry.source_download_url, fields, "download_url");
    fill(&mut entry.source_dataset, fields, "source_dataset");
    fill(
        &mut entry.precedential_status,
        fields,
        "precedential_status",
    );
    if entry.year.is_none() {
        entry.year = entry.date_filed.as_deref().and_then(year_of);
    }
}

/// Parse the leading 4-digit year from a `YYYY-MM-DD` string.
fn year_of(date: &str) -> Option<String> {
    let y = date.get(0..4)?;
    if y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit()) {
        Some(y.to_owned())
    } else {
        None
    }
}

/// Extract the raw cx_id tokens from every `[cx:<id>]` marker in `answer`, in order of
/// appearance, de-duplicated. Tokens are trimmed; a marker with an empty/oversized/
/// illegal token is still returned (so validation can reject it) but capped in length.
pub fn extract_markers(answer: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = answer.as_bytes();
    let needle = b"[cx:";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let start = i + needle.len();
            if let Some(rel) = answer[start..].find(']') {
                let token = answer[start..start + rel].trim();
                let token: String = token.chars().take(160).collect();
                if !out.contains(&token) {
                    out.push(token);
                }
                i = start + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The verdict of validating an answer's citations against the grounding set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationVerdict {
    /// Every distinct token cited in the answer.
    pub cited: Vec<String>,
    /// Cited tokens that DID appear in a tool result (grounded).
    pub valid: Vec<String>,
    /// Cited tokens that never appeared in any tool result (fabricated).
    pub invalid: Vec<String>,
}

impl CitationVerdict {
    pub fn is_grounded(&self) -> bool {
        self.invalid.is_empty()
    }
    pub fn has_citations(&self) -> bool {
        !self.cited.is_empty()
    }
}

/// Validate `answer`'s `[cx:]` markers against ids whose retained text was actually read.
pub fn validate(answer: &str, ledger: &GroundingLedger) -> CitationVerdict {
    let cited = extract_markers(answer);
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for token in &cited {
        if ledger.citable_text().contains(token) {
            valid.push(token.clone());
        } else {
            invalid.push(token.clone());
        }
    }
    CitationVerdict {
        cited,
        valid,
        invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_cx_ids_from_search_and_hops() {
        let payload = json!({
            "hits": [{"cx_id": "aaaa1111"}, {"cx_id": "bbbb2222"}],
            "hops": [{"from_cx_id": "cccc3333", "to_cx_id": "dddd4444"}],
            "metadata": {"case_name": "State v. Example"}
        });
        let mut set = std::collections::BTreeSet::new();
        collect_cx_ids(&payload, &mut set);
        assert!(set.contains("aaaa1111"));
        assert!(set.contains("dddd4444"));
        // The case name is NOT a cx_id and must never enter the grounding set.
        assert!(!set.contains("State v. Example"));
        // aaaa1111, bbbb2222, cccc3333 (from_cx_id), dddd4444 (to_cx_id) = 4 distinct.
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn extract_markers_dedupes_and_orders() {
        let answer = "A [cx:aaaa1111] then B [cx:bbbb2222] then A again [cx:aaaa1111].";
        assert_eq!(extract_markers(answer), vec!["aaaa1111", "bbbb2222"]);
    }

    #[test]
    fn fabricated_cx_id_is_flagged_invalid() {
        let mut ledger = GroundingLedger::new();
        ledger.ingest(
            "call_1",
            "corpus_search",
            &json!({"hits":[{"cx_id":"real0001"}]}),
        );
        let verdict = validate(
            "Holds X [cx:real0001] but also [cx:fabricated999].",
            &ledger,
        );
        assert_eq!(verdict.valid, vec!["real0001"]);
        assert_eq!(verdict.invalid, vec!["fabricated999"]);
        assert!(!verdict.is_grounded());
    }

    #[test]
    fn fully_grounded_answer_passes() {
        let mut ledger = GroundingLedger::new();
        ledger.ingest("c", "case_read", &json!({"cx_id":"real0001"}));
        let verdict = validate("Holds X [cx:real0001].", &ledger);
        assert!(verdict.is_grounded());
        assert!(verdict.has_citations());
    }

    #[test]
    fn provenance_reads_case_name_and_year_from_payload_only() {
        let mut ledger = GroundingLedger::new();
        ledger.ingest(
            "call_7",
            "case_read",
            &json!({
                "cx_id": "abc123",
                "metadata": {"case_name": "City of Cleveland v. Kalish", "date_filed": "2015-03-12"},
                "provenance": {"seq": 41022}
            }),
        );
        let p = ledger.provenance_of("abc123").unwrap();
        assert_eq!(p.case_name.as_deref(), Some("City of Cleveland v. Kalish"));
        assert_eq!(p.year.as_deref(), Some("2015"));
        assert_eq!(p.ledger_seq, Some(41022));
        assert_eq!(p.tool_name.as_deref(), Some("case_read"));
        assert_eq!(p.tool_call_id.as_deref(), Some("call_7"));
    }
}
