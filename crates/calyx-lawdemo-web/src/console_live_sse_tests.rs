//! Reality contract for the Research Console Inspector, pinned to REAL live SSE.
//!
//! The console (`web/console.html`) was originally verified only against the `?demo=1`
//! recorded replay. During real live episodes on the deployed service the GPU-resident
//! search and the sealed kernel are frequently unavailable, so the agent grounds answers
//! through the DB-native tools (`cite_check`, `case_read`, `case_text`) instead. The
//! Inspector bound each tab ONLY to its ideal tool's success payload, so every tab sat
//! empty in those episodes.
//!
//! These fixtures are verbatim transcripts captured from the live agent on the deployed
//! GPU host (loopback `POST /api/chat`): one cite-check-heavy episode and one
//! search-heavy episode. The ONLY edits satisfy the repo's own commit gates and touch
//! no field any tab binds to: the developer home path is neutralized
//! (`/home/<user>/calyx` -> `/var/lib/calyx`) per `scripts/public-leak-scan.sh`, and the
//! high-entropy per-opinion `key_hex` vault content key in each `case_read` row is zeroed
//! so `secret-scan.sh` (gitleaks) passes. Every shape-relevant field is untouched. This
//! module asserts — field by
//! field, against the real bytes — that each Inspector tab has at least one REAL data
//! source in each episode, i.e. the fields the fixed `captureEvidence`/tab renderers
//! bind to are present end-to-end. If a future change truncates or reshapes a
//! tool-result payload before the SPA sees it, this test fails and names the tab that
//! would go dark.

use serde_json::Value;

const BONNELL: &str = include_str!("../tests/fixtures/console/live_sse_1_bonnell.txt");
const TRAFFIC_STOP: &str = include_str!("../tests/fixtures/console/live_sse_2_traffic_stop.txt");

/// One decoded SSE event: the `event:` name and its parsed `data:` JSON.
struct Event {
    name: String,
    data: Value,
}

/// Parse an `event: <name>\ndata: <json>\n\n` SSE transcript into decoded events,
/// exactly the way the console's `fetch`+`ReadableStream` reader splits frames.
fn parse_sse(transcript: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for frame in transcript.split("\n\n") {
        let mut name = None;
        let mut data = String::new();
        for line in frame.split('\n') {
            if let Some(rest) = line.strip_prefix("event:") {
                name = Some(rest.trim().to_owned());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim());
            }
        }
        if let Some(name) = name {
            if data.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                out.push(Event { name, data: v });
            }
        }
    }
    out
}

fn tool_results<'a>(events: &'a [Event], tool: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e.name == "tool_result" && e.data["name"] == tool)
        .map(|e| &e.data)
        .collect()
}

fn first_event<'a>(events: &'a [Event], name: &str) -> Option<&'a Value> {
    events.iter().find(|e| e.name == name).map(|e| &e.data)
}

/// True if `tx` contains an `sk-`-prefixed token followed by 20+ alphanumeric chars —
/// the shape of a real API key, distinct from incidental prose like "risk-benefit".
fn looks_like_api_key(tx: &str) -> bool {
    tx.match_indices("sk-").any(|(i, _)| {
        tx[i + 3..]
            .chars()
            .take(20)
            .filter(|c| c.is_ascii_alphanumeric())
            .count()
            == 20
    })
}

/// The first `case_read`/`case_segments` row (the payload is a cx-list ARRAY).
fn first_row(payload: &Value) -> &Value {
    payload
        .as_array()
        .and_then(|a| a.first())
        .unwrap_or(payload)
}

