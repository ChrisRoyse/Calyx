use super::*;
use calyx_core::LensId;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: Timestamp = 1_785_500_000_000;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let next = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "calyx-supply-chain-{name}-{}-{next}",
        std::process::id()
    ));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn lock_file(text: &str) -> PathBuf {
    temp_file("Cargo.lock", text.as_bytes())
}

fn synthetic_lock() -> &'static str {
    r#"
version = 4

[[package]]
name = "alpha"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaa"

[[package]]
name = "beta"
version = "2.0.0"
source = "git+https://example.invalid/repo"

[[package]]
name = "gamma"
version = "3.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "cccccccc"
"#
}

#[test]
fn generate_sbom_extracts_synthetic_lock_entries() {
    let path = lock_file(synthetic_lock());
    let sbom = generate_sbom(&path, NOW).unwrap();

    assert_eq!(sbom.generated_at, NOW);
    assert_eq!(sbom.entries.len(), 3);
    assert_eq!(sbom.entries[0].crate_name, "alpha");
    assert_eq!(sbom.entries[0].version, "1.0.0");
    assert_eq!(sbom.entries[0].checksum.as_deref(), Some("aaaaaaaa"));
    assert_eq!(sbom.entries[1].crate_name, "beta");
    assert_eq!(sbom.entries[1].checksum, None);
    assert_eq!(
        sbom.entries[2].source,
        "registry+https://github.com/rust-lang/crates.io-index"
    );
}

#[test]
fn sbom_edges_cover_missing_checksum_empty_and_invalid_lock() {
    let missing_checksum = lock_file(
        r#"
[[package]]
name = "no-checksum"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
    );
    let sbom = generate_sbom(&missing_checksum, NOW).unwrap();
    println!("EDGE_MISSING_CHECKSUM_BEFORE package=no-checksum checksum=<missing>");
    println!(
        "EDGE_MISSING_CHECKSUM_AFTER entries={} checksum={:?}",
        sbom.entries.len(),
        sbom.entries[0].checksum
    );
    assert_eq!(sbom.entries.len(), 1);
    assert_eq!(sbom.entries[0].checksum, None);

    let empty = lock_file("");
    let empty_sbom = generate_sbom(&empty, NOW).unwrap();
    println!("EDGE_EMPTY_LOCK_BEFORE bytes=0");
    println!("EDGE_EMPTY_LOCK_AFTER entries={}", empty_sbom.entries.len());
    assert!(empty_sbom.entries.is_empty());

    let invalid = lock_file("[[package]");
    let error = generate_sbom(&invalid, NOW).unwrap_err();
    println!("EDGE_INVALID_LOCK_AFTER Err({})", error.code);
    assert_eq!(error.code, CALYX_SBOM_PARSE_ERROR);
}

#[test]
fn verify_lens_weight_hash_accepts_match_and_rejects_tamper() {
    let path = temp_file("weights.bin", b"known lens weights");
    let lens_id = LensId::from_bytes(lens_weight_content_address(b"known lens weights"));

    assert!(verify_lens_weight_hash(&lens_id, &path).is_ok());
    std::fs::write(&path, b"known lens weightz").unwrap();
    let error = verify_lens_weight_hash(&lens_id, &path).unwrap_err();

    assert_eq!(error.code, CALYX_LENS_WEIGHT_TAMPERED);
}

#[test]
fn missing_weights_file_fails_closed() {
    let missing =
        std::env::temp_dir().join(format!("calyx-supply-chain-missing-{}", std::process::id()));
    let lens_id = LensId::from_bytes([0u8; 16]);
    let error = verify_lens_weight_hash(&lens_id, &missing).unwrap_err();
    println!("EDGE_MISSING_WEIGHTS_AFTER Err({})", error.code);
    assert_eq!(error.code, CALYX_LENS_WEIGHT_TAMPERED);
}

#[test]
fn assert_audit_clean_classifies_results() {
    assert!(assert_audit_clean(&CargoAuditResult::Clean).is_ok());
    assert!(assert_audit_clean(&CargoAuditResult::ToolNotFound).is_ok());

    let vuln = AuditVulnEntry {
        package: "badcrate".to_string(),
        version: "0.1.0".to_string(),
        advisory_id: "RUSTSEC-2099-0001".to_string(),
        description: "synthetic advisory".to_string(),
    };
    let error = assert_audit_clean(&CargoAuditResult::Vulnerabilities(vec![vuln])).unwrap_err();
    assert_eq!(error.code, CALYX_SUPPLY_CHAIN_VULN);

    let parse_error =
        assert_audit_clean(&CargoAuditResult::ParseError("bad json".to_string())).unwrap_err();
    assert_eq!(parse_error.code, CALYX_SBOM_PARSE_ERROR);
}

