use calyx_aster::vault::{CALYX_DECRYPTION_FAILED, QuotaConfig, VaultContext};
use calyx_core::VaultId;
mod fsv_support;
use fsv_support::{fsv_root_env_subdir, reset_dir};
use serde_json::json;
use std::fs;
use ulid::Ulid;

const PLAINTEXT: &[u8] = b"ISSUE815_NONCE_SENTINEL_VALUE";
const AAD: &[u8] = b"issue815/aad";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[test]
fn issue815_nonce_envelope_fsv() {
    let (root, preserve) = fsv_root_env_subdir(
        "CALYX_FSV_ROOT",
        "issue815-nonce-envelope",
        "calyx-issue815",
    );
    reset_dir(&root);

    let ctx = VaultContext::new(
        VaultId::from_ulid(Ulid::from_bytes([0x81; 16])),
        b"issue815-master-key-material",
        QuotaConfig::default(),
        "tank/calyx",
    )
    .unwrap();

    let first = ctx.encrypt_value(PLAINTEXT, AAD).unwrap();
    let second = ctx.encrypt_value(PLAINTEXT, AAD).unwrap();
    let empty = ctx.encrypt_value(b"", AAD).unwrap();
    let first_path = root.join("first.sealed");
    let second_path = root.join("second.sealed");
    let empty_path = root.join("empty.sealed");
    fs::write(&first_path, &first).unwrap();
    fs::write(&second_path, &second).unwrap();
    fs::write(&empty_path, &empty).unwrap();

    let first_read = fs::read(&first_path).unwrap();
    let second_read = fs::read(&second_path).unwrap();
    let empty_read = fs::read(&empty_path).unwrap();
    assert_eq!(first_read.len(), NONCE_LEN + PLAINTEXT.len() + TAG_LEN);
    assert_eq!(second_read.len(), first_read.len());
    assert_eq!(empty_read.len(), NONCE_LEN + TAG_LEN);
    assert_ne!(&first_read[..NONCE_LEN], &second_read[..NONCE_LEN]);
    assert!(!contains(&first_read, PLAINTEXT));
    assert!(!contains(&second_read, PLAINTEXT));

    let first_plain = ctx.decrypt_value(&first_read, AAD).unwrap();
    let second_plain = ctx.decrypt_value(&second_read, AAD).unwrap();
    let empty_plain = ctx.decrypt_value(&empty_read, AAD).unwrap();
    let wrong_aad = ctx.decrypt_value(&first_read, b"wrong-aad").unwrap_err();
    let truncated = ctx.decrypt_value(&first_read[..27], AAD).unwrap_err();
    assert_eq!(first_plain, PLAINTEXT);
    assert_eq!(second_plain, PLAINTEXT);
    assert!(empty_plain.is_empty());
    assert_eq!(wrong_aad.code, CALYX_DECRYPTION_FAILED);
    assert_eq!(truncated.code, CALYX_DECRYPTION_FAILED);

    let readback = json!({
        "first_len": first_read.len(),
        "second_len": second_read.len(),
        "empty_len": empty_read.len(),
        "nonces_differ": first_read[..NONCE_LEN] != second_read[..NONCE_LEN],
        "first_plaintext_absent": !contains(&first_read, PLAINTEXT),
        "second_plaintext_absent": !contains(&second_read, PLAINTEXT),
        "first_decrypt": String::from_utf8(first_plain).unwrap(),
        "second_decrypt": String::from_utf8(second_plain).unwrap(),
        "empty_decrypt_len": empty_plain.len(),
        "wrong_aad_code": wrong_aad.code,
        "truncated_code": truncated.code,
        "paths": {
            "root": root,
            "first": first_path,
            "second": second_path,
            "empty": empty_path,
        }
    });
    fs::write(
        root.join("issue815_nonce_envelope_readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    if !preserve {
        fs::remove_dir_all(&root).unwrap();
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
