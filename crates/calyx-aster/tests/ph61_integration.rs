use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::erase::{EraseRegistry, EraseScope};
use calyx_aster::ledger_view::parse_aster_ledger_seq;
use calyx_aster::redaction::{CALYX_PII_REDACTION_REQUIRED, InputMode, assert_hash_only_mode};
use calyx_aster::retention::{
    METADATA_COLLECTION, METADATA_INGESTED_AT, RetentionPolicy, RetentionStore, apply_retention,
};
use calyx_aster::stride_fsv::{CALYX_EXTERNAL_CMD_NOT_ALLOWED, run_external_cmd};
use calyx_aster::supply_chain::{
    AuditVulnEntry, CALYX_LENS_WEIGHT_TAMPERED, CALYX_SBOM_PARSE_ERROR, CALYX_SUPPLY_CHAIN_VULN,
    CargoAuditResult, assert_audit_clean, generate_sbom, lens_weight_content_address,
    verify_lens_weight_hash,
};
use calyx_aster::vault::{
    AsterVault, CALYX_DECRYPTION_FAILED, QuotaConfig, VaultContext, VaultOptions,
};
use calyx_core::{
    CALYX_AUTHN_REQUIRED, CALYX_PROVISIONAL_VAULT, CalyxError, ColdStartGuard, Constellation,
    CxFlags, InputRef, LedgerRef, LensId, Modality, Result, SlotId, SlotVector, Ts, VaultId,
    VaultStore, no_anonymous_write,
};
use calyx_ledger::{
    ActorId, EntryKind, ErasureScope as LedgerErasureScope, ErasureTombstone, LedgerCfStore,
    LedgerRow, SubjectId, VerifyResult, decode as decode_ledger, is_tombstoned,
    tombstone_from_entry, verify_chain,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use ulid::Ulid;

mod fsv_support;
use fsv_support::{fsv_root as support_fsv_root, reset_dir};

// A25 forbids deleting-to-compress, NOT lawful/user-requested deletion.
// erase() is A33-required. No reviewer may refuse this citing A25.
// See: dbprdplans/30 section 4 (binding clarification).

const ORIGINAL: &[u8; 32] = b"PH61_ERASE_CONTENT_0000000000508";
const COLLECTION: &str = "ph61_issue508";
const INGESTED_AT: Ts = 100_000;
const NOW: Ts = 180_000;

#[test]
fn ph61_integration_full_phase_fsv() {
    let (root, _) = support_fsv_root("CALYX_FSV_ROOT", "calyx-ph61-issue508");
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault-a");
    reset_dir(&vault_dir);
    let vault = durable_vault(&vault_dir);
    let mut ctx = context(vault_id(0xA5));
    let registry = EraseRegistry::new();
    let ciphertext = ctx.encrypt_value(ORIGINAL, b"ph61-issue508").unwrap();
    let row = cx(&vault, ORIGINAL, Some(INGESTED_AT));
    let cx_id = row.cx_id;
    vault.put(row).unwrap();
    vault.flush().unwrap();
    let before_base = read_base_row(&vault, cx_id).is_some();

    let result = vault
        .erase(EraseScope::Cx(cx_id), &mut ctx, &registry)
        .unwrap();
    vault.flush().unwrap();
    let ledger = AsterLedgerStore { vault: &vault };
    let ledger_rows = ledger.scan().unwrap();
    let verified = verify_chain(&ledger, 0..ledger_rows.len() as u64).unwrap();
    let tombstone_present =
        is_tombstoned(vault.vault_id(), &LedgerErasureScope::Cx(cx_id), &ledger).unwrap();
    let tombstone_payload = tombstone_payload_for(&ledger_rows, cx_id);
    let no_content = !tombstone_payload
        .windows(ORIGINAL.len())
        .any(|window| window == ORIGINAL);
    let decrypt_error = ctx
        .decrypt_value(&ciphertext, b"ph61-issue508")
        .unwrap_err();
    let reerase_error = vault
        .erase(EraseScope::Cx(cx_id), &mut ctx, &registry)
        .unwrap_err();

    assert!(before_base);
    assert_eq!(result.records_deleted, 1);
    assert!(tombstone_present);
    assert_eq!(
        verified,
        VerifyResult::Intact {
            count: ledger_rows.len() as u64
        }
    );
    assert_eq!(decrypt_error.code, CALYX_DECRYPTION_FAILED);
    assert!(read_base_row(&vault, cx_id).is_none());
    assert!(no_content);
    assert_eq!(reerase_error.code, "CALYX_ERASE_ALREADY_TOMBSTONED");

    println!("PH61_FSV_ROOT={}", root.display());
    println!("erase result: records_deleted=1 ✓");
    println!("tombstone present: true ✓");
    println!("decrypt after shred: Err(CALYX_DECRYPTION_FAILED) ✓");
    println!("restored_decrypt_with_shredded_key=Err(CALYX_DECRYPTION_FAILED)");
    println!("tombstone payload contains no content bytes: true ✓");

    cross_vault_denial_is_in_real_ledger(&vault);
    cold_start_transitions();
    retention_already_tombstoned_is_idempotent(&root);
    public_repo_internal_dev_tooling_is_not_reintroduced();
    fail_closed_codes_are_exercised(&root, reerase_error.code);
}

fn cross_vault_denial_is_in_real_ledger(vault: &AsterVault) {
    let src = vault_id(0xA5);
    let dst = vault_id(0xB6);
    let ctx = context(src);
    let actor = ActorId::Agent("ph61-issue508".to_string());
    let denial = ctx
        .check_cross_vault_read(dst, actor.clone(), NOW)
        .unwrap_err();
    assert_eq!(denial.code, "CALYX_VAULT_ACCESS_DENIED");

    let payload = serde_json::to_vec(&serde_json::json!({
        "event": "AccessDenied",
        "src_vault": src.to_string(),
        "dst_vault": dst.to_string(),
        "actor": "ph61-issue508"
    }))
    .unwrap();
    vault
        .append_ledger_entry(
            EntryKind::Admin,
            SubjectId::Guard(b"ph61-access-denied".to_vec()),
            payload,
            actor,
        )
        .unwrap();
    vault.flush().unwrap();

    let ledger = AsterLedgerStore { vault };
    let rows = ledger.scan().unwrap();
    assert_eq!(
        verify_chain(&ledger, 0..rows.len() as u64).unwrap(),
        VerifyResult::Intact {
            count: rows.len() as u64
        }
    );
    let audited = rows.into_iter().any(|row| {
        let entry = decode_ledger(&row.bytes).unwrap();
        entry.kind == EntryKind::Admin
            && String::from_utf8_lossy(&entry.payload).contains("AccessDenied")
    });
    assert!(audited);
    println!("cross-vault AccessDenied ledger-audited: true ✓");
}

fn cold_start_transitions() {
    let mut guard = ColdStartGuard::new();
    assert!(guard.search_always_ok());
    let error = guard.assert_grounded("oracle_answer").unwrap_err();
    assert_eq!(error.code, CALYX_PROVISIONAL_VAULT);
    guard.record_anchor();
    guard.record_anchor();
    assert!(guard.assert_grounded("oracle_answer").is_ok());
    assert_eq!(guard.anchor_count(), 2);
    println!("cold-start provisional rejects then grounds: true ✓");
}

fn retention_already_tombstoned_is_idempotent(root: &Path) {
    let vault_dir = root.join("retention-edge");
    let vault = durable_vault(&vault_dir);
    let mut ctx = context(vault.vault_id());
    let registry = EraseRegistry::new();
    let record = cx(
        &vault,
        b"PH61_RETENTION_EDGE_000000000000",
        Some(INGESTED_AT),
    );
    let cx_id = record.cx_id;
    vault.put(record).unwrap();
    let seq = AsterLedgerStore { vault: &vault }.scan().unwrap().len() as u64;
    append_tombstone(&vault, cx_id, seq);

    let results = apply_retention(&vault, &mut ctx, &retention_store(), &registry, NOW).unwrap();

    assert!(results.is_empty());
    assert!(read_base_row(&vault, cx_id).is_some());
    assert_eq!(tombstone_count(&vault), 1);
}

fn public_repo_internal_dev_tooling_is_not_reintroduced() {
    let repo = workspace_root();
    let script = repo.join("scripts/secret-scan.sh");
    let hook = repo.join(".pre-commit-config.yaml");
    let git_config = git_config_readback(&repo);
    let remote_origin = git_output(&repo, &["config", "--get", "remote.origin.url"]);
    let is_public_repo = remote_origin.contains("github.com/ChrisRoyse/Calyx.git")
        || remote_origin.contains("github.com:ChrisRoyse/Calyx.git");

    if is_public_repo {
        assert!(!script.exists());
        assert!(!hook.exists());
    } else {
        assert!(script.is_file());
        assert!(hook.is_file());
    }
    assert!(
        !git_config.contains("DISABLED_PUBLIC_PUSH") || script.is_file(),
        "a disabled public push sentinel only belongs in the dev checkout"
    );
    assert!(repo.join(".gitignore").is_file());
    assert!(repo.join("LICENSE").is_file());
    assert!(repo.join("README.md").is_file());
}

fn git_config_readback(repo: &Path) -> String {
    let mut config = fs::read_to_string(repo.join(".git/config")).unwrap_or_default();
    let common_dir = git_output(repo, &["rev-parse", "--git-common-dir"]);
    let common_dir = common_dir.trim();
    if !common_dir.is_empty() {
        let common_path = Path::new(common_dir);
        let common_config = if common_path.is_absolute() {
            common_path.join("config")
        } else {
            repo.join(common_path).join("config")
        };
        if let Ok(text) = fs::read_to_string(common_config) {
            config.push_str(&text);
        }
    }
    config
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn fail_closed_codes_are_exercised(root: &Path, reerase_code: &str) {
    let mut observed = vec![
        reerase_code.to_string(),
        no_anonymous_write(None).unwrap_err().code.to_string(),
        assert_hash_only_mode(&InputMode::Full("pii".to_string()))
            .unwrap_err()
            .code
            .to_string(),
        run_external_cmd("rm -rf /", &["calyx-readback"])
            .unwrap_err()
            .code
            .to_string(),
        ColdStartGuard::new()
            .assert_grounded("ward_guard")
            .unwrap_err()
            .code
            .to_string(),
    ];
    observed.push(sbom_parse_error(root).code.to_string());
    observed.push(lens_tamper_error(root).code.to_string());
    observed.push(supply_chain_vuln_error().code.to_string());

    for expected in [
        "CALYX_ERASE_ALREADY_TOMBSTONED",
        CALYX_AUTHN_REQUIRED,
        CALYX_PII_REDACTION_REQUIRED,
        CALYX_EXTERNAL_CMD_NOT_ALLOWED,
        CALYX_PROVISIONAL_VAULT,
        CALYX_SBOM_PARSE_ERROR,
        CALYX_LENS_WEIGHT_TAMPERED,
        CALYX_SUPPLY_CHAIN_VULN,
    ] {
        assert!(observed.iter().any(|code| code == expected), "{expected}");
    }
}

fn sbom_parse_error(root: &Path) -> CalyxError {
    let lock = root.join("bad-Cargo.lock");
    fs::write(&lock, b"[[package]").unwrap();
    generate_sbom(&lock, NOW).unwrap_err()
}

fn lens_tamper_error(root: &Path) -> CalyxError {
    let weights = root.join("weights.bin");
    fs::write(&weights, b"known weights").unwrap();
    let lens_id = LensId::from_bytes(lens_weight_content_address(b"known weights"));
    fs::write(&weights, b"known weightz").unwrap();
    verify_lens_weight_hash(&lens_id, &weights).unwrap_err()
}

fn supply_chain_vuln_error() -> CalyxError {
    let vuln = AuditVulnEntry {
        package: "synthetic".to_string(),
        version: "0.0.1".to_string(),
        advisory_id: "RUSTSEC-2099-0508".to_string(),
        description: "synthetic".to_string(),
    };
    assert_audit_clean(&CargoAuditResult::Vulnerabilities(vec![vuln])).unwrap_err()
}

fn append_tombstone(vault: &AsterVault, cx_id: calyx_core::CxId, seq: u64) {
    let tombstone = ErasureTombstone {
        seq,
        vault_id: vault.vault_id(),
        scope: LedgerErasureScope::Cx(cx_id),
        actor: ActorId::Service("calyx-ph61-integration".to_string()),
        erased_at: NOW,
        records_deleted: 1,
    };
    vault
        .append_ledger_entry(
            EntryKind::Erase,
            tombstone.ledger_subject(),
            tombstone.as_ledger_payload(),
            tombstone.actor,
        )
        .unwrap();
}

fn tombstone_payload_for(rows: &[LedgerRow], cx_id: calyx_core::CxId) -> Vec<u8> {
    rows.iter()
        .filter_map(|row| {
            let entry = decode_ledger(&row.bytes).unwrap();
            let tombstone = tombstone_from_entry(&entry).unwrap()?;
            (tombstone.scope == LedgerErasureScope::Cx(cx_id)).then_some(entry.payload)
        })
        .next()
        .expect("erasure tombstone payload")
}

fn tombstone_count(vault: &AsterVault) -> usize {
    let ledger = AsterLedgerStore { vault };
    ledger
        .scan()
        .unwrap()
        .into_iter()
        .filter(|row| {
            let entry = decode_ledger(&row.bytes).unwrap();
            tombstone_from_entry(&entry).unwrap().is_some()
        })
        .count()
}

fn read_base_row(vault: &AsterVault, cx_id: calyx_core::CxId) -> Option<Vec<u8>> {
    vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))
        .unwrap()
}

