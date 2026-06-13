use bincode::config;
use calyx_core::{CalyxError, CxId, LensId, Result, Ts, VaultId};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::entry::{ActorId, SubjectId};
use crate::tombstone::{ErasureScope, ErasureTombstone};

const PAYLOAD_MAGIC: &[u8; 4] = b"ETB1";

#[derive(Deserialize, Serialize)]
struct WireTombstone {
    q: u64,
    v: [u8; 16],
    s: WireScope,
    a: WireActor,
    t: Ts,
    n: u64,
}

#[derive(Deserialize, Serialize)]
enum WireScope {
    Vault,
    Cx([u8; 16]),
    SubjectCx([u8; 16]),
    SubjectLens([u8; 16]),
    SubjectKernel(Vec<u8>),
    SubjectGuard(Vec<u8>),
    SubjectQuery(Vec<u8>),
}

#[derive(Deserialize, Serialize)]
enum WireActor {
    Agent(String),
    Service(String),
    System,
}

pub(super) fn encode(tombstone: &ErasureTombstone) -> Vec<u8> {
    let wire = WireTombstone::from(tombstone);
    let mut payload = PAYLOAD_MAGIC.to_vec();
    payload.extend(
        bincode::serde::encode_to_vec(&wire, config::standard())
            .expect("erasure tombstone payload serializes"),
    );
    payload
}

pub(super) fn is_wire_payload(payload: &[u8]) -> bool {
    payload.starts_with(PAYLOAD_MAGIC)
}

pub(super) fn decode(payload: &[u8]) -> Result<ErasureTombstone> {
    let bytes = payload
        .strip_prefix(PAYLOAD_MAGIC)
        .ok_or_else(|| CalyxError::ledger_corrupt("erasure tombstone missing wire magic"))?;
    let (wire, consumed) =
        bincode::serde::decode_from_slice::<WireTombstone, _>(bytes, config::standard()).map_err(
            |error| CalyxError::ledger_corrupt(format!("decode erasure tombstone: {error}")),
        )?;
    if consumed != bytes.len() {
        return Err(CalyxError::ledger_corrupt(
            "erasure tombstone payload has trailing bytes",
        ));
    }
    wire.try_into()
}

impl From<&ErasureTombstone> for WireTombstone {
    fn from(tombstone: &ErasureTombstone) -> Self {
        Self {
            q: tombstone.seq,
            v: tombstone.vault_id.as_ulid().to_bytes(),
            s: WireScope::from(&tombstone.scope),
            a: WireActor::from(&tombstone.actor),
            t: tombstone.erased_at,
            n: tombstone
                .records_deleted
                .try_into()
                .expect("usize records_deleted fits u64"),
        }
    }
}

impl TryFrom<WireTombstone> for ErasureTombstone {
    type Error = CalyxError;

    fn try_from(value: WireTombstone) -> Result<Self> {
        Ok(Self {
            seq: value.q,
            vault_id: VaultId::from_ulid(Ulid::from_bytes(value.v)),
            scope: value.s.into(),
            actor: value.a.into(),
            erased_at: value.t,
            records_deleted: usize::try_from(value.n).map_err(|_| {
                CalyxError::ledger_corrupt("erasure tombstone records_deleted overflows usize")
            })?,
        })
    }
}

impl From<&ErasureScope> for WireScope {
    fn from(scope: &ErasureScope) -> Self {
        match scope {
            ErasureScope::Vault => Self::Vault,
            ErasureScope::Cx(id) => Self::Cx(id.to_bytes()),
            ErasureScope::Subject(SubjectId::Cx(id)) => Self::SubjectCx(id.to_bytes()),
            ErasureScope::Subject(SubjectId::Lens(id)) => Self::SubjectLens(id.to_bytes()),
            ErasureScope::Subject(SubjectId::Kernel(bytes)) => Self::SubjectKernel(bytes.clone()),
            ErasureScope::Subject(SubjectId::Guard(bytes)) => Self::SubjectGuard(bytes.clone()),
            ErasureScope::Subject(SubjectId::Query(bytes)) => Self::SubjectQuery(bytes.clone()),
        }
    }
}

impl From<WireScope> for ErasureScope {
    fn from(scope: WireScope) -> Self {
        match scope {
            WireScope::Vault => Self::Vault,
            WireScope::Cx(bytes) => Self::Cx(CxId::from_bytes(bytes)),
            WireScope::SubjectCx(bytes) => Self::Subject(SubjectId::Cx(CxId::from_bytes(bytes))),
            WireScope::SubjectLens(bytes) => {
                Self::Subject(SubjectId::Lens(LensId::from_bytes(bytes)))
            }
            WireScope::SubjectKernel(bytes) => Self::Subject(SubjectId::Kernel(bytes)),
            WireScope::SubjectGuard(bytes) => Self::Subject(SubjectId::Guard(bytes)),
            WireScope::SubjectQuery(bytes) => Self::Subject(SubjectId::Query(bytes)),
        }
    }
}

impl From<&ActorId> for WireActor {
    fn from(actor: &ActorId) -> Self {
        match actor {
            ActorId::Agent(id) => Self::Agent(id.clone()),
            ActorId::Service(id) => Self::Service(id.clone()),
            ActorId::System => Self::System,
        }
    }
}

impl From<WireActor> for ActorId {
    fn from(actor: WireActor) -> Self {
        match actor {
            WireActor::Agent(id) => Self::Agent(id),
            WireActor::Service(id) => Self::Service(id),
            WireActor::System => Self::System,
        }
    }
}
