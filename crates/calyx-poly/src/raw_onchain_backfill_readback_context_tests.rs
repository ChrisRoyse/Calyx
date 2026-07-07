use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn issue213_streaming_readback_hashes_valid_json_without_materializing_value() {
    let root = test_root("valid");
    fs::create_dir_all(&root).expect("create test root");
    let body = root.join("body.json");
    let bytes = br#"{"events":[{"block":1},{"block":2}],"ok":true}"#;
    fs::write(&body, bytes).expect("write body");

    let expected = sha256_hex(bytes);
    let mut ctx = ReadbackContext::new(&root, "readback-progress.jsonl").expect("context");
    ctx.check_artifact_sha(&body, &expected, ReadbackArtifactKind::Body, true)
        .expect("streaming readback");
    ctx.check_artifact_sha(&body, &expected, ReadbackArtifactKind::Body, true)
        .expect("deduped readback");

    assert!(ctx.sha_mismatches.is_empty());
    assert!(ctx.parse_failures.is_empty());
    assert_eq!(ctx.unique_file_read_count, 1);
    assert_eq!(ctx.deduplicated_file_read_count, 1);
    assert_eq!(ctx.readback_body_bytes_read, bytes.len() as u64);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn issue213_streaming_readback_records_parse_failure_and_full_sha() {
    let root = test_root("invalid");
    fs::create_dir_all(&root).expect("create test root");
    let body = root.join("body.json");
    let bytes = br#"{"events":[1,2"#;
    fs::write(&body, bytes).expect("write body");

    let expected = sha256_hex(bytes);
    let mut ctx = ReadbackContext::new(&root, "readback-progress.jsonl").expect("context");
    ctx.check_artifact_sha(&body, &expected, ReadbackArtifactKind::Body, true)
        .expect("streaming readback");

    assert!(ctx.sha_mismatches.is_empty());
    assert_eq!(ctx.parse_failures.len(), 1);
    assert_eq!(ctx.unique_file_read_count, 1);
    assert_eq!(ctx.readback_body_bytes_read, bytes.len() as u64);
    assert_eq!(ctx.artifacts[&ctx.path_key(&body)].actual_sha256, expected);
    let _ = fs::remove_dir_all(root);
}

fn test_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-poly-issue213-readback-{name}-{}-{nanos}",
        std::process::id()
    ))
}