#[test]
fn run_cargo_audit_tool_not_found_never_panics() {
    let path = lock_file(synthetic_lock());
    let result = run_cargo_audit_with_command(Path::new("/definitely/not/cargo"), &path);
    assert_eq!(result, CargoAuditResult::ToolNotFound);
}

#[test]
fn audit_json_vulnerabilities_parse_to_structured_entries() {
    let report = json!({
        "vulnerabilities": {
            "list": [{
                "package": {"name": "badcrate", "version": "0.1.0"},
                "advisory": {"id": "RUSTSEC-2099-0001", "title": "synthetic advisory"}
            }]
        }
    });
    let parsed = parse_audit_json(report.to_string().as_bytes()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].package, "badcrate");
    assert_eq!(parsed[0].advisory_id, "RUSTSEC-2099-0001");
}

#[test]
fn supply_chain_fsv_readback_prints_known_outcomes() {
    let lock = lock_file(synthetic_lock());
    let sbom = generate_sbom(&lock, NOW).unwrap();
    println!("FSV_ISSUE506_SBOM_ENTRY_COUNT={}", sbom.entries.len());
    println!(
        "FSV_ISSUE506_SBOM_BETA_CHECKSUM={:?}",
        sbom.entries[1].checksum
    );

    let weights = temp_file("fsv-weights.bin", b"issue506-known-weights");
    let lens_id = LensId::from_bytes(lens_weight_content_address(b"issue506-known-weights"));
    println!(
        "FSV_ISSUE506_LENS_MATCH={:?}",
        verify_lens_weight_hash(&lens_id, &weights)
    );
    std::fs::write(&weights, b"issue506-known-weightz").unwrap();
    let error = verify_lens_weight_hash(&lens_id, &weights).unwrap_err();
    println!("FSV_ISSUE506_TAMPERED_ERROR={}", error.code);

    let audit_result = CargoAuditResult::ToolNotFound;
    println!(
        "FSV_ISSUE506_ASSERT_AUDIT_TOOL_NOT_FOUND={:?}",
        assert_audit_clean(&audit_result)
    );

    assert_eq!(sbom.entries.len(), 3);
    assert_eq!(error.code, CALYX_LENS_WEIGHT_TAMPERED);
    assert!(assert_audit_clean(&audit_result).is_ok());
}

#[test]
#[ignore = "manual FSV against real Cargo.lock, cargo audit, and TEI weights"]
fn issue506_actual_manual_fsv() {
    let cargo_lock = std::env::var("CALYX_FSV_CARGO_LOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("Cargo.lock"));
    let sbom = generate_sbom(&cargo_lock, NOW).unwrap();
    println!(
        "FSV_ISSUE506_ACTUAL_SBOM_COUNT={} path={}",
        sbom.entries.len(),
        cargo_lock.display()
    );
    if let Some(first) = sbom.entries.first() {
        println!(
            "FSV_ISSUE506_ACTUAL_FIRST_CRATE={}@{} checksum_present={}",
            first.crate_name,
            first.version,
            first.checksum.is_some()
        );
    }

    let audit = run_cargo_audit(&cargo_lock);
    println!("FSV_ISSUE506_ACTUAL_AUDIT_RESULT={audit:?}");
    println!(
        "FSV_ISSUE506_ACTUAL_ASSERT_AUDIT={:?}",
        assert_audit_clean(&audit)
    );
    assert!(!matches!(audit, CargoAuditResult::ParseError(_)));

    let weights_path = PathBuf::from(
        std::env::var("CALYX_FSV_LENS_WEIGHTS")
            .expect("CALYX_FSV_LENS_WEIGHTS must point at a real TEI model file"),
    );
    let weights = std::fs::read(&weights_path).unwrap();
    let lens_id = LensId::from_bytes(lens_weight_content_address(&weights));
    println!(
        "FSV_ISSUE506_ACTUAL_WEIGHTS={} bytes={} lens_id={}",
        weights_path.display(),
        weights.len(),
        lens_id
    );
    println!(
        "FSV_ISSUE506_ACTUAL_LENS_VERIFY={:?}",
        verify_lens_weight_hash(&lens_id, &weights_path)
    );
    assert!(verify_lens_weight_hash(&lens_id, &weights_path).is_ok());
}
