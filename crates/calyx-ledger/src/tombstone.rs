//! Erasure tombstones for the append-only Ledger.

use core::str::FromStr;

use calyx_core::{CalyxError, Clock, CxId, LedgerRef, LensId, Result, Ts, VaultId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::append::{LedgerAppender, LedgerCfStore};
use crate::codec::decode;
use crate::entry::{ActorId, LedgerEntry, SubjectId};
use crate::kind::EntryKind;

mod wire;

const SCOPE_VAULT: &str = "v";
const SCOPE_CX: &str = "c";
const SCOPE_SUBJECT_CX: &str = "sc";
const SCOPE_SUBJECT_LENS: &str = "sl";
const SCOPE_SUBJECT_KERNEL: &str = "sk";
const SCOPE_SUBJECT_GUARD: &str = "sg";
const SCOPE_SUBJECT_QUERY: &str = "sq";
const ACTOR_AGENT: &str = "A:";
const ACTOR_SERVICE: &str = "S:";
const ACTOR_SYSTEM: &str = "Y";
const DIGEST_DOMAIN: &[u8] = b"calyx-ledger-erasure-tombstone-subject-v1";

/// Ledger-level erase scope. It deliberately carries only identifiers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErasureScope {
    Vault,
    Cx(CxId),
    Subject(SubjectId),
}

/// Metadata-only erasure tombstone payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureTombstone {
    pub seq: u64,
    pub vault_id: VaultId,
    pub scope: ErasureScope,
    pub actor: ActorId,
    pub erased_at: Ts,
    pub records_deleted: usize,
}

#[derive(Deserialize, Serialize)]
struct CompactTombstone {
    q: u64,
    v: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    c: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sq: Option<String>,
    a: String,
    t: Ts,
    n: usize,
}

impl ErasureTombstone {
    /// Canonical compact binary payload for the Ledger entry.
    pub fn as_ledger_payload(&self) -> Vec<u8> {
        wire::encode(self)
    }

    pub fn from_ledger_payload(payload: &[u8]) -> Result<Self> {
        if wire::is_wire_payload(payload) {
            return wire::decode(payload);
        }
        let compact = serde_json::from_slice::<CompactTombstone>(payload).map_err(|error| {
            CalyxError::ledger_corrupt(format!("decode erasure tombstone: {error}"))
        })?;
        compact.try_into()
    }

    /// Human-readable projection used by CLI audit/readback surfaces.
    pub fn as_json_value(&self) -> Value {
        serde_json::to_value(CompactTombstone::from(self))
            .expect("erasure tombstone JSON projection serializes")
    }

    pub fn ledger_subject(&self) -> SubjectId {
        match &self.scope {
            ErasureScope::Cx(id) => SubjectId::Cx(*id),
            ErasureScope::Subject(subject) => subject.clone(),
            ErasureScope::Vault => SubjectId::Guard(scope_digest(self.vault_id, &self.scope)),
        }
    }

    pub fn matches_scope(&self, vault_id: VaultId, scope: &ErasureScope) -> bool {
        self.vault_id == vault_id && &self.scope == scope
    }
}

impl From<&ErasureTombstone> for CompactTombstone {
    fn from(tombstone: &ErasureTombstone) -> Self {
        let mut compact = Self {
            q: tombstone.seq,
            v: tombstone.vault_id.to_string(),
            c: None,
            sc: None,
            sl: None,
            sk: None,
            sg: None,
            sq: None,
            a: compact_actor(&tombstone.actor),
            t: tombstone.erased_at,
            n: tombstone.records_deleted,
        };
        match &tombstone.scope {
            ErasureScope::Vault => {}
            ErasureScope::Cx(id) => compact.c = Some(id.to_string()),
            ErasureScope::Subject(SubjectId::Cx(id)) => compact.sc = Some(id.to_string()),
            ErasureScope::Subject(SubjectId::Lens(id)) => compact.sl = Some(id.to_string()),
            ErasureScope::Subject(SubjectId::Kernel(bytes)) => compact.sk = Some(hex(bytes)),
            ErasureScope::Subject(SubjectId::Guard(bytes)) => compact.sg = Some(hex(bytes)),
            ErasureScope::Subject(SubjectId::Query(bytes)) => compact.sq = Some(hex(bytes)),
        }
        compact
    }
}

