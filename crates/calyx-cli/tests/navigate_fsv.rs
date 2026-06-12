//! End-to-end FSV for `calyx navigate` (issue #599).
//!
//! Drives the real `calyx` binary against a deterministic engine spec that is
//! byte-for-byte the calyx-sextant ph63 fixture (issue #600), then asserts the
//! CLI readback against the *hand-computed* navigation values proven there.
//! Every happy-path mode writes its readback to a Source-of-Truth file with
//! `--out`, and the assertions read those files back from disk — never the
//! process's claimed exit value alone. Edge cases prove each `CALYX_SEXTANT_*`
//! code fails closed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// `cx(n)` — the 32-hex-char id `CxId::from_bytes((n as u128).to_be_bytes())`.
fn cx(n: u8) -> String {
    format!("{:032x}", n as u128)
}

/// The two-lens consensus + skills + association-graph spec from ph63.
fn fixture_spec() -> Value {
    let lens_a = json!([
        {"cx": cx(1), "vector": [1.0, 0.0], "seq": 1},
        {"cx": cx(2), "vector": [0.96, 0.28], "seq": 2},
        {"cx": cx(3), "vector": [0.96, -0.28], "seq": 3},
        {"cx": cx(4), "vector": [0.0, 1.0], "seq": 4},
        {"cx": cx(5), "vector": [0.28, 0.96], "seq": 5},
        {"cx": cx(6), "vector": [-0.28, 0.96], "seq": 6},
        {"cx": cx(7), "vector": [1.0, 0.0], "seq": 7},
    ]);
    // Lens B agrees with lens A on cx1..6 but flips cx7 to orthogonal.
    let mut lens_b = lens_a.as_array().unwrap().clone();
    *lens_b.last_mut().unwrap() = json!({"cx": cx(7), "vector": [0.0, 1.0], "seq": 7});
    json!({
        "indexes": [
            {"slot": 1, "dim": 2, "seed": 7, "entries": lens_a},
            {"slot": 2, "dim": 2, "seed": 7, "entries": lens_b},
        ],
        "nodes": [
            {"cx": cx(1), "weight": 1.0},
            {"cx": cx(2), "weight": 1.0},
            {"cx": cx(3), "weight": 1.0},
        ],
        "edges": [
            {"src": cx(1), "dst": cx(2), "weight": 0.8},
            {"src": cx(2), "dst": cx(3), "weight": 0.5},
            {"src": cx(3), "dst": cx(1), "weight": 0.25},
        ],
    })
}

/// A single-lens spec: cross-lens consensus is undefined here.
fn single_lens_spec() -> Value {
    json!({
        "indexes": [
            {"slot": 1, "dim": 2, "seed": 7, "entries": [
                {"cx": cx(1), "vector": [1.0, 0.0], "seq": 1},
                {"cx": cx(2), "vector": [0.0, 1.0], "seq": 2},
            ]},
        ],
    })
}

