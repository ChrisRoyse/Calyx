use super::*;
use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy, Schema,
    SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use calyx_aster::erase::{EraseRegistry, EraseScope};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultStore,
};
use calyx_mcp::decode_jsonrpc_request;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-leapable-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn req(line: &str) -> calyx_mcp::JsonRpcRequest {
    decode_jsonrpc_request(line.as_bytes()).unwrap()
}

fn config(root: PathBuf) -> EngineConfig {
    EngineConfig {
        data_dir: root,
        master_key: vec![0xA5; 32],
        flush_policy: crate::config::FlushPolicy::Always,
    }
}

fn encrypted_options(
    cfg: &EngineConfig,
    vault_id: VaultId,
    dir: &Path,
) -> (VaultOptions, SharedVaultContext) {
    let context = Arc::new(RwLock::new(
        VaultContext::new_for_path(vault_id, &cfg.master_key, QuotaConfig::default(), dir).unwrap(),
    ));
    let options = VaultOptions {
        value_crypto: Some(Arc::clone(&context)),
        ..VaultOptions::default()
    };
    (options, context)
}

fn sample_constellation<C: Clock>(vault: &AsterVault<C>, seed: u8) -> Constellation {
    let input = [seed; 8];
    Constellation {
        cx_id: vault.cx_id_for_input(&input, 1),
        vault_id: vault.vault_id(),
        panel_version: 1,
        created_at: 1_785_500_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 4,
                data: vec![f32::from(seed); 4],
            },
        )]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "calyx-leapable-test".to_string(),
            observed_at: 1_785_500_000,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

#[test]
fn two_vaults_are_multiplexed_by_vault_ref() {
    let root = temp_root("multiplex");
    let mut engine = Engine::new(config(root.clone()));
    let create_a = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.create","params":{"vault_ref":"alpha","ts":1785500000}}"#,
    ));
    let create_b = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"vault.create","params":{"vault_ref":"beta","ts":1785500010}}"#,
    ));
    assert!(create_a.error.is_none());
    assert!(create_b.error.is_none());

    let stat_a = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":3,"method":"vault.stat","params":{"vault_ref":"alpha","ts":1785500020}}"#,
    ));
    let stat_b = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":4,"method":"vault.stat","params":{"vault_ref":"beta","ts":1785500030}}"#,
    ));
    assert_eq!(stat_a.result.unwrap()["vault_ref"], "alpha");
    assert_eq!(stat_b.result.unwrap()["vault_ref"], "beta");
    assert_ne!(vault_id_for("alpha"), vault_id_for("beta"));
    assert_eq!(
        fs::read(root.join("alpha.calyx").join(identity::SALT_FILE_NAME))
            .unwrap()
            .len(),
        32
    );
    assert!(root.join("alpha.calyx").join("cf").join("base").exists());
    assert!(root.join("alpha.calyx").join("wal").exists());
    assert!(root.join("beta.calyx").join("cf").join("base").exists());
    assert!(root.join("beta.calyx").join("wal").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn engine_info_reports_registered_served_methods_without_vector_claims() {
    let mut engine = Engine::new(config(temp_root("engine-info")));
    let response = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"engine.info","params":{}}"#,
    ));
    let result = response.result.unwrap();
    let reported = result["served_methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|method| method["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    let expected = served_method_names().collect::<Vec<_>>();
    assert_eq!(reported, expected);
    assert_eq!(result["cpu_profile"]["hnsw"], false);
    assert_eq!(result["cpu_profile"]["vector_query"], false);
    assert_eq!(result["capabilities"]["hnsw-ram"], false);
    assert_eq!(result["capabilities"]["vector-query"], false);
    assert_eq!(result["security"]["value_encryption"], "aes-256-gcm");
    assert_eq!(result["security"]["zfs_probe"], "actual_vault_path");
}

#[test]
fn vault_clock_is_monotonic_across_requests() {
    let mut engine = Engine::new(config(temp_root("monotonic")));
    let create = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.create","params":{"vault_ref":"alpha","ts":200}}"#,
    ));
    assert!(create.error.is_none(), "{create:?}");
    let stat = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"vault.stat","params":{"vault_ref":"alpha","ts":100}}"#,
    ));
    assert_eq!(stat.result.unwrap()["last_ts"], 200);
}

#[test]
fn vault_stat_requires_prior_open() {
    let mut engine = Engine::new(config(temp_root("not-open")));
    let response = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.stat","params":{"vault_ref":"ghost","ts":1}}"#,
    ));
    let error = response.error.unwrap();
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        CALYX_LEAPABLE_VAULT_NOT_OPEN
    );
}

#[test]
fn invalid_vault_ref_fails_closed() {
    let mut engine = Engine::new(config(temp_root("bad-ref")));
    let response = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.create","params":{"vault_ref":"../escape","ts":1}}"#,
    ));
    let error = response.error.unwrap();
    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        crate::paths::CALYX_LEAPABLE_PATH_INVALID
    );
}

