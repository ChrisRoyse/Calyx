//! Gate G1 durability FSV for Leapable issue #1977.
//!
//! Source of truth is the durable vault directory: WAL segment bytes,
//! manifest bytes, and Base CF SST rows are decoded directly.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::verify_restore::{VerifyRestoreReport, verify_restore};
use calyx_aster::wal::replay_dir;
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef,
    LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

const CHILD_ENV: &str = "CALYX_ISSUE1977_CHILD_WRITER";
const FSV_ROOT_ENV: &str = "CALYX_ISSUE1977_FSV_ROOT";
const TEXT_KEY: &str = "issue1977_text";
const CASE_KEY: &str = "issue1977_case";
const VAULT_SALT: &[u8] = b"leapable-issue-1977-durability-salt";
const EXPECTED_FLUSHED: usize = 3;
const EXPECTED_TOTAL: usize = 5;

#[test]
fn issue1977_kill_recover_wal_tail_and_torn_tail_fsv() {
    if std::env::var_os(CHILD_ENV).is_some() {
        child_writer();
        return;
    }

    let fixture = FixtureRoot::new("kill-recover");
    let vault_dir = fixture.root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create vault dir");

    let mut child = Command::new(std::env::current_exe().expect("current test exe"))
        .arg("--exact")
        .arg("issue1977_kill_recover_wal_tail_and_torn_tail_fsv")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env(FSV_ROOT_ENV, &fixture.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child writer");

    let ready_path = fixture.root.join("ready-to-kill.json");
    wait_for_path(&ready_path, Duration::from_secs(15));
    let expected = read_committed(&fixture.root);
    assert_eq!(expected.len(), EXPECTED_TOTAL);
    assert_eq!(
        expected
            .iter()
            .filter(|record| record.checkpoint_flushed)
            .count(),
        EXPECTED_FLUSHED
    );

    let before_kill = collect_vault_state(&vault_dir).expect("collect before-kill state");
    assert_eq!(before_kill.wal_base_rows.len(), EXPECTED_TOTAL);
    assert_eq!(before_kill.sst_base_rows.len(), EXPECTED_FLUSHED);
    println!(
        "ISSUE1977 BEFORE KILL {}",
        serde_json::to_string_pretty(&before_kill).unwrap()
    );

    child.kill().expect("kill child writer");
    let status = child.wait().expect("wait child writer");
    assert!(
        !status.success(),
        "child must be externally terminated, got {status}"
    );

    let after_kill = collect_vault_state(&vault_dir).expect("collect after-kill state");
    assert_eq!(
        before_kill.wal_files, after_kill.wal_files,
        "TerminateProcess/kill must not rewrite committed WAL segment bytes"
    );

    let recovered = AsterVault::open(&vault_dir, vault_id(), VAULT_SALT, VaultOptions::default())
        .expect("cold-open killed vault");
    let recovery_report = recovered.recovery_report().clone();
    assert_eq!(recovery_report.last_recovered_seq, EXPECTED_TOTAL as u64);
    assert_eq!(recovery_report.torn_tail, None);

    for record in &expected {
        let cx_id = parse_cx_id(&record.cx_id);
        let got = recovered
            .get(cx_id, recovered.snapshot())
            .expect("read recovered constellation");
        assert_eq!(got.metadata_value(TEXT_KEY), Some(record.text.as_str()));
        assert_eq!(got.metadata_value(CASE_KEY), Some(record.case.as_str()));
    }

    recovered
        .flush()
        .expect("checkpoint recovered WAL tail into SST bytes");
    drop(recovered);

    let after_recovery = collect_vault_state(&vault_dir).expect("collect after-recovery state");
    assert_eq!(after_recovery.sst_base_rows.len(), EXPECTED_TOTAL);
    for record in &expected {
        let wal = before_kill
            .wal_base_rows
            .get(&record.cx_id)
            .expect("expected WAL base row");
        let sst = after_recovery
            .sst_base_rows
            .get(&record.cx_id)
            .expect("expected SST base row after recovery");
        assert_eq!(sst.text, record.text);
        assert_eq!(
            sst.value_sha256, wal.value_sha256,
            "Base CF SST bytes must match the committed WAL base row for {}",
            record.cx_id
        );
    }

    let restore = verify_restore(&vault_dir).expect("verify restored killed vault");
    assert!(
        restore.success(),
        "verify_restore failed: {:?}",
        restore.failure_reasons()
    );

    let torn =
        append_torn_tail_and_recover(&vault_dir, &expected).expect("torn tail recovery edge case");
    let final_state = collect_vault_state(&vault_dir).expect("collect final state");
    println!(
        "ISSUE1977 AFTER RECOVERY {}",
        serde_json::to_string_pretty(&json!({
            "recovery_report": {
                "last_recovered_seq": recovery_report.last_recovered_seq,
                "torn_tail": recovery_report.torn_tail.as_ref().map(torn_tail_json),
            },
            "verify_restore": restore,
            "torn_tail_edge": torn,
            "final_state": final_state,
        }))
        .unwrap()
    );

    write_json(
        &fixture.root.join("issue1977-kill-recover-readback.json"),
        &json!({
            "source_of_truth": "WAL segment bytes plus Base CF SST bytes decoded from the durable vault directory",
            "before_kill": before_kill,
            "after_kill": after_kill,
            "after_recovery": after_recovery,
            "verify_restore": restore,
            "torn_tail_edge": torn,
            "expected_records": expected,
        }),
    );
}

#[test]
fn issue1977_parent_dir_sync_through_reparse_root_fsv() {
    let fixture = FixtureRoot::new("reparse-parent-sync");
    let target = fixture.root.join("target");
    let link = fixture.root.join("link-root");
    fs::create_dir_all(&target).expect("create reparse target");
    let link_kind = create_dir_link(&target, &link).expect("create reparse/symlink root");

    let vault_via_link = link.join("vault");
    let vault = AsterVault::new_durable(
        &vault_via_link,
        vault_id(),
        VAULT_SALT,
        VaultOptions::default(),
    )
    .expect("open vault through reparse root");
    let expected = sample_record(31, true);
    let expected_cx = parse_cx_id(&expected.cx_id);
    vault
        .put(sample_constellation(31, "reparse-parent-sync"))
        .expect("put through reparse root");
    vault.flush().expect("flush through reparse root");
    drop(vault);

    let vault_via_target = target.join("vault");
    let link_state = collect_vault_state(&vault_via_link).expect("collect link state");
    let target_state = collect_vault_state(&vault_via_target).expect("collect target state");
    assert_base_rows_match_ignoring_paths(&link_state.sst_base_rows, &target_state.sst_base_rows);

    let reopened = AsterVault::open(
        &vault_via_target,
        vault_id(),
        VAULT_SALT,
        VaultOptions::default(),
    )
    .expect("open target side after link write");
    let got = reopened
        .get(expected_cx, reopened.snapshot())
        .expect("read through target side");
    assert_eq!(got.metadata_value(TEXT_KEY), Some(expected.text.as_str()));
    drop(reopened);

    let restore = verify_restore(&vault_via_target).expect("verify target-side restored vault");
    assert!(
        restore.success(),
        "verify_restore failed: {:?}",
        restore.failure_reasons()
    );
    println!(
        "ISSUE1977 REPARSE PARENT SYNC {}",
        serde_json::to_string_pretty(&json!({
            "link_kind": link_kind,
            "link_path": link,
            "target_path": target,
            "link_state": link_state,
            "target_state": target_state,
            "verify_restore": restore,
        }))
        .unwrap()
    );

    write_json(
        &fixture
            .root
            .join("issue1977-reparse-parent-sync-readback.json"),
        &json!({
            "source_of_truth": "same durable vault bytes read through reparse/symlink root and target root",
            "link_kind": link_kind,
            "link_state": link_state,
            "target_state": target_state,
            "verify_restore": restore,
        }),
    );
}

fn child_writer() {
    let root = PathBuf::from(std::env::var_os(FSV_ROOT_ENV).expect("child FSV root"));
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create child vault dir");
    let vault =
        AsterVault::new_durable(&vault_dir, vault_id(), VAULT_SALT, VaultOptions::default())
            .expect("open child durable vault");

    let mut committed = Vec::new();
    for index in 1..=EXPECTED_FLUSHED {
        let record = sample_record(index as u8, true);
        vault
            .put(sample_constellation(index as u8, "kill-recover"))
            .expect("put flushed record");
        committed.push(record);
        write_committed(&root, &committed);
    }
    vault.flush().expect("checkpoint first committed records");

    for index in (EXPECTED_FLUSHED + 1)..=EXPECTED_TOTAL {
        let record = sample_record(index as u8, false);
        vault
            .put(sample_constellation(index as u8, "kill-recover"))
            .expect("put WAL-tail record");
        committed.push(record);
        write_committed(&root, &committed);
    }

    write_json(
        &root.join("ready-to-kill.json"),
        &json!({
            "pid": std::process::id(),
            "flushed_records": EXPECTED_FLUSHED,
            "wal_tail_records": EXPECTED_TOTAL - EXPECTED_FLUSHED,
            "state": "WAL tail committed; checkpoint flush intentionally withheld",
        }),
    );

    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn append_torn_tail_and_recover(
    vault_dir: &Path,
    expected: &[ExpectedRecord],
) -> std::io::Result<serde_json::Value> {
    let wal_path = latest_wal_path(vault_dir)?;
    let before_bytes = fs::read(&wal_path)?;
    let before_len = before_bytes.len() as u64;
    let mut file = OpenOptions::new().append(true).open(&wal_path)?;
    file.write_all(b"CXW1partial")?;
    file.sync_data()?;
    drop(file);
    let after_append_len = fs::metadata(&wal_path)?.len();

    let reopened = AsterVault::open(vault_dir, vault_id(), VAULT_SALT, VaultOptions::default())
        .expect("open with torn WAL tail");
    let report = reopened.recovery_report().clone();
    let tail = report.torn_tail.expect("torn tail must be reported");
    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(tail.offset, before_len);
    assert_eq!(fs::metadata(&wal_path)?.len(), before_len);
    for record in expected {
        let got = reopened
            .get(parse_cx_id(&record.cx_id), reopened.snapshot())
            .expect("read after torn-tail recovery");
        assert_eq!(got.metadata_value(TEXT_KEY), Some(record.text.as_str()));
    }
    drop(reopened);

    Ok(json!({
        "wal_path": wal_path,
        "before_len": before_len,
        "after_torn_append_len": after_append_len,
        "truncated_len": fs::metadata(&wal_path)?.len(),
        "torn_code": tail.code,
        "torn_offset": tail.offset,
        "tail_message": tail.message,
    }))
}

fn collect_vault_state(vault_dir: &Path) -> std::io::Result<VaultReadback> {
    let wal = read_wal_state(vault_dir)?;
    let sst_base_rows = read_base_sst_rows(vault_dir)?;
    let current = read_optional_string(vault_dir.join("CURRENT"))?;
    let manifest = read_optional_string(vault_dir.join("MANIFEST"))?;
    Ok(VaultReadback {
        vault_dir: vault_dir.display().to_string(),
        current_sha256: current.as_ref().map(|bytes| sha256_hex(bytes.as_bytes())),
        manifest_sha256: manifest.as_ref().map(|bytes| sha256_hex(bytes.as_bytes())),
        wal_files: wal.files,
        wal_record_count: wal.record_count,
        wal_torn_tail: wal.torn_tail,
        wal_base_rows: wal.base_rows,
        sst_base_rows,
    })
}

fn read_wal_state(vault_dir: &Path) -> std::io::Result<WalReadback> {
    let wal_dir = vault_dir.join("wal");
    let files = files_with_hashes(&wal_dir, "wal")?;
    let mut base_rows = BTreeMap::new();
    let mut record_count = 0;
    let mut torn_tail = None;
    if wal_dir.is_dir() {
        let replay = replay_dir(&wal_dir).expect("replay WAL for readback");
        record_count = replay.records.len();
        torn_tail = replay.torn_tail.as_ref().map(|tail| {
            format!(
                "{}:{}:{}",
                tail.segment_path.display(),
                tail.offset,
                tail.message
            )
        });
        for record in replay.records {
            let rows = decode_write_batch(&record.payload).expect("decode WAL write batch");
            for row in rows.into_iter().filter(|row| row.cf == ColumnFamily::Base) {
                let decoded =
                    decode_constellation_base(&row.value).expect("decode WAL Base row value");
                base_rows.insert(
                    decoded.cx_id.to_string(),
                    base_row(record.seq, None, row.value),
                );
            }
        }
    }
    Ok(WalReadback {
        files,
        record_count,
        torn_tail,
        base_rows,
    })
}

fn read_base_sst_rows(vault_dir: &Path) -> std::io::Result<BTreeMap<String, BaseRowReadback>> {
    let mut rows = BTreeMap::new();
    for path in sst_paths(vault_dir)? {
        let reader = SstReader::open(&path).expect("open Base CF SST");
        for entry in reader.iter().expect("iterate Base CF SST") {
            let decoded =
                decode_constellation_base(&entry.value).expect("decode SST Base row value");
            rows.insert(
                decoded.cx_id.to_string(),
                base_row(0, Some(path.display().to_string()), entry.value),
            );
        }
    }
    Ok(rows)
}

fn base_row(wal_seq: u64, sst_path: Option<String>, value: Vec<u8>) -> BaseRowReadback {
    let decoded = decode_constellation_base(&value).expect("decode Base row");
    BaseRowReadback {
        wal_seq,
        sst_path,
        text: decoded
            .metadata_value(TEXT_KEY)
            .expect("issue1977 text metadata")
            .to_string(),
        case: decoded
            .metadata_value(CASE_KEY)
            .expect("issue1977 case metadata")
            .to_string(),
        value_len: value.len(),
        value_sha256: sha256_hex(&value),
    }
}

fn assert_base_rows_match_ignoring_paths(
    left: &BTreeMap<String, BaseRowReadback>,
    right: &BTreeMap<String, BaseRowReadback>,
) {
    assert_eq!(
        left.keys().collect::<Vec<_>>(),
        right.keys().collect::<Vec<_>>()
    );
    for (cx_id, left_row) in left {
        let right_row = right.get(cx_id).expect("right row present");
        assert_eq!(left_row.text, right_row.text);
        assert_eq!(left_row.case, right_row.case);
        assert_eq!(left_row.value_len, right_row.value_len);
        assert_eq!(left_row.value_sha256, right_row.value_sha256);
    }
}

fn sst_paths(vault_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let base_dir = vault_dir.join("cf").join("base");
    if !base_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(base_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sst"));
    paths.sort();
    Ok(paths)
}

fn files_with_hashes(dir: &Path, extension: &str) -> std::io::Result<Vec<FileReadback>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some(extension));
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path)?;
            Ok(FileReadback {
                path: path.display().to_string(),
                len: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
            })
        })
        .collect()
}

