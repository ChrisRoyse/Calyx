use std::time::Duration;

use calyx_core::{FixedClock, VaultId};

use super::{CALYX_TXN_SERIALIZABLE_CONFLICT, IsolationLevel, TxnHandle};
use crate::cf::ColumnFamily;
use crate::collection::{
    Collection, CollectionMode, DedupPolicy, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy,
};
use crate::layers::blob::{self, BlobId};
use crate::vault::AsterVault;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn blob_collection() -> Collection {
    Collection {
        name: "blob_txn".to_string(),
        mode: CollectionMode::Blob,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

#[test]
fn serializable_commit_rejects_intervening_vault_write() {
    let vault = AsterVault::with_clock(vault_id(), b"txn-conflict", FixedClock::new(10));
    let col = blob_collection();
    let handle = TxnHandle::new(vault.vault_id());
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.blob_put_chunk(&vault, &col, BlobId::from_text("txn"), 0, b"staged")
        .unwrap();

    vault
        .write_cf(
            ColumnFamily::Online,
            b"outside".to_vec(),
            b"advance".to_vec(),
        )
        .unwrap();

    let error = txn.commit(&vault).unwrap_err();
    assert_eq!(error.code, CALYX_TXN_SERIALIZABLE_CONFLICT);
}

#[test]
fn read_committed_reads_own_staged_write() {
    let vault = AsterVault::with_clock(vault_id(), b"txn-rc-own", FixedClock::new(20));
    let col = blob_collection();
    let blob = BlobId::from_text("read-own-write");
    let key = blob::chunk_key(&col, blob, 0);
    let handle = TxnHandle::new(vault.vault_id());
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::ReadCommitted,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();

    txn.blob_put_chunk(&vault, &col, blob, 0, b"visible")
        .unwrap();

    assert_eq!(
        txn.read_cf(&vault, ColumnFamily::Blob, &key).unwrap(),
        Some(b"visible".to_vec())
    );
}
