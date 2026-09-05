//! FSV for the issue #1962 vault-scoped legal readback verbs, driven end to end
//! against a REAL durable vault with crafted, known-outcome data.
//!
//! RUNTIME NOTE: this file COMPILES on every platform, but building/opening a real
//! Calyx vault requires the owner-only (0700/0600) private filesystem enforced only
//! on Unix — on a non-Unix host the vault open fails closed with
//! `CALYX_VAULT_PERMISSIONS_UNSAFE`. The test therefore SKIPS at runtime on non-Unix
//! (`cfg!(unix)` guard) and executes its assertions only on the Linux FSV host,
//! which the orchestrator runs before merge.
//!
//! Planted corpus (5 opinions):
//!   cx1: 040dissent,            author 5352, "State v. Planted" (real retained text)
//!   cx2: 040dissent,            author 5398, "State v. Beta"
//!   cx3: 030concurrence,        author 5352, "State v. Gamma"
//!   cx4: 035concurrenceinpart,  author 5398, "State v. Delta"
//!   cx5: 010combined,           author 5352, "State v. Epsilon"
//!   citation edges: cx1->cx2, cx1->cx3, cx4->cx1  (collection legal-citations-v2)
//! Known outcomes asserted:
//!   opinion-type-census: physical=5, dissents=2, concurrence_family=2
//!   cite-check "State v. Planted" -> found{cx1}; "State v. Ghost" -> NOT_IN_CORPUS
//!   citation-neighbors cx1 out -> {cx2,cx3}; in -> {cx4}
//!   judge-census 5352 -> authored 3; case-text cx1 -> the planted holding text

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::retained_input::{canonical_text_pointer, retain_text_input};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::Value;

const PLANTED_TEXT: &str = "PLANTED OPINION TEXT: the holding is that X requires Y under Ohio law.";
const COLLECTION: &str = "legal-citations-v2";

fn calyx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
}

fn run(args: &[&str]) -> (bool, Value, String) {
    let out = calyx().args(args).output().expect("spawn calyx");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let json = serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null);
    (out.status.success(), json, stderr)
}

fn vault_id() -> VaultId {
    VaultId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").expect("valid ulid")
}

fn meta(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn constellation(
    cx_id: CxId,
    seed: u16,
    input_ref: InputRef,
    metadata: BTreeMap<String, String>,
) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_700_000 + u64::from(seed),
        input_ref,
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed as u8; 32],
        },
        flags: CxFlags::default(),
    }
}

fn synthetic_input_ref(seed: u16) -> InputRef {
    let mut hash = [0u8; 32];
    hash[0] = seed as u8;
    InputRef {
        hash,
        pointer: Some(format!("synthetic://issue1962/{seed}")),
        redacted: false,
    }
}

fn path(dir: &Path) -> &str {
    dir.to_str().expect("utf-8 path")
}

