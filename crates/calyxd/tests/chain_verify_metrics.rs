//! End-to-end chain-verify metric tests against the real `calyxd` binary:
//! synthetic ledgers with hand-computable outcomes, byte-level tampering,
//! and a live loopback `/metrics` scrape.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{ActorId, DirectoryLedgerStore, EntryKind, LedgerAppender, SubjectId};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn intact_ledger_dir_emits_ok_one_with_entry_count() {
    let dir = test_dir("intact");
    let ledger = dir.join("ledger-cf");
    write_ledger(&ledger, 5);

    let output = run_once(&["--ledger"], &ledger);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let label = label_value(&ledger);
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_ok{{vault=\"{label}\"}} 1"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_entries{{vault=\"{label}\"}} 5"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_runs_total{{outcome=\"intact\",vault=\"{label}\"}} 1"),
    );
    cleanup(dir);
}

#[test]
fn tampered_ledger_row_emits_ok_zero_and_broken_outcome() {
    let dir = test_dir("tampered");
    let ledger = dir.join("ledger-cf");
    write_ledger(&ledger, 5);
    flip_last_byte(&ledger.join("0000000000000002.ledger"));

    let output = run_once(&["--ledger"], &ledger);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let label = label_value(&ledger);
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_ok{{vault=\"{label}\"}} 0"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_entries{{vault=\"{label}\"}} 0"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_runs_total{{outcome=\"broken\",vault=\"{label}\"}} 1"),
    );
    assert!(
        stderr(&output).contains("CALYX_LEDGER_CHAIN_BROKEN at seq=2"),
        "stderr must name the broken seq: {}",
        stderr(&output)
    );
    cleanup(dir);
}

#[test]
fn truncated_ledger_row_emits_ok_zero_and_corrupt_outcome() {
    let dir = test_dir("truncated");
    let ledger = dir.join("ledger-cf");
    write_ledger(&ledger, 3);
    truncate_file(&ledger.join("0000000000000001.ledger"), 12);

    let output = run_once(&["--ledger"], &ledger);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let label = label_value(&ledger);
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_ok{{vault=\"{label}\"}} 0"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_runs_total{{outcome=\"corrupt\",vault=\"{label}\"}} 1"),
    );
    assert!(
        stderr(&output).contains("CALYX_LEDGER_CORRUPT"),
        "stderr must carry the corrupt code: {}",
        stderr(&output)
    );
    cleanup(dir);
}

#[test]
fn empty_ledger_dir_is_vacuously_intact_with_zero_entries() {
    let dir = test_dir("empty");
    let ledger = dir.join("ledger-cf");
    fs::create_dir_all(&ledger).unwrap();

    let output = run_once(&["--ledger"], &ledger);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    let label = label_value(&ledger);
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_ok{{vault=\"{label}\"}} 1"),
    );
    assert_line(
        &text,
        &format!("calyx_ledger_chain_verify_entries{{vault=\"{label}\"}} 0"),
    );
    cleanup(dir);
}

#[test]
fn missing_target_directory_exits_config_invalid() {
    let output = run_once(&["--ledger"], Path::new("Z:/missing/calyxd-602-ledger"));

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CALYX_DAEMON_CONFIG_INVALID"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn non_loopback_bind_exits_bind_failed() {
    let dir = test_dir("bindfail");
    let ledger = dir.join("ledger-cf");
    write_ledger(&ledger, 1);

    let output = Command::new(env!("CARGO_BIN_EXE_calyxd"))
        .arg("--ledger")
        .arg(&ledger)
        .arg("--bind")
        .arg("0.0.0.0:0")
        .output()
        .expect("run calyxd");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CALYX_DAEMON_BIND_FAILED"),
        "stderr: {}",
        stderr(&output)
    );
    cleanup(dir);
}

#[test]
fn http_scrape_serves_gauge_on_loopback() {
    let dir = test_dir("http");
    let ledger = dir.join("ledger-cf");
    write_ledger(&ledger, 4);

    let mut child = Command::new(env!("CARGO_BIN_EXE_calyxd"))
        .arg("--ledger")
        .arg(&ledger)
        .arg("--bind")
        .arg("127.0.0.1:0")
        .arg("--interval-secs")
        .arg("3600")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn calyxd");
    let addr = read_serving_addr(&mut child);

    let body = http_get(&addr, "/metrics");
    let label = label_value(&ledger);
    assert!(body.starts_with("HTTP/1.1 200 OK"), "response: {body}");
    assert!(body.contains("Content-Type: text/plain; version=0.0.4"));
    assert_line(
        &body,
        &format!("calyx_ledger_chain_verify_ok{{vault=\"{label}\"}} 1"),
    );

    let not_found = http_get(&addr, "/health");
    assert!(
        not_found.starts_with("HTTP/1.1 404"),
        "response: {not_found}"
    );

    child.kill().expect("kill calyxd");
    let _ = child.wait();
    cleanup(dir);
}

#[test]
#[ignore = "manual FSV for issue #602: builds synthetic ledgers under CALYX_FSV_ROOT"]
fn issue602_fsv_build_synthetic_ledgers() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .expect("CALYX_FSV_ROOT must point at the FSV evidence dir");
    for name in ["ledger-intact", "ledger-tampered", "ledger-truncated"] {
        let _ = fs::remove_dir_all(root.join(name));
    }

    // 3 entries appended at FixedClock(10): hand-computed expectation is
    // entries=3 / ok=1 for the intact copy; broken at seq=1 for the tampered
    // copy; corrupt at seq=1 for the truncated copy.
    let intact = root.join("ledger-intact");
    write_ledger(&intact, 3);

    let tampered = root.join("ledger-tampered");
    write_ledger(&tampered, 3);
    flip_last_byte(&tampered.join("0000000000000001.ledger"));

    let truncated = root.join("ledger-truncated");
    write_ledger(&truncated, 3);
    truncate_file(&truncated.join("0000000000000001.ledger"), 12);

    println!("ISSUE602_FSV_INTACT={}", intact.display());
    println!("ISSUE602_FSV_TAMPERED={}", tampered.display());
    println!("ISSUE602_FSV_TRUNCATED={}", truncated.display());
}

fn write_ledger(dir: &Path, count: usize) {
    let store = DirectoryLedgerStore::open(dir).expect("open ledger dir");
    let mut appender = LedgerAppender::open(store, FixedClock::new(10)).expect("open appender");
    for seq in 0..count {
        appender
            .append(
                EntryKind::Ingest,
                SubjectId::Cx(CxId::from_bytes([seq as u8; 16])),
                format!("calyxd-602-payload-{seq}").into_bytes(),
                ActorId::Service("calyxd-602-test".to_string()),
            )
            .expect("append");
    }
}

fn run_once<const N: usize>(prefix: &[&str; N], path: &Path) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyxd"));
    for arg in prefix {
        command.arg(arg);
    }
    command.arg(path).arg("--once");
    command.output().expect("run calyxd --once")
}

fn read_serving_addr(child: &mut Child) -> String {
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = lines
            .next()
            .expect("calyxd exited before serving")
            .expect("read calyxd stdout");
        if let Some(rest) = line.strip_prefix("calyxd: serving /metrics on ") {
            let addr = rest.split_whitespace().next().expect("addr token");
            return addr.to_string();
        }
    }
}

fn http_get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to calyxd");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("send request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

/// Escapes a path for an exposition-format label-value expectation (the
/// encoder escapes backslash and double-quote in label values).
fn label_value(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn assert_line(text: &str, expected: &str) {
    assert!(
        text.lines().any(|line| line == expected),
        "expected line {expected:?} in:\n{text}"
    );
}

fn flip_last_byte(path: &Path) {
    let mut bytes = fs::read(path).expect("read ledger row");
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(path, bytes).expect("write tampered row");
}

fn truncate_file(path: &Path, keep: usize) {
    let bytes = fs::read(path).expect("read ledger row");
    fs::write(path, &bytes[..keep]).expect("write truncated row");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyxd-602-{name}-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