#[test]
fn missing_timestamp_is_invalid_params() {
    let mut engine = Engine::new(config(temp_root("missing-ts")));
    let response = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"alpha"}}"#,
    ));
    assert_eq!(response.error.unwrap().code, -32602);
}

#[test]
fn lifecycle_snapshot_restore_clone_verify_and_delete_use_real_bytes() {
    let root = temp_root("lifecycle");
    let mut engine = Engine::new(config(root.clone()));
    let create = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.create","params":{"vault_ref":"alpha","ts":1785500000}}"#,
    ));
    assert!(create.error.is_none(), "{create:?}");
    {
        let handle = engine.vaults.get("alpha").unwrap();
        handle
            .vault
            .put(sample_constellation(&handle.vault, 7))
            .unwrap();
        handle.vault.flush().unwrap();
    }

    let delete_open = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"vault.delete","params":{"vault_ref":"alpha","ts":1785500010}}"#,
    ));
    assert_eq!(
        delete_open.error.unwrap().data.unwrap()["calyx_code"],
        CALYX_LEAPABLE_VAULT_OPEN
    );

    let snapshot = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":3,"method":"vault.snapshot","params":{"vault_ref":"alpha","snapshot_ref":"snap1","ts":1785500020}}"#,
    ));
    let snapshot_result = snapshot.result.unwrap();
    assert_eq!(
        snapshot_result["verify_restore"]["success"], true,
        "{snapshot_result}"
    );
    assert!(root.join("_snapshots/alpha.calyx/snap1/cf/base").exists());

    let clone = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":4,"method":"vault.clone","params":{"vault_ref":"alpha","target_vault_ref":"beta","ts":1785500030}}"#,
    ));
    let clone_result = clone.result.unwrap();
    assert_eq!(clone_result["verify_restore"]["success"], true);
    assert!(root.join("beta.calyx").join("cf").join("base").exists());

    let restore_open = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":5,"method":"vault.restore","params":{"vault_ref":"alpha","snapshot_ref":"snap1","overwrite":true,"ts":1785500040}}"#,
    ));
    assert_eq!(
        restore_open.error.unwrap().data.unwrap()["calyx_code"],
        CALYX_LEAPABLE_VAULT_OPEN
    );

    let close = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":6,"method":"vault.close","params":{"vault_ref":"alpha","ts":1785500050}}"#,
    ));
    assert!(close.error.is_none());
    let delete = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":7,"method":"vault.delete","params":{"vault_ref":"alpha","ts":1785500060}}"#,
    ));
    assert!(delete.error.is_none());
    assert!(!root.join("alpha.calyx").exists());

    let restore = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":8,"method":"vault.restore","params":{"vault_ref":"alpha","snapshot_ref":"snap1","ts":1785500070}}"#,
    ));
    let restore_result = restore.result.unwrap();
    assert_eq!(restore_result["verify_restore"]["success"], true);

    let list = engine.dispatch(req(r#"{"jsonrpc":"2.0","id":9,"method":"vault.list"}"#));
    let list_result = list.result.unwrap();
    let refs = list_result["vaults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|vault| vault["vault_ref"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(refs, vec!["alpha", "beta"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_verify_none_skips_restore_verification() {
    let root = temp_root("verify-none");
    let mut engine = Engine::new(config(root.clone()));
    let create = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.create","params":{"vault_ref":"alpha","ts":1785500000}}"#,
    ));
    assert!(create.error.is_none(), "{create:?}");
    let snapshot = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"vault.snapshot","params":{"vault_ref":"alpha","snapshot_ref":"snap_none","ts":1785500010,"verify":"none"}}"#,
    ));
    let result = snapshot.result.unwrap();
    assert_eq!(result["verify"], "none");
    assert!(result["verify_restore"].is_null());
    assert!(result["source_verify_restore"].is_null());
    assert!(
        root.join("_snapshots/alpha.calyx/snap_none/cf/base")
            .exists()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_stranded_inverted_collection_opens_and_skips_index_maintenance() {
    let root = temp_root("legacy-stranded-index");
    let cfg = config(root.clone());
    let vault_ref = VaultRef::parse("legacy_inv").unwrap();
    let vault_id = vault_id_for(vault_ref.as_str());
    let dir = root.join(vault_ref.storage_dir_name());
    let (options, _) = encrypted_options(&cfg, vault_id, &dir);
    let vault = AsterVault::new_durable(
        &dir,
        vault_id,
        identity::salt_for(vault_ref.as_str()),
        options,
    )
    .unwrap();
    create_collection(&vault, legacy_inverted_collection()).unwrap();
    vault.flush().unwrap();
    drop(vault);

    let mut engine = Engine::new(cfg);
    let open = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"legacy_inv","ts":1785500200}}"#,
    ));
    assert!(open.error.is_none(), "{open:?}");
    let insert = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"rel.insert","params":{"vault_ref":"legacy_inv","ts":1785500210,"collection_name":"docs","pk":{"u64":1},"row":{"body":{"text":"alpha"}}}}"#,
    ));
    assert!(insert.error.is_none(), "{insert:?}");
    let scan = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":3,"method":"rel.scan","params":{"vault_ref":"legacy_inv","ts":1785500215,"collection_name":"docs","limit":10}}"#,
    ));
    let scan_result = scan.result.unwrap();
    assert_eq!(scan_result["items"].as_array().unwrap().len(), 1);
    let query = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":4,"method":"rel.query","params":{"vault_ref":"legacy_inv","ts":1785500216,"collection_name":"docs","index_name":"body_inv","gte":{"text":"a"}}}"#,
    ));
    assert_eq!(
        query.error.unwrap().data.unwrap()["calyx_code"],
        "CALYX_LEAPABLE_UNSERVED_CAPABILITY"
    );
    let delete = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":5,"method":"rel.delete","params":{"vault_ref":"legacy_inv","ts":1785500220,"collection_name":"docs","pk":{"u64":1}}}"#,
    ));
    assert!(delete.error.is_none(), "{delete:?}");
    let handle = engine.vaults.get("legacy_inv").unwrap();
    let inverted_rows = handle
        .vault
        .scan_cf_at(handle.vault.snapshot(), ColumnFamily::IndexInverted)
        .unwrap();
    assert!(inverted_rows.is_empty(), "{inverted_rows:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vault_open_compacts_legacy_duplicate_anchor_bloat_once() {
    let root = temp_root("legacy-anchor-bloat");
    let cfg = config(root.clone());
    let vault_ref = VaultRef::parse("legacy_anchor_bloat").unwrap();
    let vault_id = vault_id_for(vault_ref.as_str());
    let dir = root.join(vault_ref.storage_dir_name());
    let (options, _) = encrypted_options(&cfg, vault_id, &dir);
    let vault = AsterVault::new_durable(
        &dir,
        vault_id,
        identity::salt_for(vault_ref.as_str()),
        options,
    )
    .unwrap();
    let mut cx = sample_constellation(&vault, 77);
    cx.anchors.push(cx.anchors[0].clone());
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    drop(vault);

    let mut engine = Engine::new(cfg);
    let open = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"legacy_anchor_bloat","ts":1785500300}}"#,
    ));
    assert!(open.error.is_none(), "{open:?}");
    let handle = engine.vaults.get("legacy_anchor_bloat").unwrap();
    let stored = handle.vault.get(cx_id, handle.vault.snapshot()).unwrap();
    let marker = handle
        .vault
        .read_cf_at(
            handle.vault.snapshot(),
            ColumnFamily::Leapable,
            b"cx_anchor_compaction_v1",
        )
        .unwrap();

    assert_eq!(stored.anchors.len(), 1);
    assert!(marker.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vault_open_backfills_legacy_cx_tombstone_index() {
    let root = temp_root("legacy-tombstone-index");
    let cfg = config(root.clone());
    let vault_ref = VaultRef::parse("legacy").unwrap();
    let vault_id = vault_id_for(vault_ref.as_str());
    let dir = root.join(vault_ref.storage_dir_name());
    let (options, context) = encrypted_options(&cfg, vault_id, &dir);
    let vault = AsterVault::new_durable(
        &dir,
        vault_id,
        identity::salt_for(vault_ref.as_str()),
        options,
    )
    .unwrap();
    let cx = sample_constellation(&vault, 42);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    let context_guard = context
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    vault
        .erase_defer_key_shred(EraseScope::Cx(cx_id), &context_guard, &EraseRegistry::new())
        .unwrap();
    drop(context_guard);
    vault.flush().unwrap();
    context
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .shred_key_for_erasure();
    drop(vault);

    let mut engine = Engine::new(cfg);
    let open = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"legacy","ts":1785500100}}"#,
    ));
    assert!(open.error.is_none(), "{open:?}");
    let scan = engine.dispatch(req(
        r#"{"jsonrpc":"2.0","id":2,"method":"cx.scan","params":{"vault_ref":"legacy","ts":1785500110,"limit":10}}"#,
    ));
    let result = scan.result.unwrap();
    assert_eq!(result["tombstones_truncated"], false);
    let tombstones = result["tombstones"].as_array().unwrap();
    let cx_id_text = cx_id.to_string();
    assert!(
        tombstones
            .iter()
            .any(|row| { row["compact"]["c"].as_str() == Some(cx_id_text.as_str()) })
    );
    let handle = engine.vaults.get("legacy").unwrap();
    let indexed = handle
        .vault
        .scan_cf_at(handle.vault.snapshot(), ColumnFamily::Leapable)
        .unwrap();
    assert!(
        indexed.len() >= 2,
        "expected marker plus tombstone index row, got {indexed:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

fn legacy_inverted_collection() -> Collection {
    Collection {
        name: "docs".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![FieldDef::new(
            "body",
            FieldType::Text,
            false,
        )])),
        panel: None,
        indexes: vec![SecondaryIndexSpec {
            name: "body_inv".to_string(),
            kind: SecondaryIndexKind::Inverted,
            fields: vec!["body".to_string()],
        }],
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}
