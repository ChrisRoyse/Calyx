#![cfg(target_os = "linux")]

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use calyx_gatebrokerd::fs_tx::{FsRoot, FsRootSpec, FsTxError};
use calyx_gatebrokerd::protocol::{LeafName, ObjectId, RootAlias};
use tempfile::TempDir;

fn real_root() -> (TempDir, FsRoot, std::path::PathBuf, std::path::PathBuf) {
    let temp = TempDir::new().expect("real temporary filesystem");
    let shared = temp.path().join("shared");
    let private = temp.path().join("private");
    std::fs::create_dir(&shared).unwrap();
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o711)).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let root = FsRoot::open(FsRootSpec {
        alias: RootAlias::new("test").unwrap(),
        common_ancestor: temp.path().to_path_buf(),
        shared_path: shared.clone(),
        private_path: private.clone(),
        broker_uid: uid,
        broker_gid: gid,
        published_uid: uid,
        published_gid: gid,
        shared_mode: 0o711,
        private_mode: 0o700,
        published_mode: 0o700,
    })
    .expect("host must provide required opaque handles and descriptor APIs");
    (temp, root, shared, private)
}

#[test]
fn prepare_publish_reopen_quarantine_and_delete_change_real_filesystem_state() {
    let (_temp, root, shared, private) = real_root();
    let object_id = ObjectId::new("11111111111111111111111111111111").unwrap();
    let prepared = root.prepare(object_id.clone()).unwrap();
    assert!(private.join("p-11111111111111111111111111111111").is_dir());
    assert!(!shared.join("happy").exists());

    let reopened = root
        .reopen_prepared(object_id.clone(), &prepared.identity)
        .unwrap();
    let published = root
        .publish(&reopened, LeafName::new("happy").unwrap())
        .unwrap();
    let metadata = std::fs::metadata(shared.join("happy")).unwrap();
    assert_eq!(metadata.ino(), published.identity.inode);
    assert!(!private.join("p-11111111111111111111111111111111").exists());
    std::fs::write(shared.join("happy/proof.txt"), b"physical-state-proof").unwrap();

    let reopened = root
        .reopen_published(
            object_id.clone(),
            LeafName::new("happy").unwrap(),
            &published.identity,
        )
        .unwrap();
    let quarantined = root.quarantine(&reopened).unwrap();
    assert!(!shared.join("happy").exists());
    assert_eq!(
        std::fs::read(private.join("q-11111111111111111111111111111111/proof.txt")).unwrap(),
        b"physical-state-proof"
    );
    let reopened = root
        .reopen_quarantined(
            object_id,
            "q-11111111111111111111111111111111",
            &quarantined.identity,
        )
        .unwrap();
    root.delete_quarantined(&reopened).unwrap();
    println!("SOURCE_OF_TRUTH shared_exists=false private_exists=false");
    assert!(!private.join("q-11111111111111111111111111111111").exists());
}

#[test]
fn collision_preserves_both_existing_destination_and_prepared_authority() {
    let (_temp, root, shared, private) = real_root();
    std::fs::create_dir(shared.join("occupied")).unwrap();
    std::fs::write(shared.join("occupied/sentinel"), b"unchanged").unwrap();
    let prepared = root
        .prepare(ObjectId::new("22222222222222222222222222222222").unwrap())
        .unwrap();
    println!("EDGE collision before shared=true prepared=true");
    let result = root.publish(&prepared, LeafName::new("occupied").unwrap());
    println!(
        "EDGE collision after shared={} prepared={}",
        shared.join("occupied").exists(),
        private.join("p-22222222222222222222222222222222").exists()
    );
    assert!(matches!(result, Err(FsTxError::Collision(_))));
    assert_eq!(
        std::fs::read(shared.join("occupied/sentinel")).unwrap(),
        b"unchanged"
    );
    assert!(private.join("p-22222222222222222222222222222222").is_dir());
}

#[test]
fn replacement_is_reported_and_never_deleted_or_adopted() {
    let (_temp, root, shared, _private) = real_root();
    let object_id = ObjectId::new("33333333333333333333333333333333").unwrap();
    let prepared = root.prepare(object_id.clone()).unwrap();
    let published = root
        .publish(&prepared, LeafName::new("victim").unwrap())
        .unwrap();
    std::fs::rename(shared.join("victim"), shared.join("original-moved")).unwrap();
    std::fs::create_dir(shared.join("victim")).unwrap();
    std::fs::write(shared.join("victim/attacker-proof"), b"must-survive").unwrap();
    println!("EDGE replacement before replacement=true original=true");
    let result = root.reopen_published(
        object_id,
        LeafName::new("victim").unwrap(),
        &published.identity,
    );
    println!(
        "EDGE replacement after replacement={} original={}",
        shared.join("victim").exists(),
        shared.join("original-moved").exists()
    );
    assert!(matches!(result, Err(FsTxError::IdentityMismatch { .. })));
    assert_eq!(
        std::fs::read(shared.join("victim/attacker-proof")).unwrap(),
        b"must-survive"
    );
    assert!(shared.join("original-moved").is_dir());
}