fn neighbor_ids((ok, v, err): &(bool, Value, String)) -> Vec<String> {
    assert!(ok, "citation-neighbors failed: {err}");
    let mut ids: Vec<String> = v["neighbors"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| n["cx_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn build_vault(dir: &Path) -> (CxId, CxId, CxId, CxId, CxId) {
    let vault = AsterVault::new_durable(
        dir,
        vault_id(),
        b"issue1962-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    // cx1 gets a real retained input blob so case-text can decode it.
    retain_text_input(dir, PLANTED_TEXT).expect("retain planted text");
    let hash: [u8; 32] = blake3::hash(PLANTED_TEXT.as_bytes()).into();
    let cx1_ref = InputRef {
        hash,
        pointer: Some(canonical_text_pointer(&hash)),
        redacted: false,
    };

    let cx1 = vault.cx_id_for_input(PLANTED_TEXT.as_bytes(), 7);
    let cx2 = vault.cx_id_for_input(b"issue1962-cx2", 7);
    let cx3 = vault.cx_id_for_input(b"issue1962-cx3", 7);
    let cx4 = vault.cx_id_for_input(b"issue1962-cx4", 7);
    let cx5 = vault.cx_id_for_input(b"issue1962-cx5", 7);

    vault
        .put(constellation(
            cx1,
            1,
            cx1_ref,
            meta(&[
                ("opinion_type", "040dissent"),
                ("author_id", "5352"),
                ("case_name", "State v. Planted"),
                ("case_name_full", "State of Ohio v. Planted"),
            ]),
        ))
        .expect("put cx1");
    vault
        .put(constellation(
            cx2,
            2,
            synthetic_input_ref(2),
            meta(&[
                ("opinion_type", "040dissent"),
                ("author_id", "5398"),
                ("case_name", "State v. Beta"),
            ]),
        ))
        .expect("put cx2");
    vault
        .put(constellation(
            cx3,
            3,
            synthetic_input_ref(3),
            meta(&[
                ("opinion_type", "030concurrence"),
                ("author_id", "5352"),
                ("case_name", "State v. Gamma"),
            ]),
        ))
        .expect("put cx3");
    vault
        .put(constellation(
            cx4,
            4,
            synthetic_input_ref(4),
            meta(&[
                ("opinion_type", "035concurrenceinpart"),
                ("author_id", "5398"),
                ("case_name", "State v. Delta"),
            ]),
        ))
        .expect("put cx4");
    vault
        .put(constellation(
            cx5,
            5,
            synthetic_input_ref(5),
            meta(&[
                ("opinion_type", "010combined"),
                ("author_id", "5352"),
                ("case_name", "State v. Epsilon"),
            ]),
        ))
        .expect("put cx5");

    // Plant citation edges in the collection the command reads.
    let graph = PlainGraph::new(&vault, COLLECTION).expect("plain graph");
    for cx in [cx1, cx2, cx3, cx4] {
        graph.put_node(cx, b"{}").expect("put node");
    }
    let ev = |w: &str| -> Vec<u8> {
        format!("{{\"edge_type\":\"cites\",\"weight\":{w},\"depth\":1}}").into_bytes()
    };
    graph
        .put_edge(cx1, "cites", cx2, &ev("0.9"))
        .expect("edge 1->2");
    graph
        .put_edge(cx1, "cites", cx3, &ev("0.5"))
        .expect("edge 1->3");
    graph
        .put_edge(cx4, "cites", cx1, &ev("0.7"))
        .expect("edge 4->1");

    vault.flush().expect("flush vault");
    drop(graph);
    drop(vault);
    (cx1, cx2, cx3, cx4, cx5)
}

#[test]
fn issue1962_law_readback_fsv() {
    if !cfg!(unix) {
        eprintln!(
            "SKIP issue1962 law-readback FSV: a real Calyx vault needs the owner-only (Unix) \
             private filesystem; run this on the Linux FSV host."
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "calyx-issue1962-fsv-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let (cx1, cx2, cx3, cx4, cx5) = build_vault(&dir);

    // ---- opinion-type census -----------------------------------------------
    let (ok, v, err) = run(&["readback", "opinion-type-census", "--vault", path(&dir)]);
    assert!(ok, "census failed: {err}");
    assert_eq!(v["physical_opinions_n"], 5, "census: {v}");
    assert_eq!(v["dissents_n"], 2, "census: {v}");
    assert_eq!(v["concurrence_family_n"], 2, "census: {v}");
    assert_eq!(v["by_type"]["010combined"], 1);

    // ---- cite-check: planted found, unplanted NOT_IN_CORPUS ----------------
    let (ok, v, err) = run(&[
        "cite-check",
        "--vault",
        path(&dir),
        "--case-name",
        "State v. Planted",
    ]);
    assert!(ok, "cite-check found failed: {err}");
    assert_eq!(v["status"], "found");
    let found: Vec<&str> = v["found_cx_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(found, vec![cx1.to_string()]);

    let (ok, v, err) = run(&[
        "cite-check",
        "--vault",
        path(&dir),
        "--case-name",
        "State v. Ghost",
    ]);
    assert!(ok, "cite-check absent failed: {err}");
    assert_eq!(v["status"], "NOT_IN_CORPUS");
    assert_eq!(v["match_count"], 0);

    // ---- citation-neighbors: exact planted edges ---------------------------
    let out_ids = neighbor_ids(&run(&[
        "readback",
        "citation-neighbors",
        "--vault",
        path(&dir),
        "--cx-id",
        &cx1.to_string(),
        "--direction",
        "out",
        "--collection",
        COLLECTION,
    ]));
    assert_eq!(out_ids, sorted(vec![cx2.to_string(), cx3.to_string()]));

    let in_ids = neighbor_ids(&run(&[
        "readback",
        "citation-neighbors",
        "--vault",
        path(&dir),
        "--cx-id",
        &cx1.to_string(),
        "--direction",
        "in",
        "--collection",
        COLLECTION,
    ]));
    assert_eq!(in_ids, vec![cx4.to_string()]);

    let both_ids = neighbor_ids(&run(&[
        "readback",
        "citation-neighbors",
        "--vault",
        path(&dir),
        "--cx-id",
        &cx1.to_string(),
        "--direction",
        "both",
        "--collection",
        COLLECTION,
    ]));
    assert_eq!(both_ids.len(), 3);

    // ---- judge census ------------------------------------------------------
    let (ok, v, err) = run(&[
        "readback",
        "judge-census",
        "--vault",
        path(&dir),
        "--judge",
        "5352",
    ]);
    assert!(ok, "judge-census failed: {err}");
    assert_eq!(v["authored_count"], 3, "judge-census: {v}");
    assert_eq!(v["authored_dissents_n"], 1); // cx1 is 5352's dissent
    assert_eq!(v["total_opinions_scanned"], 5);

    // ---- case-text: decoded retained holding -------------------------------
    let (ok, v, err) = run(&[
        "readback",
        "case-text",
        "--vault",
        path(&dir),
        "--cx-id",
        &cx1.to_string(),
    ]);
    assert!(ok, "case-text failed: {err}");
    assert_eq!(v["case_name"], "State v. Planted");
    assert!(
        v["text"]
            .as_str()
            .unwrap()
            .contains("holding is that X requires Y"),
        "case-text text: {v}"
    );
    assert_eq!(v["truncated"], false);

    // ---- case-text on a non-retained cx fails closed (typed envelope) ------
    let (ok, _v, err) = run(&[
        "readback",
        "case-text",
        "--vault",
        path(&dir),
        "--cx-id",
        &cx5.to_string(),
        "--max-bytes",
        "8",
    ]);
    assert!(
        !ok,
        "expected case-text on a non-retained cx to fail closed"
    );
    assert!(err.contains("CALYX_"), "structured error envelope: {err}");

    let _ = std::fs::remove_dir_all(&dir);
}
