//! End-to-end stdio tests for the `calyx-leapable` engine process.
//!
//! These drive the real compiled binary and assert on raw stdout/stderr plus
//! durable vault bytes under the sidecar-provided data root.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::verify_restore::verify_restore;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId, VaultStore, content_address,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use ulid::Ulid;

const TEST_MASTER_KEY_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-leapable-stdio-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create data root");
        Self {
            path: path.canonicalize().expect("canonical data root"),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn line(value: Value) -> String {
    let mut out = serde_json::to_string(&value).unwrap();
    out.push('\n');
    out
}

fn request(id: u64, method: &str, params: Value) -> String {
    line(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))
}

fn storage_dir(root: &Path, vault_ref: &str) -> PathBuf {
    if vault_ref.ends_with(".calyx") {
        root.join(vault_ref)
    } else {
        root.join(format!("{vault_ref}.calyx"))
    }
}

fn vault_id_for(vault_ref: &str) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes(content_address([vault_ref.as_bytes()])))
}

fn salt_for(vault_ref: &str) -> Vec<u8> {
    content_address([
        b"calyx-leapable-vault-salt".as_slice(),
        vault_ref.as_bytes(),
    ])
    .to_vec()
}

fn seed_vault(root: &Path, vault_ref: &str, seeds: &[u8]) {
    let dir = storage_dir(root, vault_ref);
    let vault = AsterVault::new_durable(
        &dir,
        vault_id_for(vault_ref),
        salt_for(vault_ref),
        VaultOptions::default(),
    )
    .expect("open seeded vault");
    for seed in seeds {
        vault
            .put(sample_constellation(&vault, *seed))
            .expect("put seeded constellation");
    }
    vault.flush().expect("flush seeded vault");
}

fn sample_constellation(vault: &AsterVault, seed: u8) -> Constellation {
    let input = [seed; 8];
    Constellation {
        cx_id: vault.cx_id_for_input(&input, 1),
        vault_id: vault.vault_id(),
        panel_version: 1,
        created_at: 1_785_500_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 4,
                data: vec![f32::from(seed); 4],
            },
        )]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "calyx-leapable-stdio-test".to_string(),
            observed_at: 1_785_500_000,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn run_engine(input: &str, root: &Path, panic_probe: bool) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_calyx-leapable");
    let mut command = Command::new(exe);
    command.arg("--data-dir").arg(root);
    command.env("CALYX_LEAPABLE_MASTER_KEY_HEX", TEST_MASTER_KEY_HEX);
    if panic_probe {
        command.env("CALYX_LEAPABLE_ENABLE_PANIC_PROBE", "1");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn calyx-leapable");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for calyx-leapable");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
        output.status.success(),
    )
}

fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON-RPC"))
        .collect()
}

fn assert_no_json_on_stderr(stderr: &str) {
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON: {stderr:?}"
    );
}

