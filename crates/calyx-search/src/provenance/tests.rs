use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{CfRouter, ColumnFamily, base_key, ledger_key};
use calyx_aster::sst::write_sst;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotKey, SlotShape, SlotState, VaultId,
    VaultStore,
};
use calyx_registry::measure::measure_constellation;
use calyx_registry::spec::default_recall_delta;
use calyx_registry::{
    AlgorithmicLens, LensRuntime, LensSpec, Registry, VaultPanelState, load_vault_panel_state,
    persist_vault_panel_state,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::*;
use crate::engine::{
    FusionChoice, GuardChoice, SearchBudget, SearchFreshness, measure_query_vectors,
    search_outcome, search_outcome_with_freshness, search_outcome_with_query_vectors_freshness,
    search_outcome_with_slots_traced,
};
use crate::persisted::{PersistedSearchIndexes, rebuild_for_vault};

mod cases;
mod edge_cases;

struct Fixture {
    root: PathBuf,
    vault_dir: PathBuf,
    vault_id: VaultId,
    cx_id: calyx_core::CxId,
    all_cx_ids: Vec<calyx_core::CxId>,
    ledger_ref: calyx_core::LedgerRef,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::new_with_inputs(name, &[b"alpha" as &[u8]])
    }

    fn new_with_inputs(name: &str, inputs: &[&[u8]]) -> Self {
        assert!(
            !inputs.is_empty(),
            "provenance search fixture needs at least one input"
        );
        let root = temp_root(name);
        let vault_id = VaultId::from_ulid(Ulid::new());
        let vault_dir = root.join("vault");
        let mut registry = Registry::new();
        let lens = AlgorithmicLens::byte_features("issue918-byte", Modality::Text);
        let contract = lens.contract().clone();
        let lens_id = contract.lens_id();
        let spec = LensSpec {
            name: "issue918-byte".to_string(),
            runtime: LensRuntime::Algorithmic {
                kind: "byte-features".to_string(),
            },
            output: contract.shape(),
            modality: contract.modality(),
            weights_sha256: contract.weights_sha256(),
            corpus_hash: contract.corpus_hash(),
            norm_policy: contract.norm_policy(),
            max_batch: None,
            axis: Some("issue918-byte".to_string()),
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        };
        registry
            .register_frozen_with_spec(lens, contract, spec)
            .expect("register lens");
        let panel = panel(lens_id);
        let vault = AsterVault::new_durable(
            &vault_dir,
            vault_id,
            salt(),
            VaultOptions {
                panel: Some(panel.clone()),
                ..VaultOptions::default()
            },
        )
        .expect("open vault");
        persist_vault_panel_state(&vault_dir, &panel, &registry).expect("persist panel");
        let state = VaultPanelState {
            panel,
            registry,
            registry_snapshot: None,
        };
        let mut all_cx_ids = Vec::new();
        for input in inputs {
            let measured = measure_constellation(
                &vault,
                &state,
                Input::new(Modality::Text, input.to_vec()),
                1,
            )
            .expect("measure");
            let cx_id = measured.cx_id;
            vault.put(measured).expect("put constellation");
            all_cx_ids.push(cx_id);
        }
        vault.flush().expect("flush vault");
        rebuild_for_vault(&vault_dir, &vault).expect("rebuild search index");
        let cx_id = all_cx_ids[0];
        let stored = vault.get(cx_id, vault.snapshot()).expect("read stored");
        let ledger_ref = stored.provenance;
        drop(vault);
        Self {
            root,
            vault_dir,
            vault_id,
            cx_id,
            all_cx_ids,
            ledger_ref,
        }
    }

    fn open_vault(&self) -> AsterVault {
        AsterVault::open(
            &self.vault_dir,
            self.vault_id,
            salt(),
            VaultOptions::default(),
        )
        .expect("open vault")
    }

    fn search_error(&self, state: &VaultPanelState) -> crate::error::SearchError {
        let vault = self.open_vault();
        self.search_error_with_vault(&vault, state)
    }

    fn search_error_with_vault(
        &self,
        vault: &AsterVault,
        state: &VaultPanelState,
    ) -> crate::error::SearchError {
        match search_outcome(
            vault,
            state,
            &self.vault_dir,
            "alpha",
            1,
            FusionChoice::Rrf,
            GuardChoice::Off,
            None,
            false,
        ) {
            Ok(_) => panic!("search must fail closed"),
            Err(error) => error,
        }
    }

    fn index_candidates(&self, state: &VaultPanelState) -> Vec<String> {
        let (slot, query) = measure_query_vectors(state, "alpha")
            .expect("measure query")
            .into_iter()
            .next()
            .expect("query vector");
        PersistedSearchIndexes::open(&self.vault_dir)
            .expect("open index")
            .search(slot, &query, 1)
            .expect("search index")
            .into_iter()
            .map(|hit| hit.cx_id.to_string())
            .collect()
    }

    fn readback(&self) -> Value {
        json!({
            "base_exists": cf_row_exists(&self.vault_dir, ColumnFamily::Base, &base_key(self.cx_id)),
            "ledger_rows": ledger_rows(&self.vault_dir),
            "target": {
                "cx_id": self.cx_id.to_string(),
                "ledger_seq": self.ledger_ref.seq,
                "ledger_hash": hex32(&self.ledger_ref.hash),
            },
            "manifest": read_manifest(&self.vault_dir),
            "vault_manifest": read_json(&self.vault_dir.join("MANIFEST")),
        })
    }

    fn cleanup(self) {
        if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
            let _ = fs::remove_dir_all(self.root);
        }
    }
}