#[test]
fn both_fixtures_are_wellformed_terminal_episodes() {
    for (label, tx) in [("bonnell", BONNELL), ("traffic_stop", TRAFFIC_STOP)] {
        let evs = parse_sse(tx);
        assert!(evs.len() > 5, "{label}: too few events ({})", evs.len());
        // The transport emits `opened` (episode id for cancellation) BEFORE the agent
        // loop's `episode_start`; the stream is terminated by `done`.
        assert_eq!(evs.first().unwrap().name, "opened", "{label}");
        assert!(
            evs.iter().any(|e| e.name == "episode_start"),
            "{label}: no episode_start"
        );
        assert_eq!(evs.last().unwrap().name, "done", "{label}");
        assert!(first_event(&evs, "answer").is_some(), "{label}: no answer");
        assert!(
            first_event(&evs, "validation").is_some(),
            "{label}: no validation"
        );
        // Transparency invariant: no secret ever appears on the wire. Match only real
        // secret SHAPES — a DeepSeek `sk-<20+ alnum>` key, or an HTTP auth header
        // (`authorization:` / `bearer `) — never the bare English word "authorization"
        // or an incidental `sk-` inside prose like "risk-benefit".
        let low = tx.to_lowercase();
        assert!(
            !looks_like_api_key(tx) && !low.contains("authorization:") && !low.contains("bearer "),
            "{label}: possible secret leaked into the SSE transcript"
        );
    }
}

/// SOURCES tab: verified citations carry display provenance, and the agent also read
/// opinions (`case_read`/`case_text`) + proved existence by name (`cite_check`) — every
/// one of which the fixed Sources tab renders (verified cards + read-but-uncited cards).
#[test]
fn sources_tab_has_real_data_in_both_episodes() {
    for (label, tx) in [("bonnell", BONNELL), ("traffic_stop", TRAFFIC_STOP)] {
        let evs = parse_sse(tx);

        let cites = first_event(&evs, "citations").expect("citations event");
        let items = cites["items"].as_array().expect("citation items");
        assert!(!items.is_empty(), "{label}: no citation items");
        for it in items {
            assert!(
                it["case_name"].is_string(),
                "{label}: citation w/o case_name"
            );
            assert!(it["cx_id"].is_string(), "{label}: citation w/o cx_id");
            assert!(
                it["ledger_seq"].as_u64().is_some(),
                "{label}: citation w/o ledger_seq (Provenance anchor would be missing)"
            );
        }

        // case_read rows are ARRAYS carrying metadata.case_name + provenance.seq.
        let reads = tool_results(&evs, "case_read");
        assert!(!reads.is_empty(), "{label}: no case_read results");
        for r in &reads {
            let row = first_row(&r["payload"]);
            assert!(
                row["metadata"]["case_name"].is_string(),
                "{label}: case_read row missing metadata.case_name"
            );
            assert!(
                row["provenance"]["seq"].as_u64().is_some(),
                "{label}: case_read row missing provenance.seq (Provenance anchor)"
            );
        }

        // case_text carries case_name + text + total_bytes (the decoded excerpt cards).
        for c in tool_results(&evs, "case_text") {
            let p = &c["payload"];
            assert!(
                p["case_name"].is_string(),
                "{label}: case_text w/o case_name"
            );
            assert!(p["text"].is_string(), "{label}: case_text w/o text");
            assert!(
                p["total_bytes"].as_u64().is_some(),
                "{label}: case_text w/o total_bytes"
            );
        }

        // cite_check found[] leads carry cx_id + case_name (the "matched by name" cards).
        for c in tool_results(&evs, "cite_check") {
            if c["ok"].as_bool() == Some(true)
                && let Some(found) = c["payload"]["found"].as_array()
            {
                for f in found {
                    assert!(
                        f["cx_id"].is_string(),
                        "{label}: cite_check found w/o cx_id"
                    );
                    assert!(
                        f["case_name"].is_string(),
                        "{label}: cite_check found w/o case_name"
                    );
                }
            }
        }
    }
}

