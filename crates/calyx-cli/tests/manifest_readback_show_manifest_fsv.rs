//! Full State Verification for `readback --vault <dir> --show-manifest`
//! CLI dispatch (regression guard for issue #1262).
//!
//! The #1262 bug lived in the argument dispatch in
//! `crates/calyx-cli/src/dispatch/early.rs`: `readback --vault ... --show-manifest`
//! was hard-routed to the Leapable *shadow* manifest reader, so a valid native
//! Aster vault reported the misleading `CALYX_MANIFEST_CORRUPT: shadow manifest
//! magic mismatch`. The fix (`manifest_readback::readback_vault_manifest`)
//! auto-detects the on-disk format and dispatches accordingly.
//!
//! The unit tests in `manifest_readback.rs` call `readback_vault_manifest()`
//! directly — they do NOT exercise `early.rs`, which is where the bug actually
//! was. This test drives the REAL compiled `calyx` binary as a subprocess so the
//! whole CLI dispatch path is under test; a regression that re-hard-routes
//! `--show-manifest` in `early.rs` would fail here even though the unit tests
//! stayed green.
//!
//! Source of truth: the vault directory bytes on disk (the `CURRENT` pointer and
//! the leading byte of `MANIFEST`), read back independently of the command's own
//! JSON, plus the process exit status and structured error code.

use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use serde_json::Value;

fn calyx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
}

fn show_manifest(vault: &Path) -> Output {
    calyx()
        .args([
            "readback",
            "--vault",
            vault.to_str().unwrap(),
            "--show-manifest",
        ])
        .output()
        .expect("spawn calyx")
}

/// Materializes a real, loadable native Aster vault (no mocks): valid immutable
/// panel/codebook assets plus a `CURRENT`-pointed JSON manifest mirror. Mirrors
/// the shipped `manifest_readback` unit fixture so the two stay in lockstep.
fn write_native_vault(dir: &Path) {
    std::fs::create_dir_all(dir.join("panel")).expect("panel dir");
    std::fs::create_dir_all(dir.join("codebooks")).expect("codebook dir");
    std::fs::write(dir.join("panel/panel-0001.json"), b"panel").expect("panel bytes");
    std::fs::write(dir.join("codebooks/slot_00.cb"), b"codebook").expect("codebook bytes");
    let manifest = VaultManifest::new(
        1,
        10,
        ImmutableRef::from_bytes("panel/panel-0001.json", b"panel").unwrap(),
        vec![ImmutableRef::from_bytes("codebooks/slot_00.cb", b"codebook").unwrap()],
    )
    .expect("build native manifest");
    ManifestStore::open(dir)
        .write_current(&manifest)
        .expect("write native vault manifest");
}

#[test]
fn show_manifest_cli_dispatch_fsv() {
    let root = std::env::temp_dir().join(format!("calyx-1262-cli-fsv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("root dir");

    // ================= HAPPY PATH: native vault routes to native =============
    // This is the decisive #1262 regression guard: the real CLI dispatch must
    // send a native vault to the native reader, not the shadow reader.
    let native = root.join("native");
    write_native_vault(&native);

    // ---- Source of truth on disk, read independently of the command. --------
    assert!(
        native.join("CURRENT").is_file(),
        "[SoT] native vault must have a CURRENT pointer"
    );
    let manifest_lead = std::fs::read(native.join("MANIFEST")).expect("read MANIFEST")[0];
    println!("[SoT] native MANIFEST leading byte = {manifest_lead:?} (expect b'{{' = 123)");
    assert_eq!(
        manifest_lead, b'{',
        "[SoT] native MANIFEST must be a JSON mirror beginning with '{{', not shadow magic"
    );

    let out = show_manifest(&native);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!(
        "[native] exit={:?} stdout={stdout} stderr={stderr}",
        out.status.code()
    );
    assert!(
        out.status.success(),
        "native --show-manifest must succeed (the #1262 bug made it exit non-zero): stderr={stderr}"
    );
    let v: Value = serde_json::from_str(&stdout).expect("native stdout must be JSON");
    assert_eq!(
        v["vault_format"].as_str(),
        Some("native-aster"),
        "native vault must report vault_format=native-aster"
    );
    // Manifest content actually landed — prove the native reader read the vault,
    // not an empty/placeholder shape.
    assert_eq!(
        v["manifest"]["manifest_seq"].as_u64(),
        Some(1),
        "native manifest_seq must round-trip from the on-disk vault"
    );
    assert_eq!(
        v["manifest"]["durable_seq"].as_u64(),
        Some(10),
        "native durable_seq must round-trip from the on-disk vault"
    );
    // The exact failure signature of the original bug must be absent.
    assert!(
        !stdout.contains("CALYX_MANIFEST_CORRUPT") && !stderr.contains("CALYX_MANIFEST_CORRUPT"),
        "native vault must NOT surface the shadow CALYX_MANIFEST_CORRUPT error (#1262 regression)"
    );

    // ================= EDGE: shadow-magic vault routes to shadow ==============
    // A directory whose MANIFEST begins with the shadow magic `CXSHDW1!` must be
    // dispatched to the shadow reader — proven by the ABSENCE of the native and
    // unrecognized signatures (a valid shadow vault needs the pub(crate) shadow
    // harness, unavailable to an integration test, so we assert the routing, not
    // a successful shadow read).
    let shadow = root.join("shadow");
    std::fs::create_dir_all(&shadow).expect("shadow dir");
    std::fs::write(shadow.join("MANIFEST"), b"CXSHDW1!\x00garbage-body").expect("shadow manifest");
    let out = show_manifest(&shadow);
    let sh_stdout = String::from_utf8_lossy(&out.stdout);
    let sh_stderr = String::from_utf8_lossy(&out.stderr);
    println!(
        "[shadow] exit={:?} stdout={sh_stdout} stderr={sh_stderr}",
        out.status.code()
    );
    assert!(
        !sh_stdout.contains("\"vault_format\":\"native-aster\"")
            && !sh_stdout.contains("native-aster"),
        "shadow-magic vault must NOT be misrouted to the native reader"
    );
    assert!(
        !sh_stdout.contains("CALYX_VAULT_FORMAT_UNRECOGNIZED")
            && !sh_stderr.contains("CALYX_VAULT_FORMAT_UNRECOGNIZED"),
        "shadow-magic vault must be recognized as shadow, not reported as unrecognized"
    );

    // ================= EDGE: unrecognized dir fails closed distinctly =========
    // An empty dir is neither format: it must fail with the distinct
    // CALYX_VAULT_FORMAT_UNRECOGNIZED code, never the misleading shadow
    // CALYX_MANIFEST_CORRUPT that started #1262.
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).expect("empty dir");
    let out = show_manifest(&empty);
    let em_stdout = String::from_utf8_lossy(&out.stdout);
    let em_stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{em_stdout}{em_stderr}");
    println!("[empty] exit={:?} out={combined}", out.status.code());
    assert!(
        !out.status.success(),
        "empty dir --show-manifest must fail closed"
    );
    assert!(
        combined.contains("CALYX_VAULT_FORMAT_UNRECOGNIZED"),
        "empty dir must fail with CALYX_VAULT_FORMAT_UNRECOGNIZED, got: {combined}"
    );
    assert!(
        !combined.contains("CALYX_MANIFEST_CORRUPT"),
        "empty dir must NOT surface the misleading shadow CALYX_MANIFEST_CORRUPT (#1262)"
    );

    let _ = std::fs::remove_dir_all(&root);
}
