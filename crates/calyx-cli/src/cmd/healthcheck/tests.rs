use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn try_run_claims_only_ph62_healthcheck_forms() {
    assert!(owns_form(&tokens([])));
    assert!(owns_form(&tokens(["--vault", "v"])));
    assert!(owns_form(&tokens(["--json"])));
    assert!(owns_form(&tokens(["--no-json"])));
    assert!(owns_form(&tokens(["--tei", "127.0.0.1:8088"])));
    assert!(try_run(&tokens(["healthcheck", "--out", "latest.json"])).is_none());
}

#[test]
fn parse_rejects_conflicting_formats_and_missing_vault_value() {
    assert_eq!(
        parse(&tokens(["--json", "--no-json"])).unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse(&tokens(["--vault"])).unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse(&tokens(["--tei"])).unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn parse_accepts_custom_tei_endpoint_forms() {
    let bare = parse(&tokens(["--tei", "127.0.0.1:8088"])).unwrap();
    assert_eq!(
        bare.tei[0],
        Endpoint::new("tei:8088", "127.0.0.1", 8088, "/")
    );

    let with_path = parse(&tokens(["--tei", "http://localhost:8090/health"])).unwrap();
    assert_eq!(
        with_path.tei[0],
        Endpoint::new("tei:8090", "localhost", 8090, "/health")
    );
}

#[test]
fn passing_endpoint_report_is_json_pass() {
    let endpoint = spawn_http_endpoint("tei:test-ok");
    let report = build_report(
        &Args {
            vault: None,
            json: true,
            tei: Vec::new(),
        },
        &[endpoint],
    );

    assert_eq!(report.status, "pass");
    assert_eq!(report.checks[0].name, "engine");
    assert_eq!(report.checks[1].name, "tei:test-ok");
    assert_eq!(report.checks[1].status, "pass");
    assert!(report.checks[1].latency_ms.is_some());
}

#[test]
fn unreachable_endpoint_fails_with_lens_code() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let endpoint = Endpoint::new("tei:closed", "127.0.0.1", port, "/");

    let report = build_report(
        &Args {
            vault: None,
            json: true,
            tei: Vec::new(),
        },
        &[endpoint],
    );
    let error = first_failure(&report).unwrap();

    assert_eq!(report.status, "fail");
    assert_eq!(report.checks[1].code, Some("CALYX_LENS_UNREACHABLE"));
    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
}

#[test]
fn missing_manifest_is_vault_corrupt_check() {
    let root = std::env::temp_dir().join(format!(
        "calyx-healthcheck-missing-current-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("cf/base")).unwrap();

    let error = ensure_manifest(&root).unwrap_err();

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    let _ = std::fs::remove_dir_all(root);
}

fn spawn_http_endpoint(name: &'static str) -> Endpoint {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK");
        }
    });
    Endpoint::new(name, "127.0.0.1", port, "/")
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