/// PROVENANCE tab: after any episode that touched the corpus the tab must not be empty.
/// Every fixture surfaces per-source chain anchors (citation ledger_seq + case_read
/// provenance.seq), so the anchor list always has rows.
#[test]
fn provenance_tab_has_chain_anchors_in_both_episodes() {
    for (label, tx) in [("bonnell", BONNELL), ("traffic_stop", TRAFFIC_STOP)] {
        let evs = parse_sse(tx);
        let cites = first_event(&evs, "citations").unwrap();
        let anchor_from_cite = cites["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["ledger_seq"].as_u64().is_some())
            .count();
        let anchor_from_read = tool_results(&evs, "case_read")
            .iter()
            .filter(|r| {
                first_row(&r["payload"])["provenance"]["seq"]
                    .as_u64()
                    .is_some()
            })
            .count();
        assert!(
            anchor_from_cite + anchor_from_read > 0,
            "{label}: Provenance tab would be empty — no chain anchors"
        );
    }
}

/// LENSES / GRAPH tabs: when the GPU-resident search / sealed kernel are unavailable the
/// failed tool-result still forwards a real, secret-free error code (in `upstream.code`
/// or the leading `{"code":..}` of `upstream.raw_stderr`). The fixed tabs render that as
/// an honest "unavailable this episode" note instead of a blank tab.
#[test]
fn failed_gpu_tools_carry_an_extractable_failure_code() {
    let evs = parse_sse(TRAFFIC_STOP);
    let mut saw_search_fail = false;
    let mut saw_kernel_fail = false;
    for name in ["corpus_search", "kernel_answer"] {
        for r in tool_results(&evs, name) {
            if r["ok"].as_bool() == Some(false) {
                let up = &r["payload"]["upstream"];
                let has_code = up["code"].is_string()
                    || up["raw_stderr"]
                        .as_str()
                        .map(|s| s.contains("\"code\":\""))
                        .unwrap_or(false);
                assert!(
                    has_code,
                    "{name}: failure payload has no extractable code for the honest note"
                );
                if name == "corpus_search" {
                    saw_search_fail = true;
                }
                if name == "kernel_answer" {
                    saw_kernel_fail = true;
                }
            }
        }
    }
    assert!(
        saw_search_fail && saw_kernel_fail,
        "traffic-stop fixture should exercise both a search and a kernel failure"
    );
}

/// The Rust mirror of the console's `failureReason(payload)` (web/console.html): lift a
/// real, secret-free `{code, message}` from a forwarded tool-error payload so the step
/// pill can name exactly WHY a tool was unavailable. Kept in lockstep with the JS so this
/// test proves the exact data the pill now binds (#1977). Order matches the JS:
/// `upstream.code` → embedded code in `upstream.raw_stderr` → top-level `{code,message}`.
fn failure_reason(p: &Value) -> (String, String) {
    let up = &p["upstream"];
    if let Some(code) = up["code"].as_str() {
        return (
            code.to_owned(),
            up["message"].as_str().unwrap_or("").to_owned(),
        );
    }
    if let Some(raw) = up["raw_stderr"].as_str() {
        if let Some((code, message)) = extract_embedded_code(raw) {
            return (code, message);
        }
        return (
            p["code"].as_str().unwrap_or("error").to_owned(),
            raw.chars().take(220).collect(),
        );
    }
    (
        p["code"].as_str().unwrap_or("error").to_owned(),
        p["message"].as_str().unwrap_or("").to_owned(),
    )
}

/// Mirror of the JS regex `/"code":"([A-Z0-9_]+)","message":"([^"]+)"/` over a raw stderr
/// blob (the CLI's own JSON envelope embedded in a stderr line), without a regex dep.
fn extract_embedded_code(raw: &str) -> Option<(String, String)> {
    let ci = raw.find("\"code\":\"")? + 8;
    let clen = raw[ci..].find('"')?;
    let code = &raw[ci..ci + clen];
    if code.is_empty()
        || !code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let mi = raw.find("\"message\":\"")? + 11;
    let mlen = raw[mi..].find('"')?;
    Some((code.to_owned(), raw[mi..mi + mlen].to_owned()))
}

/// STEP PILLS: every failed tool_result in a real episode must yield an extractable
/// CALYX_* code + a non-empty reason — the exact `{code, reason}` the fixed step pill
/// binds (data-code + .serr detail) instead of a bare "unavailable, adapted" (#1977).
#[test]
fn failed_tool_results_yield_a_code_and_reason_for_the_pill() {
    let evs = parse_sse(TRAFFIC_STOP);
    let mut failures = 0;
    for e in &evs {
        if e.name == "tool_result" && e.data["ok"].as_bool() == Some(false) {
            failures += 1;
            let (code, reason) = failure_reason(&e.data["payload"]);
            assert!(
                code.starts_with("CALYX_"),
                "{} failure did not surface a CALYX_* code for the pill (got {code:?})",
                e.data["name"]
            );
            assert!(
                !reason.trim().is_empty(),
                "{} failure surfaced an empty reason; the pill detail would be blank",
                e.data["name"]
            );
        }
    }
    assert!(
        failures > 0,
        "traffic-stop fixture should carry at least one failed tool for the pill contract"
    );
}

/// The `case_read` '⚠ unavailable, adapted · 0ms' defect: a 0ms failure is a PRE-SPAWN
/// arg rejection from `build_argv` (the only latency_ms==0 path). Prove the real error
/// body a bad cx_id produces, and that `failure_reason` (the pill binding) surfaces the
/// exact CALYX_LAWDEMO_CHAT_TOOL_ARG code + a self-correcting reason — so the pill names
/// the cause instead of hiding it (#1977).
#[test]
fn case_read_bad_cx_id_failure_binds_tool_arg_code_for_the_pill() {
    use crate::chat::tools::{ToolContext, build_argv};
    let ctx = ToolContext {
        vault: "/zfs/PROD".into(),
        resident_addr: "127.0.0.1:18401".into(),
        d1_vault: "/zfs/D1".into(),
        d1_kernel: "k".repeat(32),
        d2_vault: "/zfs/D2".into(),
        d2_kernel: "k".repeat(32),
        search_k_max: 20,
    };
    // A frontier / out-of-corpus neighbor ref is NOT a cx_id — the exact 0ms failure.
    // Post-#1977 the reshaped citation_neighbors payload carries these as `frontier:<id>`,
    // and case_read now returns a SPECIFIC, actionable correction (use cite_check) rather
    // than the generic token-shape rejection — so the pill tells the model exactly what
    // to do instead.
    let err = build_argv(
        &ctx,
        "case_read",
        &serde_json::json!({"cx_id":"frontier:rodriguez-v-us"}),
    )
    .expect_err("a frontier ref must be rejected pre-spawn");
    let body = err.body();
    assert_eq!(body["code"], "CALYX_LAWDEMO_CHAT_TOOL_ARG");
    let (code, reason) = failure_reason(&body);
    assert_eq!(code, "CALYX_LAWDEMO_CHAT_TOOL_ARG");
    assert!(
        reason.contains("out-of-corpus") && reason.contains("cite_check"),
        "the pill reason must tell the model to use cite_check for an out-of-corpus ref (got {reason:?})"
    );
    // The message echoes the offending token so the model can fix its next call.
    assert!(
        body["message"].as_str().unwrap().contains("rodriguez-v-us"),
        "the frontier error should echo the bad value the agent supplied"
    );

    // A plain (non-frontier) malformed token still gets the generic shape rejection that
    // names the cx_id contract.
    let generic = build_argv(&ctx, "case_read", &serde_json::json!({"cx_id":"nope!"}))
        .expect_err("a malformed cx_id must be rejected pre-spawn");
    let (_, greason) = failure_reason(&generic.body());
    assert!(
        greason.contains("cx_id must be"),
        "a generic bad cx_id must still explain the token contract (got {greason:?})"
    );
}

/// CORPUS tab: the live episodes here call `corpus_stats` with the MANIFEST dimension
/// (verify-chain), which returns `{status, checked}` — NOT the opinion-type census. The
/// fixed Corpus tab therefore falls back to the SHA-pinned demo-4 census artifact and
/// also surfaces the manifest. Assert the manifest shape the fixed tab reads is present.
#[test]
fn corpus_stats_manifest_shape_is_what_the_corpus_tab_reads() {
    let evs = parse_sse(TRAFFIC_STOP);
    let stats = tool_results(&evs, "corpus_stats");
    assert!(!stats.is_empty(), "traffic-stop: no corpus_stats result");
    let ok_manifest = stats.iter().any(|r| {
        let p = &r["payload"];
        r["ok"].as_bool() == Some(true)
            && p["physical_opinions_n"].is_null() // not the opinion-type census
            && p["status"].is_string() // manifest / verify-chain shape
    });
    assert!(
        ok_manifest,
        "expected a manifest-dimension corpus_stats ({{status, checked}}); \
         the live Corpus tab requires this production state and must refuse its absence"
    );
}