#[test]
fn multiplexes_two_vaults_and_survives_panic_request() {
    let root = TestRoot::new("multiplex-panic");
    assert!(!root.path().join("alpha").exists(), "before: alpha absent");
    assert!(!root.path().join("beta").exists(), "before: beta absent");

    let input = [
        request(1, "engine.info", json!({})),
        request(
            2,
            "vault.create",
            json!({"vault_ref": "alpha", "ts": 1785500000_u64}),
        ),
        request(
            3,
            "vault.create",
            json!({"vault_ref": "beta", "ts": 1785500010_u64}),
        ),
        request(
            4,
            "vault.stat",
            json!({"vault_ref": "alpha", "ts": 1785500020_u64}),
        ),
        request(5, "engine.panic_probe", json!({})),
        request(
            6,
            "vault.stat",
            json!({"vault_ref": "beta", "ts": 1785500030_u64}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), true);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 6);
    for response in responses.iter().take(4) {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(responses[4]["error"]["code"], -32603);
    assert_eq!(responses[4]["error"]["message"], "internal server error");
    assert!(
        responses[5].get("error").is_none(),
        "post-panic request served"
    );
    assert_eq!(responses[5]["result"]["vault_ref"], "beta");
    assert_eq!(responses[5]["result"]["last_ts"], 1785500030_u64);
    assert!(stderr.contains("calyx-leapable: stdio engine ready"));
    assert!(stderr.contains("request panic isolated"));
    assert_no_json_on_stderr(&stderr);
    assert!(ok, "engine exits cleanly on EOF after panic isolation");

    assert!(
        storage_dir(root.path(), "alpha")
            .join("cf")
            .join("base")
            .exists()
    );
    assert!(
        storage_dir(root.path(), "beta")
            .join("cf")
            .join("base")
            .exists()
    );
    assert!(!wal_files(&storage_dir(root.path(), "alpha")).is_empty());
    assert!(!wal_files(&storage_dir(root.path(), "beta")).is_empty());
}

#[test]
fn malformed_line_logs_to_stderr_and_next_request_is_served() {
    let root = TestRoot::new("malformed");
    let input = [
        "not-json\n".to_string(),
        request(1, "engine.info", json!({})),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert!(stderr.contains("CALYX_MCP_JSONRPC_INVALID"));
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

#[test]
fn unsafe_vault_ref_is_rejected_without_creating_escape_dir() {
    let root = TestRoot::new("unsafe-ref");
    let input = request(
        1,
        "vault.open",
        json!({"vault_ref": "../escape", "ts": 1785500000_u64}),
    );
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["error"]["data"]["calyx_code"],
        "CALYX_LEAPABLE_PATH_INVALID"
    );
    assert!(!root.path().join("escape").exists());
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

#[test]
fn notification_gets_no_response_but_following_request_does() {
    let root = TestRoot::new("notification");
    let input = [
        line(json!({"jsonrpc": "2.0", "method": "engine.info", "params": {}})),
        request(1, "engine.info", json!({})),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

#[test]
fn mutating_notification_is_rejected_without_side_effects() {
    let root = TestRoot::new("mutating-notification");
    let input = [
        line(json!({
            "jsonrpc": "2.0",
            "method": "vault.create",
            "params": {"vault_ref": "ghost", "ts": 1785500000_u64}
        })),
        request(1, "engine.info", json!({})),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert!(stderr.contains("CALYX_LEAPABLE_MUTATION_NOTIFICATION"));
    assert_no_json_on_stderr(&stderr);
    assert!(!storage_dir(root.path(), "ghost").exists());
    assert!(ok);
}

#[test]
fn lifecycle_snapshot_restore_clone_verify_and_delete_round_trip() {
    let root = TestRoot::new("lifecycle");
    seed_vault(root.path(), "life", &[1, 2, 3]);
    assert!(storage_dir(root.path(), "life").join("cf/base").exists());
    assert!(!root.path().join("_snapshots/life.calyx/snap_a").exists());

    let input = [
        request(
            1,
            "vault.open",
            json!({"vault_ref": "life", "ts": 1785500100_u64}),
        ),
        request(
            2,
            "vault.snapshot",
            json!({"vault_ref": "life", "snapshot_ref": "snap_a", "ts": 1785500110_u64}),
        ),
        request(
            3,
            "vault.close",
            json!({"vault_ref": "life", "ts": 1785500120_u64}),
        ),
        request(
            4,
            "vault.delete",
            json!({"vault_ref": "life", "ts": 1785500130_u64}),
        ),
        request(
            5,
            "vault.restore",
            json!({"vault_ref": "life", "snapshot_ref": "snap_a", "ts": 1785500140_u64}),
        ),
        request(
            6,
            "vault.clone",
            json!({"vault_ref": "life", "target_vault_ref": "copy", "ts": 1785500150_u64}),
        ),
        request(
            7,
            "vault.verify",
            json!({"vault_ref": "copy", "ts": 1785500160_u64}),
        ),
        request(
            8,
            "vault.snapshot",
            json!({"vault_ref": "life", "snapshot_ref": "snap_a", "ts": 1785500170_u64}),
        ),
        request(
            9,
            "vault.delete",
            json!({"vault_ref": "life", "ts": 1785500180_u64}),
        ),
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 9);
    for response in responses.iter().take(7) {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(responses[1]["result"]["verify_restore"]["success"], true);
    assert_eq!(responses[4]["result"]["verify_restore"]["success"], true);
    assert_eq!(responses[5]["result"]["verify_restore"]["success"], true);
    assert_eq!(responses[6]["result"]["verify_restore"]["success"], true);
    assert_eq!(
        responses[7]["error"]["data"]["calyx_code"],
        "CALYX_LEAPABLE_VAULT_ALREADY_EXISTS"
    );
    assert!(responses[8].get("error").is_none(), "{}", responses[8]);
    assert_no_json_on_stderr(&stderr);
    assert!(ok);

    assert!(
        root.path()
            .join("_snapshots/life.calyx/snap_a/cf/base")
            .exists()
    );
    assert!(!storage_dir(root.path(), "life").exists());
    assert!(storage_dir(root.path(), "copy").join("cf/base").exists());
    assert!(!wal_files(&storage_dir(root.path(), "copy")).is_empty());
}

#[test]
#[ignore = "manual FSV seeder for Leapable issue #1973"]
fn fsv_seed_lifecycle_vault_for_manual_inspection() {
    let root = PathBuf::from(
        std::env::var("CALYX_ISSUE1973_FSV_ROOT").expect("set CALYX_ISSUE1973_FSV_ROOT"),
    );
    fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = storage_dir(&root, "life");
    assert!(
        !vault_dir.exists(),
        "FSV seed target must be fresh: {}",
        vault_dir.display()
    );

    seed_vault(&root, "life", &[1, 2, 3]);
    let report = verify_restore(&vault_dir).expect("verify seeded lifecycle vault");
    println!("issue-1973 fsv root: {}", root.display());
    println!("issue-1973 fsv seed report: {report:?}");
    assert!(report.success(), "seeded vault must verify");
}

fn wal_files(vault_root: &Path) -> Vec<String> {
    let mut files = fs::read_dir(vault_root.join("wal"))
        .expect("read wal dir")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.ends_with(".wal").then_some(name)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}