impl TryFrom<CompactTombstone> for ErasureTombstone {
    type Error = CalyxError;

    fn try_from(value: CompactTombstone) -> Result<Self> {
        Ok(Self {
            seq: value.q,
            vault_id: VaultId::from_str(&value.v)
                .map_err(|error| CalyxError::ledger_corrupt(format!("vault id: {error}")))?,
            scope: parse_compact_scope(&value)?,
            actor: parse_actor(&value.a)?,
            erased_at: value.t,
            records_deleted: value.n,
        })
    }
}

pub fn write_tombstone<S, C>(
    tombstone: &ErasureTombstone,
    ledger: &mut LedgerAppender<S, C>,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    if ledger.next_seq() != tombstone.seq {
        return Err(CalyxError::ledger_chain_broken(format!(
            "erasure tombstone seq {} does not match ledger next_seq {}",
            tombstone.seq,
            ledger.next_seq()
        )));
    }
    ledger.append(
        EntryKind::Erase,
        tombstone.ledger_subject(),
        tombstone.as_ledger_payload(),
        tombstone.actor.clone(),
    )
}

pub fn is_tombstoned(
    vault_id: VaultId,
    scope: &ErasureScope,
    ledger: &dyn LedgerCfStore,
) -> Result<bool> {
    Ok(find_tombstone(vault_id, scope, ledger)?.is_some())
}

pub fn find_tombstone(
    vault_id: VaultId,
    scope: &ErasureScope,
    ledger: &dyn LedgerCfStore,
) -> Result<Option<ErasureTombstone>> {
    for row in ledger.scan()? {
        let entry = decode(&row.bytes)?;
        if let Some(tombstone) = tombstone_from_entry(&entry)?
            && tombstone.matches_scope(vault_id, scope)
        {
            return Ok(Some(tombstone));
        }
    }
    Ok(None)
}

pub fn tombstone_from_entry(entry: &LedgerEntry) -> Result<Option<ErasureTombstone>> {
    if entry.kind != EntryKind::Erase {
        return Ok(None);
    }
    let tombstone = ErasureTombstone::from_ledger_payload(&entry.payload)?;
    if tombstone.seq != entry.seq {
        return Err(CalyxError::ledger_corrupt(format!(
            "erasure tombstone payload seq {} != ledger seq {}",
            tombstone.seq, entry.seq
        )));
    }
    if tombstone.actor != entry.actor {
        return Err(CalyxError::ledger_corrupt(
            "erasure tombstone actor does not match ledger actor",
        ));
    }
    Ok(Some(tombstone))
}

fn compact_scope(scope: &ErasureScope) -> (&'static str, Option<String>) {
    match scope {
        ErasureScope::Vault => (SCOPE_VAULT, None),
        ErasureScope::Cx(id) => (SCOPE_CX, Some(id.to_string())),
        ErasureScope::Subject(subject) => compact_subject(subject),
    }
}

fn compact_subject(subject: &SubjectId) -> (&'static str, Option<String>) {
    match subject {
        SubjectId::Cx(id) => (SCOPE_SUBJECT_CX, Some(id.to_string())),
        SubjectId::Lens(id) => (SCOPE_SUBJECT_LENS, Some(id.to_string())),
        SubjectId::Kernel(bytes) => (SCOPE_SUBJECT_KERNEL, Some(hex(bytes))),
        SubjectId::Guard(bytes) => (SCOPE_SUBJECT_GUARD, Some(hex(bytes))),
        SubjectId::Query(bytes) => (SCOPE_SUBJECT_QUERY, Some(hex(bytes))),
    }
}

