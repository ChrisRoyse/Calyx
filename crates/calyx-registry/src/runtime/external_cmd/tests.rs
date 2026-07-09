use super::*;
use calyx_core::Input;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn resident_external_process_is_reused_across_measurements() {
    let dir = test_dir("external-reuse");
    let marker = dir.join("spawn-count.txt");
    let script = format!(
        r#"
import json, pathlib, struct, sys
p = pathlib.Path({})
p.write_text(str(int(p.read_text()) + 1 if p.exists() else 1))
while True:
    header = sys.stdin.buffer.read(4)
    if not header:
        break
    if len(header) != 4:
        sys.exit(2)
    size = struct.unpack(">I", header)[0]
    payload = json.loads(sys.stdin.buffer.read(size))
    vectors = []
    for item in payload["inputs"]:
        value = (sum(item) % 251) / 251.0
        vectors.append([value, 1.0 - value, 0.5, 0.25])
    body = json.dumps({{"vectors": vectors}}).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(body)))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()
"#,
        serde_json::to_string(marker.to_str().unwrap()).unwrap()
    );
    let lens = ExternalCmdLens::new(
        "external-reuse",
        "python3",
        vec!["-c".to_string(), script],
        Modality::Text,
        4,
    )
    .with_timeout(Duration::from_secs(5));

    let first = lens
        .measure(&Input::new(Modality::Text, b"first".to_vec()))
        .unwrap();
    let second = lens
        .measure(&Input::new(Modality::Text, b"second".to_vec()))
        .unwrap();

    assert_dense_dim(first, 4);
    assert_dense_dim(second, 4);
    assert_eq!(fs::read_to_string(&marker).unwrap(), "1");
    cleanup(dir);
}

#[test]
fn stderr_is_drained_while_external_process_returns_response() {
    let script = r#"
import json, struct, sys
sys.stderr.buffer.write(b"warning-noise\n" * 8192)
sys.stderr.buffer.flush()
header = sys.stdin.buffer.read(4)
size = struct.unpack(">I", header)[0]
json.loads(sys.stdin.buffer.read(size))
body = json.dumps({"vectors": [[0.25, 0.75, 0.5, 0.0]]}).encode()
sys.stdout.buffer.write(struct.pack(">I", len(body)))
sys.stdout.buffer.write(body)
sys.stdout.buffer.flush()
"#;
    let lens = ExternalCmdLens::new(
        "external-stderr-drain",
        "python3",
        vec!["-c".to_string(), script.to_string()],
        Modality::Text,
        4,
    )
    .with_timeout(Duration::from_secs(5));

    let vector = lens
        .measure(&Input::new(Modality::Text, b"stderr-heavy".to_vec()))
        .expect("stderr drain keeps child unblocked");

    assert_dense_dim(vector, 4);
}

#[test]
fn timeout_kills_slow_external_process_before_finished_marker() {
    let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("external-timeout"),
        |root| {
            let dir = root.join("external-timeout");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        },
    );
    let marker = dir.join("marker.txt");
    let before_marker = read_marker(&marker);
    let script = format!(
        "import pathlib,time; p=pathlib.Path({}); p.write_text('started\\n'); time.sleep(2); p.write_text(p.read_text() + 'finished\\n')",
        serde_json::to_string(marker.to_str().unwrap()).unwrap()
    );
    let lens = ExternalCmdLens::new(
        "external-timeout",
        "python3",
        vec!["-c".to_string(), script],
        Modality::Text,
        4,
    )
    .with_timeout(Duration::from_millis(750));

    let started = Instant::now();
    let error = lens
        .measure(&Input::new(Modality::Text, b"slow".to_vec()))
        .expect_err("slow command times out");
    let elapsed = started.elapsed();
    let immediate_marker = read_marker(&marker);
    std::thread::sleep(Duration::from_secs(3));
    let after_wait_marker = read_marker(&marker);

    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
    assert!(error.message.contains("timed out"));
    assert_eq!(before_marker, None);
    assert!(
        !immediate_marker
            .as_deref()
            .unwrap_or("")
            .contains("finished"),
        "timeout returned after child wrote finished marker: {immediate_marker:?}"
    );
    assert!(
        !after_wait_marker
            .as_deref()
            .unwrap_or("")
            .contains("finished"),
        "timed-out child kept running after kill: {after_wait_marker:?}"
    );

    if let Some(root) = fsv_root {
        write_timeout_readback(
            &root,
            &marker,
            before_marker.as_deref(),
            immediate_marker.as_deref(),
            after_wait_marker.as_deref(),
            elapsed,
            &error,
        );
    } else {
        cleanup(dir);
    }
}

fn assert_dense_dim(vector: SlotVector, expected: u32) {
    match vector {
        SlotVector::Dense { dim, data } => {
            assert_eq!(dim, expected);
            assert_eq!(data.len(), expected as usize);
        }
        other => panic!("expected dense vector, got {other:?}"),
    }
}

fn write_timeout_readback(
    root: &Path,
    marker: &Path,
    before_marker: Option<&str>,
    immediate_marker: Option<&str>,
    after_wait_marker: Option<&str>,
    elapsed: Duration,
    error: &CalyxError,
) {
    fs::create_dir_all(root).unwrap();
    let readback = json!({
        "marker": marker,
        "before_marker": before_marker,
        "immediate_marker": immediate_marker,
        "after_wait_marker": after_wait_marker,
        "elapsed_ms": elapsed.as_millis(),
        "error_code": error.code,
        "error_message": error.message,
    });
    fs::write(
        root.join("external-cmd-timeout-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

fn read_marker(marker: &Path) -> Option<String> {
    fs::read_to_string(marker).ok()
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("calyx-registry-{name}-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
