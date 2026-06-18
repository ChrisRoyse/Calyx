use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::data::VideoMetadata;
use super::request::{DEFAULT_VAULT_ID, VideoCommand, VideoRequest};

#[test]
fn video_validate_request_parses_required_paths() {
    let args = strings([
        "--metadata",
        "metadata/video_metadata.jsonl",
        "--dataset-root",
        "/datasets/media_fsv_mini",
        "--metrics-dir",
        "/tmp/metrics",
        "--vault",
        "/tmp/vault",
    ]);
    let parsed = VideoRequest::parse("video-validate", &args).unwrap();
    let VideoCommand::Validate(request) = parsed else {
        panic!("expected validate request");
    };
    assert_eq!(
        request.metadata,
        PathBuf::from("metadata/video_metadata.jsonl")
    );
    assert_eq!(
        request.dataset_root,
        Some(PathBuf::from("/datasets/media_fsv_mini"))
    );
    assert_eq!(request.metrics_dir, PathBuf::from("/tmp/metrics"));
    assert_eq!(request.vault, PathBuf::from("/tmp/vault"));
    assert_eq!(request.vault_id, DEFAULT_VAULT_ID);
}

#[test]
fn video_metadata_duplicate_sha_fails_closed() {
    let dir = test_dir("video-duplicate-sha");
    let path = dir.join("video_metadata.jsonl");
    fs::write(
        &path,
        [
            metadata_line(
                "video/a.webm",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            metadata_line(
                "video/b.webm",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ]
        .join("\n"),
    )
    .unwrap();
    let err = VideoMetadata::load(&path).unwrap_err();
    assert!(err.contains("CALYX_FSV_MEDIA_VIDEO_DUPLICATE_SOURCE_SHA256"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn video_metadata_empty_manifest_fails_closed() {
    let dir = test_dir("video-empty");
    let path = dir.join("video_metadata.jsonl");
    fs::write(&path, "\n").unwrap();
    let err = VideoMetadata::load(&path).unwrap_err();
    assert_eq!(err, "CALYX_FSV_MEDIA_VIDEO_EMPTY_METADATA");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn video_readback_requires_vault() {
    let err = VideoRequest::parse("video-readback", &[]).unwrap_err();
    assert_eq!(err, "media video readback requires --vault");
}

fn metadata_line(path: &str, sha256: &str) -> String {
    serde_json::json!({
        "path": path,
        "source_title": "unit",
        "source_url": "https://example.test/video",
        "page_url": "https://example.test/page",
        "license": "CC0",
        "license_url": "https://example.test/license",
        "sha256": sha256,
        "bytes": 42,
        "frame_count": 3,
        "fps": 1.0,
        "width": 16,
        "height": 16,
        "codec": "vp9",
        "container": "matroska,webm",
        "mime": "video/webm"
    })
    .to_string()
}

fn strings<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}

fn test_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("calyx-{name}-{stamp}"));
    fs::create_dir_all(&dir).unwrap();
    assert!(Path::new(&dir).is_dir());
    dir
}
