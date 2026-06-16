use std::ffi::OsStr;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

use serde_json::Value;

#[test]
fn ph62_cli_workflow_readback_healthcheck_and_idempotency() {
    let root = reset_root("ph62-workflow-healthcheck");
    let vault = "ph62-workflow";
    let text = "PH62 known workflow input: why does x fail under load";

    let create = calyx(
        &root,
        ["create-vault", vault, "--panel-template", "text-default"],
    );
    assert_success(&create);
    let vault_id = json(&create)["vault_id"].as_str().unwrap().to_string();
    let vault_path = root.join("vaults").join(&vault_id);

    let add_lens = calyx(
        &root,
        [
            "add-lens",
            vault,
            "--name",
            "ph62-byte",
            "--runtime",
            "algorithmic",
            "--modality",
            "text",
        ],
    );
    assert_success(&add_lens);

    let first_ingest = calyx(&root, ["ingest", vault, "--text", text]);
    assert_success(&first_ingest);
    let first = json(&first_ingest);
    let cx_id = first["cx_id"].as_str().unwrap().to_string();
    assert_eq!(first["new"], true);

    let anchor = calyx(
        &root,
        [
            "anchor",
            vault,
            &cx_id,
            "--kind",
            "test-pass",
            "--value",
            "true",
            "--confidence",
            "1",
            "--source",
            "ph62-workflow-test",
        ],
    );
    assert_success(&anchor);

    let search = calyx(
        &root,
        [
            "search",
            vault,
            "fail under load",
            "--explain",
            "--provenance",
            "--k",
            "3",
        ],
    );
    assert_success(&search);
    let hits = json(&search);
    let hit = hits
        .as_array()
        .unwrap()
        .iter()
        .find(|hit| hit["cx_id"] == cx_id)
        .expect("search should return ingested cx_id");
    assert!(hit["provenance"]["ledger_seq"].as_u64().unwrap() > 0);

    let before_readback = cf_readback(&root, &vault_path, &cx_id);
    assert_success(&before_readback);
    let before_bytes = stdout(&before_readback);
    assert!(!before_bytes.trim().is_empty());
    assert!(compact_hex(&before_bytes).contains(&cx_id));

    let second_ingest = calyx(&root, ["ingest", vault, "--text", text]);
    assert_success(&second_ingest);
    let second = json(&second_ingest);
    assert_eq!(second["cx_id"], cx_id);
    assert_eq!(second["new"], false);

    let after_readback = cf_readback(&root, &vault_path, &cx_id);
    assert_success(&after_readback);
    assert_eq!(before_bytes, stdout(&after_readback));

    let health_url = spawn_http_endpoint();
    let health = calyx(
        &root,
        [
            "healthcheck",
            "--vault",
            vault,
            "--tei",
            &health_url,
            "--json",
        ],
    );
    assert_success(&health);
    let report = json(&health);
    assert_eq!(report["status"], "pass");
    let vault_check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "vault")
        .unwrap();
    assert!(vault_check["n_cx"].as_u64().unwrap() >= 1);

    fs::remove_dir_all(root).unwrap();
}

fn cf_readback(root: &Path, vault_path: &Path, cx_id: &str) -> Output {
    calyx(
        root,
        [
            OsStr::new("readback"),
            OsStr::new("--cf-row"),
            vault_path.as_os_str(),
            OsStr::new("--cf"),
            OsStr::new("base"),
            OsStr::new("--key"),
            OsStr::new(cx_id),
        ],
    )
}

fn calyx<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .env("CALYX_HOME", root)
        .args(args)
        .output()
        .expect("run calyx")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse stdout JSON: {error}\nstdout: {}\nstderr: {}",
            stdout(output),
            stderr(output)
        )
    })
}

fn compact_hex(output: &str) -> String {
    output.chars().filter(char::is_ascii_hexdigit).collect()
}

fn spawn_http_endpoint() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::{Read, Write};
            let mut buf = [0_u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK");
        }
    });
    format!("http://127.0.0.1:{port}/health")
}

fn reset_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
