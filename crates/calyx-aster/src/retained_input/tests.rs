use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{InputRef, Modality};

use super::*;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn retains_canonical_text_and_reads_exact_bytes() {
    let root = test_dir("canonical");
    let text = "issue1423 retained source";
    let expected_hash = *blake3::hash(text.as_bytes()).as_bytes();

    let input = retain_text_input(&root, text).expect("retain text");
    let pointer = input.pointer.as_deref().expect("retained pointer");

    assert_eq!(pointer, canonical_text_pointer(&expected_hash));
    assert_eq!(input.bytes, text.as_bytes());
    assert_eq!(
        fs::read(retained_pointer_path(&root, pointer).expect("pointer path")).unwrap(),
        text.as_bytes()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_blob_with_different_bytes_fails_closed() {
    let root = test_dir("collision");
    let input = retain_text_input(&root, "authoritative").expect("first retain");
    let path = retained_pointer_path(&root, input.pointer.as_deref().unwrap()).unwrap();
    fs::write(&path, b"tampered").unwrap();

    let error = retain_text_input(&root, "authoritative").expect_err("reject collision");

    assert_eq!(error.code, CALYX_INPUT_BLOB_HASH_MISMATCH);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_ref_hash_mismatch_is_rejected() {
    let root = test_dir("hash-mismatch");
    let input = retain_text_input(&root, "verified bytes").expect("retain");
    let input_ref = InputRef {
        hash: [7; 32],
        pointer: input.pointer,
        redacted: false,
    };

    let error = input_from_ref(&root, Modality::Text, &input_ref).expect_err("reject hash");

    assert_eq!(error.code, CALYX_INPUT_BLOB_HASH_MISMATCH);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pointer_filename_hash_must_match_retained_bytes() {
    let root = test_dir("pointer-hash-mismatch");
    let text = "bytes stored under the wrong canonical name";
    let expected_hash = *blake3::hash(text.as_bytes()).as_bytes();
    let wrong_hash = *blake3::hash(b"different source bytes").as_bytes();
    let wrong_pointer = canonical_text_pointer(&wrong_hash);
    let wrong_path = retained_pointer_path(&root, &wrong_pointer).unwrap();
    fs::create_dir_all(wrong_path.parent().unwrap()).unwrap();
    fs::write(&wrong_path, text.as_bytes()).unwrap();
    let input_ref = InputRef {
        hash: expected_hash,
        pointer: Some(wrong_pointer),
        redacted: false,
    };

    let error = input_from_ref(&root, Modality::Text, &input_ref)
        .expect_err("misnamed retained blob must fail");

    assert_eq!(error.code, CALYX_INPUT_BLOB_HASH_MISMATCH);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_length_delimited_hash_remains_readable() {
    let root = test_dir("legacy-hash");
    let text = "legacy mcp retained bytes";
    let legacy_hash = full_content_hash([text.as_bytes()]);
    let pointer = canonical_text_pointer(&legacy_hash);
    let path = retained_pointer_path(&root, &pointer).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text.as_bytes()).unwrap();
    let input_ref = InputRef {
        hash: legacy_hash,
        pointer: Some(pointer),
        redacted: false,
    };

    let replay = input_from_ref(&root, Modality::Text, &input_ref).expect("legacy read");

    assert_eq!(replay.bytes, text.as_bytes());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pointer_escape_is_rejected() {
    let error = retained_pointer_path(
        Path::new("/vault"),
        "calyx-vault://inputs/../../outside.bin",
    )
    .expect_err("reject escape");

    assert_eq!(error.code, CALYX_INPUT_POINTER_INVALID);
}

#[test]
fn noncanonical_blob_names_are_rejected() {
    let hash = "a".repeat(64);
    for pointer in [
        format!("{VAULT_INPUT_POINTER_PREFIX}short.bin"),
        format!("{VAULT_INPUT_POINTER_PREFIX}{}.txt", hash),
        format!("{VAULT_INPUT_POINTER_PREFIX}{}.bin", hash.to_uppercase()),
        format!("{VAULT_INPUT_POINTER_PREFIX}nested/{hash}.bin"),
    ] {
        let error = retained_pointer_path(Path::new("/vault"), &pointer)
            .expect_err("reject noncanonical pointer");
        assert_eq!(error.code, CALYX_INPUT_POINTER_INVALID);
    }
}

#[cfg(unix)]
#[test]
fn symlinked_input_directory_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = test_dir("symlink-root");
    let outside = test_dir("symlink-outside");
    let bytes = b"outside retained bytes";
    let hash = *blake3::hash(bytes).as_bytes();
    let pointer = canonical_text_pointer(&hash);
    fs::write(outside.join(format!("{}.bin", hex(&hash))), bytes).unwrap();
    symlink(&outside, root.join(INPUT_DIR)).unwrap();
    let input_ref = InputRef {
        hash,
        pointer: Some(pointer),
        redacted: false,
    };

    let error = input_from_ref(&root, Modality::Text, &input_ref)
        .expect_err("reject symlinked input directory");

    assert_eq!(error.code, CALYX_INPUT_POINTER_INVALID);
    fs::remove_file(root.join(INPUT_DIR)).unwrap();
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "calyx-retained-input-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