fn latest_wal_path(vault_dir: &Path) -> std::io::Result<PathBuf> {
    files_with_hashes(&vault_dir.join("wal"), "wal")?
        .into_iter()
        .map(|file| PathBuf::from(file.path))
        .max()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no WAL files"))
}

fn sample_constellation(index: u8, case: &str) -> Constellation {
    let text = sample_text(index);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![index as f32, index as f32 + 0.25, index as f32 + 0.5],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );

    let mut metadata = BTreeMap::new();
    metadata.insert(TEXT_KEY.to_string(), text.clone());
    metadata.insert(CASE_KEY.to_string(), case.to_string());
    metadata.insert("issue1977_index".to_string(), index.to_string());

    Constellation {
        cx_id: cx_id(index),
        vault_id: vault_id(),
        panel_version: 1977,
        created_at: 1_785_600_000_000 + index as u64,
        input_ref: InputRef {
            hash: *blake3::hash(text.as_bytes()).as_bytes(),
            pointer: Some(format!("synthetic://leapable/issue1977/{index}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: vec![Anchor {
            kind: AnchorKind::Label("issue1977-g1".to_string()),
            value: AnchorValue::Text(format!("anchor-{index}")),
            source: "synthetic-g1-fixture".to_string(),
            observed_at: 1_785_600_000_000 + index as u64,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: index as u64,
            hash: [index; 32],
        },
        flags: CxFlags::default(),
    }
}

fn sample_record(index: u8, checkpoint_flushed: bool) -> ExpectedRecord {
    ExpectedRecord {
        index,
        cx_id: cx_id(index).to_string(),
        text: sample_text(index),
        case: "kill-recover".to_string(),
        checkpoint_flushed,
    }
}

fn sample_text(index: u8) -> String {
    format!("Leapable issue 1977 known text row {index}: WAL replay must preserve this exactly.")
}

fn cx_id(index: u8) -> CxId {
    let mut bytes = [0x77; 16];
    bytes[15] = index;
    CxId::from_bytes(bytes)
}

fn parse_cx_id(value: &str) -> CxId {
    value.parse().expect("parse cx id")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

fn write_committed(root: &Path, committed: &[ExpectedRecord]) {
    write_json(&root.join("committed-records.json"), committed);
}

fn read_committed(root: &Path) -> Vec<ExpectedRecord> {
    serde_json::from_slice(&fs::read(root.join("committed-records.json")).expect("read committed"))
        .expect("decode committed")
}

fn write_json(path: &Path, value: &(impl Serialize + ?Sized)) {
    let bytes = serde_json::to_vec_pretty(value).expect("encode json");
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).expect("write temp json");
    fs::rename(&tmp, path).expect("publish json");
}

fn torn_tail_json(tail: &calyx_aster::wal::TornTail) -> serde_json::Value {
    json!({
        "segment_path": tail.segment_path,
        "offset": tail.offset,
        "code": tail.code,
        "message": tail.message,
    })
}

fn read_optional_string(path: PathBuf) -> std::io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn wait_for_path(path: &Path, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {}", path.display());
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
        Ok("unix-symlink".to_string())
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => Ok("windows-symlink".to_string()),
            Err(symlink_error) => {
                let output = Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .output()?;
                if output.status.success() {
                    Ok("windows-junction".to_string())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!(
                            "symlink_dir failed: {symlink_error}; mklink /J failed: status={} stdout={} stderr={}",
                            output.status,
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        ),
                    ))
                }
            }
        }
    }
}

