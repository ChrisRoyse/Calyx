use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::manifest::is_vault_seq_quarantined;
use calyx_core::{CxId, Result as CalyxResult};
use calyx_ledger::{
    ActorId, AuditFilter, EntryKind, LedgerEntry, QuarantineLookup, SubjectId,
    audit as ledger_audit, get_answer_trace as ledger_get_answer_trace,
    get_provenance as ledger_get_provenance, tombstone_from_entry,
};
use serde_json::{Value, json};

use crate::error::CliResult;
use crate::ledger_store::AsterLedgerCfStore;
use crate::output::print_json;

pub fn get_provenance(vault: &Path, cx: &str) -> CliResult {
    let cx_id = CxId::from_str(cx).map_err(|error| format!("invalid --cx: {error}"))?;
    let store = AsterLedgerCfStore::open(vault)?;
    let quarantine = VaultQuarantine::new(vault);
    let entries = ledger_get_provenance(&store, &quarantine, cx_id)?;
    print_json(&json!(entries_json(&entries)))
}

pub fn get_answer_trace(vault: &Path, answer: &str) -> CliResult {
    let answer_id = parse_answer_id(answer)?;
    let store = AsterLedgerCfStore::open(vault)?;
    let quarantine = VaultQuarantine::new(vault);
    let trace = ledger_get_answer_trace(&store, &quarantine, &answer_id)?;
    print_json(&serde_json::to_value(trace)?)
}

pub fn audit(vault: &Path, kind: &str) -> CliResult {
    let kind = parse_kind(kind)?;
    let store = AsterLedgerCfStore::open(vault)?;
    let quarantine = VaultQuarantine::new(vault);
    let entries = ledger_audit(
        &store,
        &quarantine,
        AuditFilter {
            kind: Some(kind),
            ..AuditFilter::default()
        },
    )?;
    print_json(&json!(entries_json(&entries)))
}

struct VaultQuarantine {
    vault: PathBuf,
}

impl VaultQuarantine {
    fn new(vault: &Path) -> Self {
        Self {
            vault: vault.to_path_buf(),
        }
    }
}

impl QuarantineLookup for VaultQuarantine {
    fn contains_quarantined(&self, range: Range<u64>) -> CalyxResult<bool> {
        for seq in range {
            if is_vault_seq_quarantined(&self.vault, seq)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn entries_json(entries: &[LedgerEntry]) -> Vec<Value> {
    entries.iter().map(entry_json).collect()
}

fn entry_json(entry: &LedgerEntry) -> Value {
    json!({
        "seq": entry.seq,
        "kind": entry.kind.as_str(),
        "subject": subject_json(&entry.subject),
        "actor": actor_json(&entry.actor),
        "ts": entry.ts,
        "entry_hash": hex(&entry.entry_hash),
        "payload_hex": hex(&entry.payload),
        "payload_json": payload_json(entry),
    })
}

fn payload_json(entry: &LedgerEntry) -> Value {
    match tombstone_from_entry(entry) {
        Ok(Some(tombstone)) => tombstone.as_json_value(),
        Ok(None) | Err(_) => serde_json::from_slice::<Value>(&entry.payload).unwrap_or(Value::Null),
    }
}

fn subject_json(subject: &SubjectId) -> Value {
    match subject {
        SubjectId::Cx(id) => json!({"type": "cx", "id": id.to_string()}),
        SubjectId::Lens(id) => json!({"type": "lens", "id": id.to_string()}),
        SubjectId::Kernel(id) => json!({"type": "kernel", "id": hex(id)}),
        SubjectId::Guard(id) => json!({"type": "guard", "id": hex(id)}),
        SubjectId::Query(id) => json!({"type": "query", "id": hex(id)}),
    }
}

fn actor_json(actor: &ActorId) -> Value {
    match actor {
        ActorId::Agent(id) => json!({"type": "agent", "id": id}),
        ActorId::Service(id) => json!({"type": "service", "id": id}),
        ActorId::System => json!({"type": "system"}),
    }
}

fn parse_kind(value: &str) -> Result<EntryKind, String> {
    match value.to_ascii_lowercase().as_str() {
        "ingest" => Ok(EntryKind::Ingest),
        "measure" => Ok(EntryKind::Measure),
        "assay" => Ok(EntryKind::Assay),
        "kernel" => Ok(EntryKind::Kernel),
        "guard" => Ok(EntryKind::Guard),
        "answer" => Ok(EntryKind::Answer),
        "anneal" => Ok(EntryKind::Anneal),
        "migrate" => Ok(EntryKind::Migrate),
        "admin" => Ok(EntryKind::Admin),
        "erase" => Ok(EntryKind::Erase),
        _ => Err(format!("invalid --kind: {value}")),
    }
}

fn parse_answer_id(value: &str) -> Result<Vec<u8>, String> {
    if value.len().is_multiple_of(2) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        decode_hex(value)
    } else {
        Ok(value.as_bytes().to_vec())
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_value(chunk[0])?;
            let low = hex_value(chunk[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hex answer id".to_string()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
