use std::net::SocketAddr;

use calyx_aster::ledger_view::read_ledger_seq;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode};
use serde_json::{Value, json};

use super::ReproduceOut;
use crate::cf_read::hex_bytes;
use crate::cmd::search::{rederive_kernel_answer_hash, rederive_kernel_citation_answer_hash};
use crate::cmd::vault::{ResolvedVault, vault_salt};
use crate::error::{CliError, CliResult};

pub(super) fn record(
    resolved: &ResolvedVault,
    answer_id: &[u8],
    answer_payload: &Value,
    resident_override: Option<SocketAddr>,
) -> CliResult<ReproduceOut> {
    let original_hash = answer_payload
        .get("derivation_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::runtime("kernel Answer payload has no derivation_hash"))?
        .to_ascii_lowercase();
    if original_hash.len() != 64 || !original_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::runtime(
            "kernel Answer derivation_hash is not a 32-byte hexadecimal digest",
        ));
    }
    let reproduced = if answer_payload.get("type").and_then(Value::as_str)
        == Some("kernel_citation_answer_v1")
    {
        rederive_kernel_citation_answer_hash(
            resolved,
            answer_id,
            answer_payload,
            resident_override,
        )?
    } else {
        rederive_kernel_answer_hash(resolved, answer_id, answer_payload, resident_override)?
    };
    let reproduced_hash = hex_bytes(&reproduced);
    let bit_parity = original_hash == reproduced_hash;
    let payload = serde_json::to_vec(&json!({
        "type": "kernel_answer_reproduce_v1",
        "answer_id": hex_bytes(answer_id),
        "bit_parity": bit_parity,
        "original_hash": original_hash,
        "reproduced_hash": reproduced_hash,
    }))
    .map_err(|error| CliError::runtime(format!("encode kernel reproduce payload: {error}")))?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    )?;
    let reference = vault.append_ledger_entry(
        EntryKind::Admin,
        SubjectId::Query(answer_id.to_vec()),
        payload.clone(),
        ActorId::Service("calyx-kernel-reproduce".to_string()),
    )?;
    let row = read_ledger_seq(&resolved.path, reference.seq)?
        .ok_or_else(|| CliError::runtime("kernel reproduce ledger row is physically absent"))?;
    let entry = decode(&row.bytes)?;
    if entry.entry_hash != reference.hash
        || entry.kind != EntryKind::Admin
        || entry.payload != payload
        || !matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
    {
        return Err(CliError::runtime(
            "kernel reproduce ledger physical readback differs from the appended proof",
        ));
    }
    Ok(ReproduceOut {
        bit_parity,
        original_hash,
        reproduced_hash,
    })
}
