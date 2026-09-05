//! Vault-free, OS-independent unit tests: argv contracts + known-outcome
//! aggregation/projection cores with crafted synthetic data. The real-vault
//! end-to-end path is exercised by `tests/issue1962_law_readback_fsv.rs`
//! (Unix-only at runtime; see that file).

use std::collections::BTreeMap;

use calyx_aster::plain_graph::PlainGraphEdge;
use calyx_core::CxId;

use super::*;

fn cx(n: u8) -> CxId {
    let mut bytes = [0u8; 16];
    bytes[15] = n;
    CxId::from_bytes(bytes)
}

fn meta(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn tokens(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

// ---- opinion-type census ---------------------------------------------------

#[test]
fn census_counts_dissent_and_concurrence_family_exactly() {
    // 5 live opinions: 2 dissents, 1 concurrence, 1 concurrence-in-part, 1 lead;
    // plus one tombstone that must not inflate physical_opinions_n.
    let mut c = OpinionTypeCensus::default();
    c.observe_live(&meta(&[("opinion_type", "040dissent")]));
    c.observe_live(&meta(&[("opinion_type", "040dissent")]));
    c.observe_live(&meta(&[("opinion_type", "030concurrence")]));
    c.observe_live(&meta(&[("opinion_type", "035concurrenceinpart")]));
    c.observe_live(&meta(&[("opinion_type", "020lead")]));
    c.observe_tombstone();

    let v = c.to_json(std::path::Path::new("/v"));
    assert_eq!(v["physical_opinions_n"], 5);
    assert_eq!(v["tombstoned_n"], 1);
    assert_eq!(v["dissents_n"], 2);
    assert_eq!(v["concurrence_n"], 1);
    assert_eq!(v["concurrence_in_part_n"], 1);
    assert_eq!(v["concurrence_family_n"], 2);
    assert_eq!(v["by_type"]["020lead"], 1);
    assert_eq!(v["missing_opinion_type_n"], 0);
}

#[test]
fn census_tracks_missing_opinion_type() {
    let mut c = OpinionTypeCensus::default();
    c.observe_live(&meta(&[("case_name", "X")]));
    c.observe_live(&meta(&[("opinion_type", "  ")]));
    let v = c.to_json(std::path::Path::new("/v"));
    assert_eq!(v["physical_opinions_n"], 2);
    assert_eq!(v["missing_opinion_type_n"], 2);
    assert_eq!(v["dissents_n"], 0);
}

// ---- cite-check ------------------------------------------------------------

#[test]
fn cite_check_finds_planted_and_reports_not_in_corpus() {
    let planted = meta(&[
        ("case_name", "State v. Planted"),
        ("case_name_full", "State of Ohio v. Planted"),
    ]);
    let other = meta(&[("case_name", "City of Cleveland v. Kalish")]);

    // Exact match on the planted name.
    let mut found = CiteChecker::new("State v. Planted");
    found.observe(cx(1), &planted);
    found.observe(cx(2), &other);
    let v = found.to_json(&CiteCheckArgs {
        vault: "/v".into(),
        case_name: "State v. Planted".into(),
        proposition: None,
    });
    assert_eq!(v["status"], "found");
    assert_eq!(v["match_count"], 1);
    assert_eq!(v["found"][0]["match_kind"], "exact");
    assert_eq!(v["found_cx_ids"][0], cx(1).to_string());

    // Unplanted name -> first-class NOT_IN_CORPUS, not an error.
    let mut absent = CiteChecker::new("State v. Unplanted");
    absent.observe(cx(1), &planted);
    absent.observe(cx(2), &other);
    let v = absent.to_json(&CiteCheckArgs {
        vault: "/v".into(),
        case_name: "State v. Unplanted".into(),
        proposition: None,
    });
    assert_eq!(v["status"], "NOT_IN_CORPUS");
    assert_eq!(v["match_count"], 0);
}

#[test]
fn cite_check_substring_and_case_insensitive() {
    let row = meta(&[("case_name", "State v.  KALISH  ")]);
    let mut c = CiteChecker::new("kalish");
    c.observe(cx(7), &row);
    let v = c.to_json(&CiteCheckArgs {
        vault: "/v".into(),
        case_name: "kalish".into(),
        proposition: None,
    });
    assert_eq!(v["status"], "found");
    assert_eq!(v["found"][0]["match_kind"], "contains");
}

#[test]
fn cite_check_proposition_is_delegated_not_evaluated() {
    let mut c = CiteChecker::new("State v. Planted");
    c.observe(cx(1), &meta(&[("case_name", "State v. Planted")]));
    let v = c.to_json(&CiteCheckArgs {
        vault: "/v".into(),
        case_name: "State v. Planted".into(),
        proposition: Some("holds that X requires Y".into()),
    });
    assert_eq!(v["proposition"]["status"], "not_evaluated");
    assert!(
        v["proposition"]["text"]
            .as_str()
            .unwrap()
            .contains("requires Y")
    );
}

#[test]
fn normalize_case_name_collapses_and_lowercases() {
    assert_eq!(
        normalize_case_name("  State   v.  Smith  "),
        "state v. smith"
    );
    assert_eq!(normalize_case_name("\"Quoted Name\""), "quoted name");
    assert_eq!(normalize_case_name(""), "");
}

// ---- citation-neighbors ----------------------------------------------------

fn edge(src: CxId, dst: CxId, edge_type: &str, weight: f64) -> PlainGraphEdge {
    PlainGraphEdge {
        src,
        dst,
        edge_type: edge_type.to_string(),
        value: serde_json::to_vec(&serde_json::json!({
            "edge_type": edge_type,
            "weight": weight,
            "depth": 1,
        }))
        .unwrap(),
    }
}

#[test]
fn citation_neighbors_returns_exact_planted_edges() {
    // Planted graph:  1 -> 2, 1 -> 3 (out of node 1); 4 -> 1 (into node 1).
    let edges = vec![
        edge(cx(1), cx(2), "cites", 0.9),
        edge(cx(1), cx(3), "cites", 0.5),
        edge(cx(4), cx(1), "cites", 0.7),
        edge(cx(5), cx(6), "cites", 0.1), // unrelated
    ];

    let (out, out_n) = collect_neighbors(&edges, cx(1), Direction::Out, 100);
    assert_eq!(out_n, 2);
    let out_ids: Vec<&str> = out.iter().map(|n| n["cx_id"].as_str().unwrap()).collect();
    assert_eq!(out_ids, vec![cx(2).to_string(), cx(3).to_string()]);
    assert!(out.iter().all(|n| n["direction"] == "out"));
    assert_eq!(out[0]["weight"], 0.9);
    assert_eq!(out[0]["edge_type"], "cites");
    assert_eq!(out[0]["edge_value"]["depth"], 1);

    let (inn, in_n) = collect_neighbors(&edges, cx(1), Direction::In, 100);
    assert_eq!(in_n, 1);
    assert_eq!(inn[0]["cx_id"], cx(4).to_string());
    assert_eq!(inn[0]["direction"], "in");

    let (both, both_n) = collect_neighbors(&edges, cx(1), Direction::Both, 100);
    assert_eq!(both_n, 3);
    assert_eq!(both.len(), 3);

    // Limit truncates deterministically while reporting the true match count.
    let (limited, matched) = collect_neighbors(&edges, cx(1), Direction::Both, 2);
    assert_eq!(matched, 3);
    assert_eq!(limited.len(), 2);
}

#[test]
fn citation_neighbors_handles_non_json_edge_value() {
    let edges = vec![PlainGraphEdge {
        src: cx(1),
        dst: cx(2),
        edge_type: "cites".into(),
        value: b"\x00\x01not-json".to_vec(),
    }];
    let (out, _) = collect_neighbors(&edges, cx(1), Direction::Out, 10);
    assert_eq!(out.len(), 1);
    assert!(out[0]["weight"].is_null());
    assert!(out[0]["edge_value"]["raw_hex"].is_string());
}

// ---- case-text window ------------------------------------------------------

#[test]
fn text_window_respects_offset_and_max_bytes() {
    let text = "abcdefghij";
    let (w, total, returned, truncated) = text_window(text, 0, 4);
    assert_eq!(w, "abcd");
    assert_eq!(total, 10);
    assert_eq!(returned, 4);
    assert!(truncated);

    let (w, _, _, truncated) = text_window(text, 8, 100);
    assert_eq!(w, "ij");
    assert!(!truncated);

    let (w, _, returned, _) = text_window(text, 100, 10);
    assert_eq!(w, "");
    assert_eq!(returned, 0);
}

#[test]
fn text_window_snaps_to_char_boundaries() {
    // Each "é" is 2 bytes in UTF-8; a raw 3-byte window must not split one.
    let text = "éé"; // 4 bytes
    let (w, total, _, _) = text_window(text, 0, 3);
    assert_eq!(total, 4);
    assert_eq!(w, "é"); // snapped down to the 2-byte boundary
    assert!(w.is_char_boundary(w.len()));
}

// ---- judge census ----------------------------------------------------------

#[test]
fn judge_census_counts_authored_by_person_id() {
    // Judge A = person_id 5352, Judge B = 5398. Plant: A authors 2 (one dissent),
    // B authors 1; A sits on one more panel.
    let mut c = JudgeCensus::new("5352");
    c.observe(&meta(&[("author_id", "5352"), ("opinion_type", "020lead")]));
    c.observe(&meta(&[
        ("author_id", "5352"),
        ("opinion_type", "040dissent"),
    ]));
    c.observe(&meta(&[("author_id", "5398"), ("opinion_type", "020lead")]));
    c.observe(&meta(&[("judges", "5352 5398")])); // panel only, no author_id
    let v = c.to_json(std::path::Path::new("/v"));
    assert_eq!(v["authored_count"], 2);
    assert_eq!(v["authored_dissents_n"], 1);
    assert_eq!(v["total_opinions_scanned"], 4);
    assert_eq!(v["opinions_with_author_id_n"], 3);
    assert_eq!(v["panel_member_count"], 1);
    // 3 of 4 rows carry author_id.
    assert!((v["live_author_resolution_coverage"].as_f64().unwrap() - 0.75).abs() < 1e-9);
}

#[test]
fn judge_census_matches_name_substring_case_insensitive() {
    let mut c = JudgeCensus::new("Boyle");
    c.observe(&meta(&[
        ("author_str", "Mary J. Boyle"),
        ("opinion_type", "020lead"),
    ]));
    c.observe(&meta(&[("author_str", "Some Other")]));
    let v = c.to_json(std::path::Path::new("/v"));
    assert_eq!(v["authored_count"], 1);
    assert!(v["resolved_as"].as_str().unwrap().contains("author_str"));
}

// ---- argv contracts --------------------------------------------------------

#[test]
fn citation_neighbors_argv_parses_and_defaults() {
    let a = parse_citation_neighbors_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        &cx(1).to_string(),
        "--direction",
        "both",
    ]))
    .unwrap();
    assert_eq!(a.direction, Direction::Both);
    assert_eq!(a.collection, "legal-citations-v2");
    assert_eq!(a.limit, 256);

    let a = parse_citation_neighbors_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        &cx(1).to_string(),
        "--direction",
        "out",
        "--collection",
        "legal-citations-v1",
        "--limit",
        "5",
    ]))
    .unwrap();
    assert_eq!(a.collection, "legal-citations-v1");
    assert_eq!(a.limit, 5);
    assert_eq!(a.direction, Direction::Out);
}