fn panel(lens_id: calyx_core::LensId) -> Panel {
    let slot = calyx_core::SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "issue918-byte"),
            lens_id,
            shape: SlotShape::Dense(16),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("issue918-byte".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: Default::default(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn remove_cf_row(vault: &Path, cf: ColumnFamily, key: &[u8]) {
    rewrite_cf_rows(vault, cf, |rows| rows.retain(|row| row.key != key));
}

fn corrupt_cf_row(vault: &Path, cf: ColumnFamily, key: &[u8]) {
    rewrite_cf_rows(vault, cf, |rows| {
        let row = rows.iter_mut().find(|row| row.key == key).expect("row");
        let last = row.value.len().checked_sub(1).expect("non-empty row");
        row.value[last] ^= 0x55;
    });
}

fn rewrite_cf_rows(
    vault: &Path,
    cf: ColumnFamily,
    mutate: impl FnOnce(&mut Vec<calyx_aster::sst::SstEntry>),
) {
    let router = CfRouter::open(vault, 0).expect("open CF router");
    let mut rows = router.iter_cf(cf).expect("read CF rows");
    mutate(&mut rows);
    let cf_dir = vault.join("cf").join(cf.name());
    for entry in fs::read_dir(&cf_dir).expect("read CF directory") {
        let path = entry.expect("read CF entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            fs::remove_file(path).expect("remove original SST");
        }
    }
    if !rows.is_empty() {
        write_sst(
            cf_dir.join("00000000000000000001.sst"),
            rows.iter()
                .map(|entry| (entry.key.as_slice(), entry.value.as_slice())),
        )
        .expect("write rewritten SST");
    }
    let wal_dir = vault.join("wal");
    if wal_dir.exists() {
        fs::remove_dir_all(wal_dir).expect("remove stale WAL");
    }
}

fn cf_row_exists(vault: &Path, cf: ColumnFamily, key: &[u8]) -> bool {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(cf))
        .map(|rows| rows.iter().any(|row| row.key == key))
        .unwrap_or(false)
}

fn ledger_rows(vault: &Path) -> Vec<Value> {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(ColumnFamily::Ledger))
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "seq": u64::from_be_bytes(row.key.as_slice().try_into().expect("ledger key")),
                "bytes_len": row.value.len(),
                "bytes_sha256": sha256_hex(&row.value),
            })
        })
        .collect()
}

fn decoded_ledger_entries(vault: &Path) -> Vec<Value> {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(ColumnFamily::Ledger))
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let entry = decode(&row.value).expect("decode ledger row");
            let payload: Value = serde_json::from_slice(&entry.payload).unwrap_or(Value::Null);
            json!({
                "seq": entry.seq,
                "kind": format!("{:?}", entry.kind),
                "subject": subject_json(&entry.subject),
                "payload": payload,
                "entry_hash": hex32(&entry.entry_hash),
            })
        })
        .collect()
}

fn subject_json(subject: &SubjectId) -> Value {
    match subject {
        SubjectId::Cx(id) => json!({"type": "cx", "id": id.to_string()}),
        SubjectId::Lens(id) => json!({"type": "lens", "id": id.to_string()}),
        SubjectId::Kernel(bytes) => json!({"type": "kernel", "bytes_sha256": sha256_hex(bytes)}),
        SubjectId::Guard(bytes) => json!({"type": "guard", "bytes_sha256": sha256_hex(bytes)}),
        SubjectId::Query(bytes) => json!({"type": "query", "bytes_sha256": sha256_hex(bytes)}),
    }
}

fn read_manifest(vault: &Path) -> Value {
    read_json(&vault.join("idx/search/manifest.json"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()))
}

fn error_json(error: &crate::error::SearchError) -> Value {
    json!({
        "code": error.code(),
        "message": error.message(),
    })
}

fn trace_event_json(event: &crate::engine::SearchTraceEvent) -> Value {
    json!({
        "phase": event.phase,
        "slot": event.slot.map(|slot| slot.get()),
        "elapsed_ms": event.elapsed_ms,
        "count": event.count,
        "detail": event.detail,
    })
}

fn maybe_write_fsv_json(name: &str, value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV");
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-search-issue918-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");
    root
}

fn salt() -> Vec<u8> {
    b"issue918-search".to_vec()
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
