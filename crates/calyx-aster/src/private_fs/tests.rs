use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> std::path::PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "calyx-private-fs-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn create_private_dir_all_is_0700_under_permissive_umask() {
    use nix::sys::stat::{Mode, umask};

    let root = scratch("dir-all");
    let previous = umask(Mode::empty()); // umask 000: worst case for leakage
    let nested = root.join("cf").join("base");
    create_private_dir_all(&nested).unwrap();
    umask(previous);

    assert_eq!(mode(&root), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&root.join("cf")), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&nested), PRIVATE_DIRECTORY_MODE);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn hardened_open_options_and_temp_are_0600() {
    use std::fs::OpenOptions;

    let root = scratch("file");
    create_private_dir_all(&root).unwrap();

    let fresh = root.join("fresh.bin");
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    harden_open_options(&mut options);
    let file = options.open(&fresh).unwrap();
    enforce_private_file_mode(&file, &fresh).unwrap();
    assert_eq!(mode(&fresh), PRIVATE_FILE_MODE);

    let tmp = root.join("staging.tmp");
    {
        use std::io::Write;
        let mut f = open_private_temp_file(&tmp).unwrap();
        f.write_all(b"first").unwrap();
    }
    assert_eq!(mode(&tmp), PRIVATE_FILE_MODE);
    // Truncate-existing path still enforces 0600 even if left wrong.
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o666)).unwrap();
    }
    let _ = open_private_temp_file(&tmp).unwrap();
    assert_eq!(mode(&tmp), PRIVATE_FILE_MODE);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn write_private_atomic_publishes_0600_with_readback() {
    let root = scratch("atomic");
    create_private_dir_all(&root).unwrap();
    let target = root.join("MANIFEST.json");
    let payload = b"{\"durable\":true}";
    write_private_atomic(&target, payload).unwrap();

    assert_eq!(mode(&target), PRIVATE_FILE_MODE);
    assert_eq!(fs::read(&target).unwrap(), payload);
    // No temp residue.
    let residue = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!residue, "atomic write left a temp residue");
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn harden_tree_fixes_world_accessible_descendants() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("harden");
    fs::create_dir_all(root.join("cf/base")).unwrap();
    fs::write(root.join("MANIFEST"), b"m").unwrap();
    fs::write(root.join("cf/base/000001.sst"), b"s").unwrap();
    // Simulate a vault written by an older binary under a permissive umask.
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    fs::set_permissions(root.join("cf"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(root.join("cf/base"), fs::Permissions::from_mode(0o775)).unwrap();
    fs::set_permissions(root.join("MANIFEST"), fs::Permissions::from_mode(0o666)).unwrap();
    fs::set_permissions(
        root.join("cf/base/000001.sst"),
        fs::Permissions::from_mode(0o664),
    )
    .unwrap();

    harden_tree(&root).unwrap();

    assert_eq!(mode(&root), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&root.join("cf")), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&root.join("cf/base")), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&root.join("MANIFEST")), PRIVATE_FILE_MODE);
    assert_eq!(mode(&root.join("cf/base/000001.sst")), PRIVATE_FILE_MODE);
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn harden_tree_hardens_through_symlinked_root() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    // A vault may live on one pool and be linked into the vault directory.
    // Hardening must follow the root link and harden the target tree, rather
    // than refusing the open (#1935 regression: read-only readback of a
    // symlinked production vault failed with CALYX_VAULT_PERMISSIONS_UNSAFE).
    let target = scratch("harden-root-target");
    fs::create_dir_all(target.join("cf/base")).unwrap();
    fs::write(target.join("MANIFEST"), b"m").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o777)).unwrap();
    fs::set_permissions(target.join("MANIFEST"), fs::Permissions::from_mode(0o666)).unwrap();

    let link = scratch("harden-root-link");
    let _ = fs::remove_file(&link);
    symlink(&target, &link).unwrap();

    harden_tree(&link).expect("a symlinked vault root must harden its target");

    assert_eq!(mode(&target), PRIVATE_DIRECTORY_MODE);
    assert_eq!(mode(&target.join("MANIFEST")), PRIVATE_FILE_MODE);

    fs::remove_file(&link).unwrap();
    fs::remove_dir_all(&target).unwrap();
}

#[cfg(unix)]
#[test]
fn harden_tree_rejects_symlink_descendant() {
    use std::os::unix::fs::symlink;

    let root = scratch("harden-symlink");
    fs::create_dir_all(&root).unwrap();
    let outside = scratch("harden-outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), b"x").unwrap();
    symlink(&outside, root.join("link")).unwrap();

    let error = harden_tree(&root).expect_err("symlink descendant must be rejected");
    assert_eq!(error.code, CALYX_VAULT_PERMISSIONS_UNSAFE);

    fs::remove_file(root.join("link")).unwrap();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[cfg(unix)]
#[test]
fn create_private_file_rejects_symlink_target() {
    use std::os::unix::fs::symlink;

    let root = scratch("nofollow");
    create_private_dir_all(&root).unwrap();
    let outside = scratch("nofollow-outside");
    create_private_dir_all(&outside).unwrap();
    let victim = outside.join("victim");
    let link = root.join("staging.tmp");
    symlink(&victim, &link).unwrap();

    let error = open_private_temp_file(&link).expect_err("O_NOFOLLOW must reject a symlink");
    assert_eq!(error.code, CALYX_VAULT_PERMISSIONS_UNSAFE);
    assert!(!victim.exists(), "write must not have followed the link");

    fs::remove_file(&link).unwrap();
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}