#[test]
fn citation_neighbors_argv_rejects_missing_and_bad_flags() {
    // Missing --direction.
    let err =
        parse_citation_neighbors_args(&tokens(&["--vault", "/v", "--cx-id", &cx(1).to_string()]))
            .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    // Bad direction.
    let err = parse_citation_neighbors_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        &cx(1).to_string(),
        "--direction",
        "sideways",
    ]))
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    // Bad cx-id.
    let err = parse_citation_neighbors_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        "not-hex",
        "--direction",
        "in",
    ]))
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    // Unknown flag.
    let err = parse_citation_neighbors_args(&tokens(&["--vault", "/v", "--boom"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn cite_check_argv_requires_case_name() {
    let a =
        parse_cite_check_args(&tokens(&["--vault", "/v", "--case-name", "State v. X"])).unwrap();
    assert_eq!(a.case_name, "State v. X");
    assert!(a.proposition.is_none());

    let a = parse_cite_check_args(&tokens(&[
        "--vault",
        "/v",
        "--case-name",
        "State v. X",
        "--proposition",
        "holds Y",
    ]))
    .unwrap();
    assert_eq!(a.proposition.as_deref(), Some("holds Y"));

    let err = parse_cite_check_args(&tokens(&["--vault", "/v"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    let err = parse_cite_check_args(&tokens(&["--case-name", "X"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn case_text_argv_defaults_and_validates() {
    let a =
        parse_case_text_args(&tokens(&["--vault", "/v", "--cx-id", &cx(1).to_string()])).unwrap();
    assert_eq!(a.max_bytes, 32 * 1024);
    assert_eq!(a.offset, 0);

    let a = parse_case_text_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        &cx(1).to_string(),
        "--max-bytes",
        "64",
        "--offset",
        "10",
    ]))
    .unwrap();
    assert_eq!(a.max_bytes, 64);
    assert_eq!(a.offset, 10);

    let err = parse_case_text_args(&tokens(&[
        "--vault",
        "/v",
        "--cx-id",
        &cx(1).to_string(),
        "--max-bytes",
        "0",
    ]))
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn judge_and_opinion_census_argv_contracts() {
    assert!(parse_opinion_type_census_args(&tokens(&["--vault", "/v"])).is_ok());
    let err = parse_opinion_type_census_args(&tokens(&[])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");

    let a = parse_judge_census_args(&tokens(&["--vault", "/v", "--judge", "5352"])).unwrap();
    assert_eq!(a.judge, "5352");
    let err = parse_judge_census_args(&tokens(&["--vault", "/v"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}
