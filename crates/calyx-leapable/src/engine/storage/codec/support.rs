use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, SecondaryIndexKind, SecondaryIndexSpec, decode_collection,
};
use calyx_core::{CalyxError, VaultStore};

use super::storage_error;
use crate::engine::storage::{CALYX_LEAPABLE_UNSERVED_CAPABILITY, EngineResult, VaultHandle};

pub(in crate::engine::storage) fn warn_stranded_indexes(handle: &VaultHandle) -> EngineResult<()> {
    let snapshot = handle.vault.snapshot();
    for (_, bytes) in handle
        .vault
        .scan_cf_at(snapshot, ColumnFamily::Collections)?
    {
        let collection = decode_collection(&bytes)?;
        for index in collection
            .indexes
            .iter()
            .filter(|index| !is_served_index(&collection, index))
        {
            eprintln!(
                "calyx-leapable: CALYX_LEAPABLE_STRANDED_INDEX: vault_ref={} collection={} index={} kind={:?} has no served query surface",
                handle.vault_ref.as_str(),
                collection.name,
                index.name,
                index.kind
            );
        }
    }
    Ok(())
}

pub(in crate::engine::storage) fn served_write_collection(col: &Collection) -> Collection {
    let mut served = col.clone();
    served.indexes.retain(|index| is_served_index(col, index));
    served
}

fn is_served_index(col: &Collection, index: &SecondaryIndexSpec) -> bool {
    col.mode == CollectionMode::Records
        && index.kind == SecondaryIndexKind::Btree
        && index.fields.len() == 1
}

pub(in crate::engine::storage) fn unserved_index_kind(kind: &str, name: &str) -> CalyxError {
    unserved_capability_error(
        format!(
            "Leapable cannot serve {kind} index {name:?}; only single-field btree indexes are queryable"
        ),
        "declare kind `btree` with one field, or omit the index until Leapable exposes that query surface",
    )
}

pub(in crate::engine::storage) fn unserved_capability_error(
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    storage_error(CALYX_LEAPABLE_UNSERVED_CAPABILITY, message, remediation)
}