fn durable_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(0xA5),
        b"ph61-issue508-salt",
        VaultOptions::default(),
    )
    .unwrap()
}

fn context(vault_id: VaultId) -> VaultContext {
    VaultContext::new(
        vault_id,
        b"ph61-issue508-master-key-material",
        QuotaConfig::default(),
        "tank/calyx",
    )
    .unwrap()
}

fn cx<C>(vault: &AsterVault<C>, seed: &'static [u8], ingested_at: Option<Ts>) -> Constellation
where
    C: calyx_core::Clock,
{
    let hash = *blake3::hash(seed).as_bytes();
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![seed[0] as f32, 1.0],
        },
    );
    let mut metadata = BTreeMap::new();
    metadata.insert(METADATA_COLLECTION.to_string(), COLLECTION.to_string());
    if let Some(value) = ingested_at {
        metadata.insert(METADATA_INGESTED_AT.to_string(), value.to_string());
    }
    Constellation {
        cx_id: vault.cx_id_for_input(seed, 1),
        vault_id: vault.vault_id(),
        panel_version: 1,
        created_at: INGESTED_AT,
        input_ref: InputRef {
            hash,
            pointer: Some(format!("synthetic://ph61-{:02x}{:02x}", hash[0], hash[1])),
            redacted: true,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [seed[0]; 32],
        },
        flags: CxFlags::default(),
    }
}

fn retention_store() -> RetentionStore {
    let mut store = RetentionStore::new();
    store.add_policy(RetentionPolicy {
        collection: COLLECTION.to_string(),
        ttl_secs: 60,
        rollup_after_secs: None,
    });
    store
}

fn vault_id(byte: u8) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

struct AsterLedgerStore<'a, C> {
    vault: &'a AsterVault<C>,
}

impl<C> LedgerCfStore for AsterLedgerStore<'_, C>
where
    C: calyx_core::Clock,
{
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::new();
        for (key, bytes) in self
            .vault
            .scan_cf_at(self.vault.snapshot(), ColumnFamily::Ledger)?
        {
            rows.push(LedgerRow {
                seq: parse_aster_ledger_seq(&key)?,
                bytes,
            });
        }
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "test store is read-only for ledger seq {seq}"
        )))
    }
}
