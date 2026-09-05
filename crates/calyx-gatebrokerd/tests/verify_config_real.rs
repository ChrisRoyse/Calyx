#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::Connection;

#[derive(Debug, PartialEq, Eq)]
struct StateEntry {
    relative: PathBuf,
    kind: &'static str,
    uid: u32,
    gid: u32,
    mode: u32,
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
    bytes: Vec<u8>,
}

#[test]
#[ignore = "requires root and real open_by_handle_at authority; run explicitly under sudo"]
fn verify_config_preserves_exact_filesystem_and_sqlite_state() {
    assert_eq!(unsafe { libc::geteuid() }, 0, "test must run as root");
    let state = tempfile::Builder::new()
        .prefix("calyx-gatebroker-verify-state-")
        .tempdir_in("/var/lib")
        .unwrap();
    let source = tempfile::Builder::new()
        .prefix("calyx-gatebroker-verify-source-")
        .tempdir_in("/var/lib")
        .unwrap();
    chmod(state.path(), 0o711);
    chmod(source.path(), 0o755);

    let private = state.path().join("private");
    let journal_directory = private.join("journal");
    let shared = state.path().join("objects/tmp");
    let quarantine = private.join("quarantine/tmp");
    create_directory(&private, 0o700);
    create_directory(&journal_directory, 0o700);
    create_directory(&state.path().join("objects"), 0o711);
    create_directory(&shared, 0o711);
    create_directory(&private.join("quarantine"), 0o700);
    create_directory(&quarantine, 0o700);

    let journal = journal_directory.join("journal.turso");
    let sqlite = Connection::open(&journal).unwrap();
    assert_eq!(
        sqlite
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    sqlite
        .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA synchronous=FULL;")
        .unwrap();
    sqlite
        .execute_batch(
            "CREATE TABLE physical_proof(id INTEGER PRIMARY KEY, value TEXT NOT NULL);\
             INSERT INTO physical_proof(id, value) VALUES(1455, 'physical-sqlite-proof');",
        )
        .unwrap();
    assert!(journal.with_extension("sqlite-wal").exists());
    assert!(journal.with_extension("sqlite-shm").exists());

    let config = state.path().join("verify.toml");
    fs::write(
        &config,
        config_text(
            state.path(),
            &private,
            &journal_directory,
            &journal,
            &shared,
            &quarantine,
            source.path(),
        ),
    )
    .unwrap();
    chmod(&config, 0o600);

    let before = snapshot(state.path());
    print_evidence("before", &before);
    let output = Command::new(env!("CARGO_BIN_EXE_calyx-gatebrokerd"))
        .args(["verify-config", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("CALYX_GATEBROKER_CONFIG_VERIFIED"));

    let after = snapshot(state.path());
    print_evidence("after", &after);
    assert_eq!(after, before, "verify-config changed durable state");
    let proof = sqlite
        .query_row(
            "SELECT value FROM physical_proof WHERE id=1455",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    println!("SOURCE_OF_TRUTH sqlite.physical_proof[1455]={proof}");
    assert_eq!(proof, "physical-sqlite-proof");
}

fn create_directory(path: &Path, mode: u32) {
    fs::create_dir(path).unwrap();
    chmod(path, mode);
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn snapshot(root: &Path) -> Vec<StateEntry> {
    let mut paths = vec![root.to_path_buf()];
    let mut index = 0;
    while index < paths.len() {
        let path = paths[index].clone();
        index += 1;
        if path.symlink_metadata().unwrap().is_dir() {
            let mut children = fs::read_dir(&path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            paths.extend(children);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let metadata = path.symlink_metadata().unwrap();
            let kind = if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let bytes = if metadata.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            };
            StateEntry {
                relative: path.strip_prefix(root).unwrap().to_path_buf(),
                kind,
                uid: metadata.uid(),
                gid: metadata.gid(),
                mode: metadata.mode() & 0o7777,
                device: metadata.dev(),
                inode: metadata.ino(),
                size: metadata.len(),
                modified_seconds: metadata.mtime(),
                modified_nanoseconds: metadata.mtime_nsec(),
                changed_seconds: metadata.ctime(),
                changed_nanoseconds: metadata.ctime_nsec(),
                bytes,
            }
        })
        .collect()
}

fn print_evidence(label: &str, state: &[StateEntry]) {
    for entry in state {
        if entry.kind == "file" {
            println!(
                "SOURCE_OF_TRUTH {label} path={} inode={} mode={:04o} size={} blake3={}",
                entry.relative.display(),
                entry.inode,
                entry.mode,
                entry.size,
                blake3::hash(&entry.bytes)
            );
        }
    }
}

fn config_text(
    state: &Path,
    private: &Path,
    journal_directory: &Path,
    journal: &Path,
    shared: &Path,
    quarantine: &Path,
    source: &Path,
) -> String {
    format!(
        r#"schema_version = 1
socket_path = "/run/calyx-gatebroker-verify.sock"
journal_path = "{}"
worker_user = "nobody"
client_group = "root"
unit_prefix = "calyx-gate-verify"
max_active_runs = 1
max_rpc_frame_bytes = 65536
max_argv_entries = 256
max_environment_entries = 256

[state]
anchor = "{}"
anchor_owner = "root"
anchor_mode = "0711"
private = "{}"
private_owner = "root"
private_mode = "0700"
journal_directory = "{}"
journal_directory_owner = "root"
journal_directory_mode = "0700"
require_root_owned_path_chain = true
require_no_symlinks = true

[journal]
mode = "mvcc"
synchronous = "full"
foreign_keys = true
integrity_check_on_start = true

[containment]
system_manager = true
cgroup_version = 2
delegate = false
bind_units_to_broker = true
require_pidfd_owner = true
require_held_cgroup_fd = true
allow_user_manager = false
allow_same_uid_stage = false

[roots.tmp]
common_ancestor = "{}"
shared = "{}"
private = "{}"
shared_owner = "root"
shared_mode = "0711"
private_owner = "root"
private_mode = "0700"
published_mode = "0700"
require_same_mount = true
require_rename_noreplace = true
require_opaque_file_handles = true
allow_existing_object_adoption = false

[execution_roots.source]
path = "{}"
expected_owner = "root"
expected_mode = "0755"
read_only = true
require_openat2 = true
require_resolve_beneath = true
require_no_symlinks = true
require_no_magiclinks = true
"#,
        journal.display(),
        state.display(),
        private.display(),
        journal_directory.display(),
        state.display(),
        shared.display(),
        quarantine.display(),
        source.display(),
    )
}
