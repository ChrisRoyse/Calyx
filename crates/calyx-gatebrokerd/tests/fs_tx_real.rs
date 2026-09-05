//! Real opaque-handle filesystem transaction validation.
//!
//! These cases require `CAP_DAC_READ_SEARCH`. Run them explicitly as root:
//! `cargo test -p calyx-gatebrokerd --test __calyx_integration_platform_0 \
//! fs_tx_real -- --ignored --nocapture --test-threads=1`.

#![cfg(target_os = "linux")]

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use calyx_gatebrokerd::fs_tx::{FsRoot, FsRootSpec, FsTxError};
use calyx_gatebrokerd::protocol::{LeafName, ObjectId, RootAlias};
use tempfile::TempDir;

const PRIVILEGED_REASON: &str =
    "requires root and real open_by_handle_at authority; run explicitly under sudo";
const PRIVILEGED_CASES: [&str; 4] = [
    "prepare_publish_reopen_quarantine_and_delete_change_real_filesystem_state",
    "collision_preserves_both_existing_destination_and_prepared_authority",
    "replacement_is_reported_and_never_deleted_or_adopted",
    "replacement_after_reopen_is_moved_validated_and_restored_without_deletion",
];

fn real_root() -> (TempDir, FsRoot, std::path::PathBuf, std::path::PathBuf) {
    try_real_root().expect("host must provide required opaque handles and descriptor APIs")
}

fn try_real_root() -> Result<(TempDir, FsRoot, std::path::PathBuf, std::path::PathBuf), FsTxError> {
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
    })?;
    Ok((temp, root, shared, private))
}

#[test]
fn opaque_handle_capability_probe_is_explicit() {
    match try_real_root() {
        Ok((_temp, _root, _shared, _private)) => {
            println!("FS_TX_REAL_CAPABILITY available=true");
        }
        Err(FsTxError::CapabilityUnavailable { capability, detail }) => {
            assert_eq!(capability, "open_by_handle_at opaque handle reopen");
            assert!(!detail.is_empty());
            println!(
                "FS_TX_REAL_CAPABILITY available=false capability={capability:?} detail={detail:?}"
            );
        }
        Err(error) => panic!("unexpected opaque-handle capability probe failure: {error}"),
    }
}

#[test]
fn privileged_transaction_cases_remain_explicitly_ignored() {
    let source = include_str!("fs_tx_real.rs");
    for case in PRIVILEGED_CASES {
        let marker = format!("#[ignore = {PRIVILEGED_REASON:?}]\nfn {case}");
        assert!(
            source.contains(&marker),
            "missing privileged marker for {case}"
        );
    }
}

#[test]
#[ignore = "requires root and real open_by_handle_at authority; run explicitly under sudo"]
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
#[ignore = "requires root and real open_by_handle_at authority; run explicitly under sudo"]
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
#[ignore = "requires root and real open_by_handle_at authority; run explicitly under sudo"]
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

#[test]
#[ignore = "requires root and real open_by_handle_at authority; run explicitly under sudo"]
fn replacement_after_reopen_is_moved_validated_and_restored_without_deletion() {
    let (_temp, root, shared, private) = real_root();
    let object_id = ObjectId::new("44444444444444444444444444444444").unwrap();
    let prepared = root.prepare(object_id.clone()).unwrap();
    let published = root
        .publish(&prepared, LeafName::new("race-window").unwrap())
        .unwrap();
    let reopened = root
        .reopen_published(
            object_id,
            LeafName::new("race-window").unwrap(),
            &published.identity,
        )
        .unwrap();

    // This is the former check/use window: the shared name changes only after
    // the broker has a valid descriptor for the registered object.
    std::fs::rename(
        shared.join("race-window"),
        shared.join("registered-object-moved"),
    )
    .unwrap();
    std::fs::create_dir(shared.join("race-window")).unwrap();
    std::fs::write(
        shared.join("race-window/replacement-proof"),
        b"replacement-must-survive",
    )
    .unwrap();

    let before = std::fs::metadata(shared.join("race-window")).unwrap();
    println!(
        "EDGE post_reopen_swap before replacement_dev={} replacement_ino={} registered=true",
        before.dev(),
        before.ino()
    );
    let result = root.quarantine(&reopened);
    let after = std::fs::metadata(shared.join("race-window")).unwrap();
    println!(
        "EDGE post_reopen_swap after replacement_dev={} replacement_ino={} registered=true private_q=false",
        after.dev(),
        after.ino()
    );

    assert!(matches!(
        result,
        Err(FsTxError::IdentityMismatch {
            disposition: calyx_gatebrokerd::fs_tx::MismatchDisposition::RestoredShared,
            ..
        })
    ));
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(
        std::fs::read(shared.join("race-window/replacement-proof")).unwrap(),
        b"replacement-must-survive"
    );
    assert!(shared.join("registered-object-moved").is_dir());
    assert!(!private.join("q-44444444444444444444444444444444").exists());
}
