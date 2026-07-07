//! Repo-hygiene guard: Poly is local-only (issue #159, context #2; purge #220). No Poly-owned
//! source may point the engine at the private build host or any remote dataset host. This test
//! fails loud if a forbidden remote-host token reappears anywhere under `calyx-poly/src` **or**
//! `calyx-poly/tests`, so the stale Calyx-import assumption that sent an agent to a remote box
//! (corrected #219) cannot silently creep back into Poly runtime code or its
//! full-state-verification tests.
//!
//! Scope note: this guard deliberately covers only the Poly-owned crate (`calyx-poly`). The
//! vendored upstream Calyx crates and design docs legitimately carry Calyx's own Linux/`/zfs/`
//! deployment references, and the relocated `docs2/ops-reference/` tree keeps those tokens on
//! purpose as Windows-rewrite reference (#128). Neither is Poly-owned runtime source, so neither is
//! scanned here.

use std::fs;
use std::path::{Path, PathBuf};

/// Tokens that mean "remote box" and must never appear in Poly-owned source.
const FORBIDDEN: &[&str] = &[concat!("ai", "wonder"), "CALYX_DATASET_ROOT", "/zfs/"];

/// This guard file itself necessarily names the forbidden tokens (above); skip it by basename so it
/// never flags itself.
const SELF_FILE: &str = "local_only_guard.rs";

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return; // a missing tests/ or src/ dir is not a violation
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && path.file_name().and_then(|n| n.to_str()) != Some(SELF_FILE)
        {
            out.push(path);
        }
    }
}

#[test]
fn poly_owned_source_never_targets_a_remote_host() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_sources(&crate_root.join("src"), &mut files);
    rust_sources(&crate_root.join("tests"), &mut files);
    assert!(
        !files.is_empty(),
        "expected calyx-poly src/tests to contain sources"
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read source");
        for (lineno, line) in text.lines().enumerate() {
            for token in FORBIDDEN {
                if line.contains(token) {
                    violations.push(format!(
                        "{}:{} references forbidden remote-host token `{token}`: {}",
                        file.display(),
                        lineno + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Poly is local-only (#159/#2, #220); remove these remote-host references from Poly-owned \
         source (or, if this is deliberate inert reference material, move it under \
         docs2/ops-reference/):\n{}",
        violations.join("\n")
    );
}
