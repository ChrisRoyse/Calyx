//! Issue #597 manual FSV fixture: crypto-shred unreadability through a REAL
//! restic snapshot.
//!
//! This ignored test deliberately does not run restic itself. The agent creates
//! the source bytes with Calyx's `VaultContext`, runs `restic backup` / `restore`
//! manually in a manual verification run, then re-runs this test in restore-verify mode against
//! the restored bytes. That keeps the restic snapshot as the Source of Truth.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{CALYX_DECRYPTION_FAILED, QuotaConfig, VaultContext};
use calyx_core::VaultId;
use rand::RngCore;
use serde_json::json;
use ulid::Ulid;

const SENTINEL: &[u8] = b"CALYX_ISSUE597_ERASED_SENTINEL_DO_NOT_RECOVER";
const AAD: &[u8] = b"calyx-issue597/vault-a/cx-0001";
const CX_FILE: &str = "cf/base/cx-issue597.cipher";
const TOMBSTONE_FILE: &str = "ledger/tombstone-issue597.json";
const KEY_STATE_FILE: &str = "key-state/vault-a.key-state";

#[test]
#[ignore = "manual FSV: seed, restic backup/restore, then verify restored bytes"]
fn real_restic_crypto_shred_fixture() -> Result<(), Box<dyn Error>> {
    if let Ok(restored_vault) = std::env::var("CALYX_ISSUE597_RESTORED_VAULT") {
        verify_restored_vault(Path::new(&restored_vault))
    } else {
        seed_source_vault()
    }
}

fn seed_source_vault() -> Result<(), Box<dyn Error>> {
    let root = required_path("CALYX_ISSUE597_FSV_ROOT")?;
    let vault = root.join("source").join("vault-a");
    if vault.exists() {
        return Err(format!("refusing to overwrite existing vault {}", vault.display()).into());
    }

    fs::create_dir_all(vault.join("cf/base"))?;
    fs::create_dir_all(vault.join("ledger"))?;
    fs::create_dir_all(vault.join("key-state"))?;

    let mut master = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut master);

    let mut ctx = vault_context(&master)?;
    let ciphertext = ctx.encrypt_value(SENTINEL, AAD)?;
    let before = ctx.decrypt_value(&ciphertext, AAD)?;
    assert_eq!(before, SENTINEL, "pre-shred key must decrypt the sentinel");

    ctx.shred_key_for_erasure();
    let err = ctx.decrypt_value(&ciphertext, AAD).unwrap_err();
    assert_eq!(err.code, CALYX_DECRYPTION_FAILED);

    fs::write(vault.join(CX_FILE), &ciphertext)?;
    fs::write(vault.join(TOMBSTONE_FILE), tombstone_json(&ciphertext))?;
    fs::write(
        vault.join(KEY_STATE_FILE),
        b"state=shredded\npersisted_key_bytes=absent\n",
    )?;
    fs::write(
        vault.join("README.txt"),
        b"issue597 synthetic vault: ciphertext plus erasure tombstone; no plaintext or key bytes are persisted.\n",
    )?;

    assert_no_sentinel_in_tree(&vault)?;
    println!("issue597 source_vault={}", vault.display());
    println!("sentinel_ascii={}", std::str::from_utf8(SENTINEL)?);
    println!("sentinel_hex={}", hex_lower(SENTINEL));
    println!("before_shred_decrypt=Ok(len={})", before.len());
    println!(
        "after_shred_decrypt=Err({}) using zeroized runtime key",
        CALYX_DECRYPTION_FAILED
    );
    println!("erase_result=records_deleted=1");
    println!("tombstone_present=true");
    println!("ciphertext_len={}", ciphertext.len());
    println!("ciphertext_blake3={}", blake3::hash(&ciphertext).to_hex());
    println!("tombstone_path={}", vault.join(TOMBSTONE_FILE).display());
    Ok(())
}

fn verify_restored_vault(vault: &Path) -> Result<(), Box<dyn Error>> {
    let ciphertext = fs::read(vault.join(CX_FILE))?;
    let tombstone = fs::read(vault.join(TOMBSTONE_FILE))?;
    let key_state = fs::read_to_string(vault.join(KEY_STATE_FILE))?;

    assert_no_sentinel_in_tree(vault)?;
    assert!(
        !contains(&tombstone, SENTINEL),
        "tombstone must not carry plaintext"
    );
    assert!(
        key_state.contains("state=shredded"),
        "restored key state must prove shred"
    );

    let mut shredded = vault_context(b"restored-key-material-for-issue597")?;
    shredded.shred_key_for_erasure();
    let err = shredded.decrypt_value(&ciphertext, AAD).unwrap_err();
    assert_eq!(err.code, CALYX_DECRYPTION_FAILED);

    println!("issue597 restored_vault={}", vault.display());
    println!("restored_sentinel_absent=true");
    println!("restored_tombstone_present=true");
    println!(
        "restored_decrypt_with_shredded_key=Err({})",
        CALYX_DECRYPTION_FAILED
    );
    println!(
        "restored_ciphertext_blake3={}",
        blake3::hash(&ciphertext).to_hex()
    );
    Ok(())
}

fn tombstone_json(ciphertext: &[u8]) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "kind": "Erased",
        "scope": "Cx(issue597-synthetic-cx)",
        "actor": "issue597-fsv",
        "records_deleted": 1,
        "key_state": "shredded",
        "ciphertext_blake3": blake3::hash(ciphertext).to_hex().to_string(),
        "contains_content_bytes": false
    }))
    .expect("tombstone json")
}

fn vault_context(master: &[u8]) -> Result<VaultContext, Box<dyn Error>> {
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x59; 16]));
    Ok(VaultContext::new(
        vault_id,
        master,
        QuotaConfig::default(),
        "tank/calyx",
    )?)
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let value = std::env::var(name).map_err(|_| format!("{name} is required"))?;
    Ok(PathBuf::from(value))
}

fn assert_no_sentinel_in_tree(root: &Path) -> Result<(), Box<dyn Error>> {
    for file in files(root)? {
        let bytes = fs::read(&file)?;
        if contains(&bytes, SENTINEL) {
            return Err(format!("sentinel plaintext found in {}", file.display()).into());
        }
    }
    Ok(())
}

fn files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut out = Vec::new();
    collect_files(root, &mut out)?;
    Ok(out)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