fn parse_compact_scope(value: &CompactTombstone) -> Result<ErasureScope> {
    let mut scope = None;
    set_scope(
        &mut scope,
        value
            .c
            .as_deref()
            .map(parse_cx)
            .transpose()?
            .map(ErasureScope::Cx),
    )?;
    set_scope(
        &mut scope,
        value
            .sc
            .as_deref()
            .map(parse_cx)
            .transpose()?
            .map(|id| ErasureScope::Subject(SubjectId::Cx(id))),
    )?;
    set_scope(
        &mut scope,
        value
            .sl
            .as_deref()
            .map(parse_lens)
            .transpose()?
            .map(|id| ErasureScope::Subject(SubjectId::Lens(id))),
    )?;
    set_scope(
        &mut scope,
        value
            .sk
            .as_deref()
            .map(parse_hex_id)
            .transpose()?
            .map(|id| ErasureScope::Subject(SubjectId::Kernel(id))),
    )?;
    set_scope(
        &mut scope,
        value
            .sg
            .as_deref()
            .map(parse_hex_id)
            .transpose()?
            .map(|id| ErasureScope::Subject(SubjectId::Guard(id))),
    )?;
    set_scope(
        &mut scope,
        value
            .sq
            .as_deref()
            .map(parse_hex_id)
            .transpose()?
            .map(|id| ErasureScope::Subject(SubjectId::Query(id))),
    )?;
    Ok(scope.unwrap_or(ErasureScope::Vault))
}

fn set_scope(scope: &mut Option<ErasureScope>, next: Option<ErasureScope>) -> Result<()> {
    let Some(next) = next else {
        return Ok(());
    };
    if scope.replace(next).is_some() {
        return Err(CalyxError::ledger_corrupt(
            "erasure tombstone has multiple scope fields",
        ));
    }
    Ok(())
}

fn parse_cx(value: &str) -> Result<CxId> {
    CxId::from_str(value).map_err(|error| CalyxError::ledger_corrupt(format!("cx id: {error}")))
}

fn parse_lens(value: &str) -> Result<LensId> {
    LensId::from_str(value).map_err(|error| CalyxError::ledger_corrupt(format!("lens id: {error}")))
}

fn compact_actor(actor: &ActorId) -> String {
    match actor {
        ActorId::Agent(id) => format!("{ACTOR_AGENT}{id}"),
        ActorId::Service(id) => format!("{ACTOR_SERVICE}{id}"),
        ActorId::System => ACTOR_SYSTEM.to_string(),
    }
}

fn parse_actor(value: &str) -> Result<ActorId> {
    if let Some(id) = value.strip_prefix(ACTOR_AGENT) {
        return Ok(ActorId::Agent(id.to_string()));
    }
    if let Some(id) = value.strip_prefix(ACTOR_SERVICE) {
        return Ok(ActorId::Service(id.to_string()));
    }
    if value == ACTOR_SYSTEM {
        return Ok(ActorId::System);
    }
    Err(CalyxError::ledger_corrupt(
        "unknown erasure tombstone actor",
    ))
}

fn scope_digest(vault_id: VaultId, scope: &ErasureScope) -> Vec<u8> {
    let (scope, id) = compact_scope(scope);
    let mut hasher = blake3::Hasher::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update(vault_id.to_string().as_bytes());
    hasher.update(scope.as_bytes());
    if let Some(id) = id {
        hasher.update(id.as_bytes());
    }
    hasher.finalize().as_bytes().to_vec()
}

fn parse_hex_id(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(CalyxError::ledger_corrupt("hex id has odd length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Ok((hex_value(chunk[0])? << 4) | hex_value(chunk[1])?))
        .collect()
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(CalyxError::ledger_corrupt("invalid hex id byte")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
