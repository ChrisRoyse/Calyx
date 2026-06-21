use super::*;
use crate::cf::ColumnFamily;
use crate::erase::{EraseHandler, NoopEraseHandler};
use calyx_core::{
    Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use calyx_ledger::{
    ActorId, EntryKind, ErasureScope as LedgerErasureScope, ErasureTombstone,
    decode as decode_ledger, tombstone_from_entry,
};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use ulid::Ulid;

const COLLECTION: &str = "issue504_docs";
const NOW_MS: Ts = 161_000;

fn vault_id() -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes([0xCE; 16]))
}

fn context() -> VaultContext {
    VaultContext::new(
        vault_id(),
        b"retention-test-master-key-material",
        crate::vault::QuotaConfig::default(),
        "tank/calyx",
    )
    .unwrap()
}

fn durable_vault(name: &str) -> (PathBuf, AsterVault) {
    let dir = std::env::temp_dir().join(format!("calyx-retention-{name}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    (dir, vault)
}

fn policy(ttl_secs: u64) -> RetentionPolicy {
    RetentionPolicy {
        collection: COLLECTION.to_string(),
        ttl_secs,
        rollup_after_secs: None,
    }
}

fn store(ttl_secs: u64) -> RetentionStore {
    let mut store = RetentionStore::new();
    store.add_policy(policy(ttl_secs));
    store
}

fn cx<C>(vault: &AsterVault<C>, seed: &'static [u8], ingested_at: Option<&str>) -> Constellation
where
    C: Clock,
{
    let hash = *blake3::hash(seed).as_bytes();
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 1,
            data: vec![seed[0] as f32],
        },
    );
    let mut metadata = BTreeMap::new();
    metadata.insert(METADATA_COLLECTION.to_string(), COLLECTION.to_string());
    if let Some(value) = ingested_at {
        metadata.insert(METADATA_INGESTED_AT.to_string(), value.to_string());
    }
    Constellation {
        cx_id: vault.cx_id_for_input(seed, 1),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash,
            pointer: Some(format!(
                "synthetic://issue504/{:02x}{:02x}",
                hash[0], hash[1]
            )),
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

#[test]
fn is_expired_obeys_boundary_and_clock_skew() {
    let policy = policy(60);
    assert!(is_expired(100_000, &policy, 161_000));
    assert!(!is_expired(100_000, &policy, 160_000));
    assert!(!is_expired(100_000, &policy, 159_000));
    assert!(!is_expired(200_000, &policy, 161_000));
    assert!(!is_expired(
        1,
        &RetentionPolicy {
            ttl_secs: 0,
            ..policy
        },
        u64::MAX
    ));
}

proptest! {
    #[test]
    fn is_expired_matches_saturating_formula(
        ingested_at in any::<u64>(),
        ttl_secs in 0_u64..1_000_000,
        now in any::<u64>(),
    ) {
        let policy = RetentionPolicy {
            collection: COLLECTION.to_string(),
            ttl_secs,
            rollup_after_secs: None,
        };
        let expected = ttl_secs != 0
            && now.saturating_sub(ingested_at) > ttl_secs.saturating_mul(MILLIS_PER_SEC);
        prop_assert_eq!(is_expired(ingested_at, &policy, now), expected);
    }
}

#[test]
fn apply_retention_erases_expired_and_retains_live_cx() {
    let (_dir, vault) = durable_vault("mixed");
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let expired_a = cx(&vault, b"expired-a", Some("100000"));
    let expired_b = cx(&vault, b"expired-b", Some("90000"));
    let live = cx(&vault, b"live-c", Some("150000"));
    let expired_a_id = expired_a.cx_id;
    let expired_b_id = expired_b.cx_id;
    let live_id = live.cx_id;
    vault.put(expired_a).unwrap();
    vault.put(expired_b).unwrap();
    vault.put(live).unwrap();

    let before = scan_expired_cxs(&vault, &ctx, &store(60), NOW_MS).unwrap();
    let results = apply_retention(&vault, &mut ctx, &store(60), &registry, NOW_MS).unwrap();

    assert_eq!(before.len(), 2);
    assert_eq!(results.len(), 2);
    assert_eq!(ledger_tombstone_count(&vault), 2);
    assert!(vault.get(live_id, vault.snapshot()).is_ok());
    assert!(vault.get(expired_a_id, vault.snapshot()).is_err());
    assert!(vault.get(expired_b_id, vault.snapshot()).is_err());
}

#[test]
fn no_policy_and_zero_ttl_retain_rows() {
    let (_dir, vault) = durable_vault("retain");
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let record = cx(&vault, b"retained", Some("1"));
    let id = record.cx_id;
    vault.put(record).unwrap();

    assert!(
        apply_retention(&vault, &mut ctx, &RetentionStore::new(), &registry, NOW_MS)
            .unwrap()
            .is_empty()
    );
    assert!(vault.get(id, vault.snapshot()).is_ok());
    assert!(
        apply_retention(&vault, &mut ctx, &store(0), &registry, NOW_MS)
            .unwrap()
            .is_empty()
    );
    assert!(vault.get(id, vault.snapshot()).is_ok());
}

#[test]
fn malformed_or_missing_timestamp_fails_closed() {
    let (_dir, vault) = durable_vault("bad-metadata");
    let ctx = context();
    vault
        .put(cx(&vault, b"bad-ts", Some("not-a-number")))
        .unwrap();

    let error = scan_expired_cxs(&vault, &ctx, &store(60), NOW_MS).unwrap_err();

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn all_expired_rows_are_erased_per_cx() {
    let (_dir, vault) = durable_vault("all-expired");
    let mut ctx = context();
    let registry = EraseRegistry::new();
    vault.put(cx(&vault, b"all-a", Some("1"))).unwrap();
    vault.put(cx(&vault, b"all-b", Some("2"))).unwrap();

    let results = vault
        .apply_retention(&mut ctx, &store(60), &registry, NOW_MS)
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(ledger_tombstone_count(&vault), 2);
    assert_eq!(
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn erase_failures_are_accumulated_after_full_scan() {
    let (_dir, vault) = durable_vault("failures");
    let mut ctx = context();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = EraseRegistry::new();
    registry.add_handler(CountingFailHandler {
        calls: calls.clone(),
    });
    let first = cx(&vault, b"fail-a", Some("1"));
    let second = cx(&vault, b"fail-b", Some("2"));
    let first_id = first.cx_id;
    let second_id = second.cx_id;
    vault.put(first).unwrap();
    vault.put(second).unwrap();

    let error = apply_retention(&vault, &mut ctx, &store(60), &registry, NOW_MS).unwrap_err();

    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(vault.get(first_id, vault.snapshot()).is_ok());
    assert!(vault.get(second_id, vault.snapshot()).is_ok());
}

#[test]
fn already_tombstoned_cx_is_not_fatal() {
    let (_dir, vault) = durable_vault("already");
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let record = cx(&vault, b"already-a", Some("1"));
    let id = record.cx_id;
    vault.put(record).unwrap();
    append_preexisting_tombstone(&vault, id);

    let results = apply_retention(&vault, &mut ctx, &store(60), &registry, NOW_MS).unwrap();

    assert!(results.is_empty());
    assert_eq!(ledger_tombstone_count(&vault), 1);
    assert!(vault.get(id, vault.snapshot()).is_ok());
}

struct CountingFailHandler {
    calls: Arc<AtomicUsize>,
}

impl EraseHandler for CountingFailHandler {
    fn erase(&self, _scope: &EraseScope, _vault_id: VaultId) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CalyxError::backpressure("retention failpoint"))
    }
}

fn append_preexisting_tombstone<C>(vault: &AsterVault<C>, cx_id: CxId)
where
    C: Clock,
{
    let tombstone = ErasureTombstone {
        seq: vault.next_ledger_seq_locked().unwrap(),
        vault_id: vault.vault_id(),
        scope: LedgerErasureScope::Cx(cx_id),
        actor: ActorId::Service("calyx-retention-test".to_string()),
        erased_at: NOW_MS,
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

fn ledger_tombstone_count<C>(vault: &AsterVault<C>) -> usize
where
    C: Clock,
{
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .filter(|(_, bytes)| {
            let entry = decode_ledger(bytes).unwrap();
            tombstone_from_entry(&entry).unwrap().is_some()
        })
        .count()
}

#[test]
fn issue504_retention_fsv_fixture() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue504-fsv"));
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("issue504-retention-vault");
    if vault_dir.exists() {
        fs::remove_dir_all(&vault_dir).unwrap();
    }
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let expired_a = cx(&vault, b"FSV_ISSUE504_EXPIRED_A", Some("100000"));
    let expired_b = cx(&vault, b"FSV_ISSUE504_EXPIRED_B", Some("90000"));
    let live = cx(&vault, b"FSV_ISSUE504_RETAINED", Some("150000"));
    let expired_a_id = expired_a.cx_id;
    let expired_b_id = expired_b.cx_id;
    let live_id = live.cx_id;
    vault.put(expired_a).unwrap();
    vault.put(expired_b).unwrap();
    vault.put(live).unwrap();

    println!("FSV_ISSUE504_VAULT={}", vault_dir.display());
    println!("FSV_ISSUE504_EXPIRED_A={expired_a_id}");
    println!("FSV_ISSUE504_EXPIRED_B={expired_b_id}");
    println!("FSV_ISSUE504_LIVE={live_id}");
    println!(
        "FSV_ISSUE504_BASE_BEFORE={}",
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_EXPIRED_BEFORE={}",
        scan_expired_cxs(&vault, &ctx, &store(60), NOW_MS)
            .unwrap()
            .len()
    );

    let results = apply_retention(&vault, &mut ctx, &store(60), &registry, NOW_MS).unwrap();

    println!("FSV_ISSUE504_ERASED={}", results.len());
    println!(
        "FSV_ISSUE504_BASE_AFTER={}",
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_TOMBSTONES_AFTER={}",
        ledger_tombstone_count(&vault)
    );
    println!(
        "FSV_ISSUE504_LIVE_PRESENT={}",
        vault.get(live_id, vault.snapshot()).is_ok()
    );
    println!(
        "FSV_ISSUE504_EXPIRED_A_PRESENT={}",
        vault.get(expired_a_id, vault.snapshot()).is_ok()
    );

    retention_edge_fsv(&root);
}

fn retention_edge_fsv(root: &std::path::Path) {
    let registry = EraseRegistry::new();
    let edge_dir = root.join("issue504-no-policy");
    if edge_dir.exists() {
        fs::remove_dir_all(&edge_dir).unwrap();
    }
    let edge = AsterVault::new_durable(
        &edge_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut edge_ctx = context();
    let edge_row = cx(&edge, b"FSV_ISSUE504_NO_POLICY", Some("1"));
    let edge_id = edge_row.cx_id;
    edge.put(edge_row).unwrap();
    println!(
        "FSV_ISSUE504_NO_POLICY_ERASED={}",
        apply_retention(
            &edge,
            &mut edge_ctx,
            &RetentionStore::new(),
            &registry,
            NOW_MS
        )
        .unwrap()
        .len()
    );
    println!(
        "FSV_ISSUE504_NO_POLICY_PRESENT={}",
        edge.get(edge_id, edge.snapshot()).is_ok()
    );
    println!(
        "FSV_ISSUE504_ZERO_TTL_ERASED={}",
        apply_retention(&edge, &mut edge_ctx, &store(0), &registry, NOW_MS)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_ZERO_TTL_PRESENT={}",
        edge.get(edge_id, edge.snapshot()).is_ok()
    );

    let bad_dir = root.join("issue504-invalid-ts");
    if bad_dir.exists() {
        fs::remove_dir_all(&bad_dir).unwrap();
    }
    let bad = AsterVault::new_durable(
        &bad_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let bad_ctx = context();
    let bad_row = cx(&bad, b"FSV_ISSUE504_BAD_TS", Some("bad-ts"));
    let bad_id = bad_row.cx_id;
    bad.put(bad_row).unwrap();
    let bad_error = scan_expired_cxs(&bad, &bad_ctx, &store(60), NOW_MS).unwrap_err();
    println!("FSV_ISSUE504_INVALID_TS_ERROR={}", bad_error.code);
    println!(
        "FSV_ISSUE504_INVALID_TS_PRESENT={}",
        bad.get(bad_id, bad.snapshot()).is_ok()
    );

    let all_dir = root.join("issue504-all-expired");
    if all_dir.exists() {
        fs::remove_dir_all(&all_dir).unwrap();
    }
    let all = AsterVault::new_durable(
        &all_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut all_ctx = context();
    all.put(cx(&all, b"FSV_ISSUE504_ALL_A", Some("1"))).unwrap();
    all.put(cx(&all, b"FSV_ISSUE504_ALL_B", Some("2"))).unwrap();
    println!(
        "FSV_ISSUE504_ALL_EXPIRED_BEFORE={}",
        scan_expired_cxs(&all, &all_ctx, &store(60), NOW_MS)
            .unwrap()
            .len()
    );
    let all_results = apply_retention(&all, &mut all_ctx, &store(60), &registry, NOW_MS).unwrap();
    println!("FSV_ISSUE504_ALL_EXPIRED_ERASED={}", all_results.len());
    println!(
        "FSV_ISSUE504_ALL_EXPIRED_BASE_AFTER={}",
        all.scan_cf_at(all.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );

    let mut noop_registry = EraseRegistry::new();
    noop_registry.add_handler(NoopEraseHandler);
    println!("FSV_ISSUE504_NOOP_HANDLER_REGISTERED=true");
}