#[derive(Debug)]
struct FixtureRoot {
    root: PathBuf,
    keep: bool,
}

impl FixtureRoot {
    fn new(name: &str) -> Self {
        if let Some(root) = std::env::var_os(FSV_ROOT_ENV) {
            let root = PathBuf::from(root).join(name);
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("create FSV fixture root");
            return Self { root, keep: true };
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "calyx-issue1977-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp fixture root");
        Self { root, keep: false }
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FileReadback {
    path: String,
    len: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BaseRowReadback {
    wal_seq: u64,
    sst_path: Option<String>,
    text: String,
    case: String,
    value_len: usize,
    value_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct VaultReadback {
    vault_dir: String,
    current_sha256: Option<String>,
    manifest_sha256: Option<String>,
    wal_files: Vec<FileReadback>,
    wal_record_count: usize,
    wal_torn_tail: Option<String>,
    wal_base_rows: BTreeMap<String, BaseRowReadback>,
    sst_base_rows: BTreeMap<String, BaseRowReadback>,
}

struct WalReadback {
    files: Vec<FileReadback>,
    record_count: usize,
    torn_tail: Option<String>,
    base_rows: BTreeMap<String, BaseRowReadback>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExpectedRecord {
    index: u8,
    cx_id: String,
    text: String,
    case: String,
    checkpoint_flushed: bool,
}

#[allow(dead_code)]
fn assert_restore_report_is_serializable(report: &VerifyRestoreReport) {
    serde_json::to_vec(report).expect("serialize restore report");
}
