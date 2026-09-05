#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

use super::*;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> std::path::PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-private-fs-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

fn mode(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn write_private_json_is_0600_under_0700_parent() {
    let root = scratch("json");
    let target = root.join("runs").join("status.json");
    let value = json!({"ok": true, "ready": false, "stopping": true});
    write_private_json(&target, &value).unwrap();

    assert_eq!(mode(&target), 0o600);
    assert_eq!(mode(&root.join("runs")), 0o700);
    assert_eq!(mode(&root), 0o700);
    let read: serde_json::Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    assert_eq!(read, value);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn overwrite_stays_0600_and_leaves_no_temp() {
    let root = scratch("overwrite");
    let target = root.join("status.json");
    write_private_json(&target, &json!({"n": 1})).unwrap();
    write_private_json(&target, &json!({"n": 2, "more": "bytes"})).unwrap();

    assert_eq!(mode(&target), 0o600);
    let read: serde_json::Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    assert_eq!(read, json!({"n": 2, "more": "bytes"}));
    let residue = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!residue, "atomic overwrite left a temp residue");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn write_refuses_symlinked_parent_leaf_target() {
    use std::os::unix::fs::symlink;

    let root = scratch("symlink");
    create_private_dir_all(&root).unwrap();
    let outside = scratch("symlink-outside");
    create_private_dir_all(&outside).unwrap();
    let victim = outside.join("victim.json");
    // Pre-plant a symlink where the temp sibling would be created.
    let target = root.join("status.json");
    let tmp = root.join(format!(".status.json.{}.tmp", std::process::id()));
    symlink(&victim, &tmp).unwrap();

    let error = write_private_bytes(&target, b"payload")
        .expect_err("O_NOFOLLOW must reject a symlinked temp path");
    assert_eq!(error.code(), CALYX_EVIDENCE_PERMISSIONS_UNSAFE);
    assert!(!victim.exists(), "write must not have followed the link");

    let _ = fs::remove_file(&tmp);
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}
