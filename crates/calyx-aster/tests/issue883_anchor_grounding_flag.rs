use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(label: &str) -> PathBuf {
    let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{label}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

#[test]
fn cli_ledger_anchor_clears_ungrounded_flag_in_base_row() {
    let dir = test_dir("issue883-anchor-flag");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable");
    let input = b"issue883 ungrounded source";
    let id = vault.cx_id_for_input(input, 1);
    let mut hash = [0_u8; 32];
    hash[..input.len()].copy_from_slice(input);
    let cx = calyx_core::Constellation {
        cx_id: id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 123,
        input_ref: InputRef {
            hash,
            pointer: Some("test://issue883-anchor-flag".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    };
    let anchor = Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(1.0),
        source: "issue883-test".to_string(),
        observed_at: 124,
        confidence: 1.0,
    };

    vault.put(cx).expect("put");
    vault
        .anchor_with_ledger_entry(
            id,
            anchor.clone(),
            EntryKind::Ingest,
            SubjectId::Cx(id),
            b"anchor".to_vec(),
            ActorId::Service("calyx-cli".to_string()),
        )
        .expect("anchor with ledger");
    let got = vault.get(id, vault.snapshot()).expect("get anchored");

    assert_eq!(got.anchors.as_slice(), std::slice::from_ref(&anchor));
    assert!(!got.flags.ungrounded);
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