#[test]
fn neighbors_ranks_self_first() {
    let dir = reset_temp_root("calyx-nav-neighbors");
    let spec = write_spec(&dir, &fixture_spec());
    let out = dir.join("neighbors.json");
    let output = run(&[
        "navigate",
        "neighbors",
        "--spec",
        spec_str(&spec),
        "--cx",
        &cx(1),
        "--slot",
        "1",
        "--k",
        "3",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let readback = read_json(&out);
    let hits = readback["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 3, "expected k=3 hits: {readback}");
    // cx1's own vector is the query; cosine 1.0 dominates.
    assert!((hits[0]["score"].as_f64().unwrap() - 1.0).abs() < 1e-3);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn define_returns_anchor_constellation_with_slots() {
    let dir = reset_temp_root("calyx-nav-define");
    let spec = write_spec(&dir, &fixture_spec());
    let out = dir.join("define.json");
    let output = run(&[
        "navigate",
        "define",
        "--spec",
        spec_str(&spec),
        "--cx",
        &cx(1),
        "--slot",
        "1",
        "--k",
        "3",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let readback = read_json(&out);
    assert_eq!(readback["constellation"]["cx_id"], json!(cx(1)));
    assert!(
        !readback["constellation"]["slots"]
            .as_object()
            .unwrap()
            .is_empty(),
        "define should gather slot centroids: {readback}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn agree_and_disagree_match_ph63_handcomputed_values() {
    let dir = reset_temp_root("calyx-nav-consensus");
    let spec = write_spec(&dir, &fixture_spec());

    let agree_out = dir.join("agree.json");
    let output = run(&[
        "navigate",
        "agree",
        "--spec",
        spec_str(&spec),
        "--anchor",
        &cx(1),
        "--k",
        "10",
        "--out",
        agree_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let agree = read_json(&agree_out);
    let agree_hits = agree["report"]["hits"].as_array().unwrap();
    // All lenses concur on cx2 (cos 0.96 on both); it wins the min-cosine rank.
    assert_eq!(agree_hits[0]["cx_id"], json!(cx(2)));
    assert!((agree_hits[0]["score"].as_f64().unwrap() - 0.96).abs() < 1e-3);

    let disagree_out = dir.join("disagree.json");
    let output = run(&[
        "navigate",
        "disagree",
        "--spec",
        spec_str(&spec),
        "--anchor",
        &cx(1),
        "--k",
        "10",
        "--out",
        disagree_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let disagree = read_json(&disagree_out);
    let disagree_hits = disagree["report"]["hits"].as_array().unwrap();
    // cx7 is aligned on lens A, orthogonal on lens B: maximum cross-lens spread.
    assert_eq!(disagree_hits[0]["cx_id"], json!(cx(7)));
    assert!((disagree_hits[0]["score"].as_f64().unwrap() - 1.0).abs() < 1e-3);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn traverse_is_directional_and_attenuated() {
    let dir = reset_temp_root("calyx-nav-traverse");
    let spec = write_spec(&dir, &fixture_spec());

    let fwd_out = dir.join("forward.json");
    let output = run(&[
        "navigate",
        "traverse",
        "--spec",
        spec_str(&spec),
        "--anchor",
        &cx(1),
        "--direction",
        "forward",
        "--hops",
        "2",
        "--out",
        fwd_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let forward = read_json(&fwd_out);
    let fwd = forward["path"]["steps"].as_array().unwrap();
    assert_eq!(fwd.len(), 2);
    assert!((fwd[0]["score"].as_f64().unwrap() - 0.72).abs() < 1e-3);
    assert!((fwd[1]["score"].as_f64().unwrap() - 0.324).abs() < 1e-3);

    let bwd_out = dir.join("backward.json");
    let output = run(&[
        "navigate",
        "traverse",
        "--spec",
        spec_str(&spec),
        "--anchor",
        &cx(1),
        "--direction",
        "backward",
        "--hops",
        "2",
        "--out",
        bwd_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let backward = read_json(&bwd_out);
    let bwd = backward["path"]["steps"].as_array().unwrap();
    assert!((bwd[0]["score"].as_f64().unwrap() - 0.225).abs() < 1e-3);
    assert_ne!(
        fwd[0]["cx_id"], bwd[0]["cx_id"],
        "forward/backward must differ"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn skills_then_search_skill_stays_in_scope() {
    let dir = reset_temp_root("calyx-nav-skills");
    let spec = write_spec(&dir, &fixture_spec());

    let skills_out = dir.join("skills.json");
    let output = run(&[
        "navigate",
        "skills",
        "--spec",
        spec_str(&spec),
        "--min-cluster-size",
        "2",
        "--min-samples",
        "1",
        "--out",
        skills_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let tree = read_json(&skills_out);
    let selected = tree["tree"]["selected"].as_array().unwrap();
    assert_eq!(selected.len(), 2, "two planted clusters: {tree}");

    // Find the selected skill that contains cx4 (the [0,1]-aligned cluster).
    let nodes = tree["tree"]["nodes"].as_object().unwrap();
    let scope = selected
        .iter()
        .map(|name| name.as_str().unwrap())
        .find(|name| {
            nodes[*name]["members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|member| member == &json!(cx(4)))
        })
        .expect("a selected skill must contain cx4");

    let search_out = dir.join("search_skill.json");
    let output = run(&[
        "navigate",
        "search-skill",
        "--spec",
        spec_str(&spec),
        "--min-cluster-size",
        "2",
        "--min-samples",
        "1",
        "--skill",
        scope,
        "--slot",
        "1",
        "--k",
        "2",
        "--vec",
        "1.0,0.0",
        "--out",
        search_out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let hits = read_json(&search_out);
    let hits = hits["hits"].as_array().unwrap();
    // Within the cx4-cluster, cosine to [1,0]: cx5 (0.28) > cx4 (0.0).
    assert_eq!(hits[0]["cx_id"], json!(cx(5)));
    assert_eq!(hits[1]["cx_id"], json!(cx(4)));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fail_closed_codes() {
    let dir = reset_temp_root("calyx-nav-edges");
    let spec = write_spec(&dir, &fixture_spec());
    let single = write_named_spec(&dir, "single.json", &single_lens_spec());
    let s = spec_str(&spec);

    // Unknown anchor: no dense vector anywhere.
    expect_fail(
        &[
            "navigate",
            "agree",
            "--spec",
            s,
            "--anchor",
            &cx(99),
            "--k",
            "10",
        ],
        "CALYX_SEXTANT_CX_MISSING",
    );
    // Hops below and above the 1..=10 contract.
    expect_fail(
        &[
            "navigate",
            "traverse",
            "--spec",
            s,
            "--anchor",
            &cx(1),
            "--direction",
            "forward",
            "--hops",
            "0",
        ],
        "CALYX_SEXTANT_TRAVERSE_HOPS",
    );
    expect_fail(
        &[
            "navigate",
            "traverse",
            "--spec",
            s,
            "--anchor",
            &cx(1),
            "--direction",
            "forward",
            "--hops",
            "11",
        ],
        "CALYX_SEXTANT_TRAVERSE_HOPS",
    );
    // No association graph on a single-lens spec with no edges.
    expect_fail(
        &[
            "navigate",
            "traverse",
            "--spec",
            spec_str(&single),
            "--anchor",
            &cx(1),
            "--direction",
            "forward",
            "--hops",
            "2",
        ],
        "CALYX_SEXTANT_ASSOC_GRAPH_MISSING",
    );
    // Budget below the doc count.
    expect_fail(
        &[
            "navigate",
            "skills",
            "--spec",
            s,
            "--max-constellations",
            "2",
        ],
        "CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED",
    );
    // Unknown skill scope.
    expect_fail(
        &[
            "navigate",
            "search-skill",
            "--spec",
            s,
            "--skill",
            "skill-nope",
            "--slot",
            "1",
            "--k",
            "2",
            "--vec",
            "1.0,0.0",
        ],
        "CALYX_SEXTANT_SKILL_UNKNOWN",
    );
    // Cross-lens consensus needs two lenses.
    expect_fail(
        &[
            "navigate",
            "agree",
            "--spec",
            spec_str(&single),
            "--anchor",
            &cx(1),
            "--k",
            "10",
        ],
        "CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES",
    );
    let _ = fs::remove_dir_all(dir);
}

// ---- helpers ----------------------------------------------------------------

fn run(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    for arg in args {
        command.arg(arg);
    }
    command.output().expect("run calyx navigate")
}

fn expect_fail(args: &[&str], code: &str) {
    let output = run(args);
    assert!(
        !output.status.success(),
        "expected failure for {args:?}, got success"
    );
    assert!(
        stderr(&output).contains(code),
        "expected `{code}` in stderr for {args:?}, got: {}",
        stderr(&output)
    );
}

fn write_spec(dir: &Path, spec: &Value) -> PathBuf {
    write_named_spec(dir, "spec.json", spec)
}

fn write_named_spec(dir: &Path, name: &str, spec: &Value) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(spec).unwrap()).expect("write spec");
    path
}

fn spec_str(path: &Path) -> &str {
    path.to_str().expect("utf8 spec path")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read readback")).expect("readback json")
}

fn reset_temp_root(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");
    root
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
