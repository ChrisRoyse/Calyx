//! Canonical ledger entry structure and hash framing.

use calyx_core::{CalyxError, CxId, LensId, Result};
use serde::{Deserialize, Serialize};

use crate::kind::EntryKind;

pub(crate) const HASH_BYTES: usize = 32;
pub(crate) const TAG_CX: u8 = 0;
pub(crate) const TAG_LENS: u8 = 1;
pub(crate) const TAG_KERNEL: u8 = 2;
pub(crate) const TAG_GUARD: u8 = 3;
pub(crate) const TAG_QUERY: u8 = 4;
pub(crate) const TAG_AGENT: u8 = 0;
pub(crate) const TAG_SERVICE: u8 = 1;
pub(crate) const TAG_SYSTEM: u8 = 2;
const MAX_ACTOR_ID_BYTES: usize = 64;

/// Tagged subject identifier for the ledger entry's primary object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubjectId {
    Cx(CxId),
    Lens(LensId),
    Kernel(Vec<u8>),
    Guard(Vec<u8>),
    Query(Vec<u8>),
}

impl SubjectId {
    pub(crate) fn wire_tag(&self) -> u8 {
        match self {
            Self::Cx(_) => TAG_CX,
            Self::Lens(_) => TAG_LENS,
            Self::Kernel(_) => TAG_KERNEL,
            Self::Guard(_) => TAG_GUARD,
            Self::Query(_) => TAG_QUERY,
        }
    }

    pub(crate) fn wire_bytes(&self) -> Vec<u8> {
        match self {
            Self::Cx(id) => id.as_bytes().to_vec(),
            Self::Lens(id) => id.as_bytes().to_vec(),
            Self::Kernel(bytes) | Self::Guard(bytes) | Self::Query(bytes) => bytes.clone(),
        }
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        tagged_slice(self.wire_tag(), &self.wire_bytes())
    }
}

/// Tagged actor identifier for the service or agent that caused an entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorId {
    Agent(String),
    Service(String),
    System,
}

impl ActorId {
    pub(crate) fn wire_tag(&self) -> u8 {
        match self {
            Self::Agent(_) => TAG_AGENT,
            Self::Service(_) => TAG_SERVICE,
            Self::System => TAG_SYSTEM,
        }
    }

    pub(crate) fn wire_bytes(&self) -> &[u8] {
        match self {
            Self::Agent(value) | Self::Service(value) => value.as_bytes(),
            Self::System => &[],
        }
    }

    pub fn validate(&self) -> Result<()> {
        let len = self.wire_bytes().len();
        if len <= MAX_ACTOR_ID_BYTES {
            Ok(())
        } else {
            Err(CalyxError::ledger_actor_too_long(format!(
                "actor id has {len} UTF-8 bytes, max {MAX_ACTOR_ID_BYTES}"
            )))
        }
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        tagged_var(self.wire_tag(), self.wire_bytes())
    }
}

/// Canonical append-only ledger entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub seq: u64,
    pub prev_hash: [u8; HASH_BYTES],
    pub kind: EntryKind,
    pub subject: SubjectId,
    pub payload: Vec<u8>,
    pub actor: ActorId,
    pub ts: u64,
    pub entry_hash: [u8; HASH_BYTES],
}

impl LedgerEntry {
    /// Builds an entry and computes its canonical BLAKE3 hash.
    pub fn new(
        seq: u64,
        prev_hash: [u8; HASH_BYTES],
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
        ts: u64,
    ) -> Self {
        let entry_hash = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &actor, ts);
        Self {
            seq,
            prev_hash,
            kind,
            subject,
            payload,
            actor,
            ts,
            entry_hash,
        }
    }

    /// Recomputes and compares the canonical entry hash.
    pub fn verify(&self) -> bool {
        self.entry_hash
            == compute_entry_hash(
                self.seq,
                &self.prev_hash,
                self.kind,
                &self.subject,
                &self.payload,
                &self.actor,
                self.ts,
            )
    }
}

/// Computes the canonical BLAKE3 ledger entry hash.
pub fn compute_entry_hash(
    seq: u64,
    prev_hash: &[u8; HASH_BYTES],
    kind: EntryKind,
    subject: &SubjectId,
    payload: &[u8],
    actor: &ActorId,
    ts: u64,
) -> [u8; HASH_BYTES] {
    let mut hasher = blake3::Hasher::new();
    frame(&mut hasher, &seq.to_be_bytes());
    frame(&mut hasher, prev_hash);
    frame(&mut hasher, &[kind.wire_code()]);
    frame(&mut hasher, &subject.canonical_bytes());
    frame(&mut hasher, payload);
    frame(&mut hasher, &actor.canonical_bytes());
    frame(&mut hasher, &ts.to_be_bytes());
    *hasher.finalize().as_bytes()
}

fn frame(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn tagged_slice(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + bytes.len());
    out.push(tag);
    out.extend_from_slice(bytes);
    out
}

fn tagged_var(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + bytes.len());
    out.push(tag);
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const GOLDEN_HASH: [u8; HASH_BYTES] = [
        0x21, 0xf5, 0xff, 0x34, 0xd0, 0x85, 0xba, 0x09, 0x4e, 0x9e, 0x83, 0x1a, 0x73, 0x4f, 0xc4,
        0xbb, 0xfd, 0x7d, 0x8e, 0xca, 0xab, 0x11, 0x38, 0xa8, 0x05, 0xa9, 0x6b, 0xc4, 0x6c, 0x17,
        0xae, 0x88,
    ];

    fn golden_entry() -> LedgerEntry {
        LedgerEntry::new(
            1,
            [0; HASH_BYTES],
            EntryKind::Ingest,
            SubjectId::Cx(CxId::from_bytes([1; 16])),
            b"test".to_vec(),
            ActorId::Service("svc".to_string()),
            1_785_000_000,
        )
    }

    fn kind_strategy() -> impl Strategy<Value = EntryKind> {
        prop_oneof![
            Just(EntryKind::Ingest),
            Just(EntryKind::Measure),
            Just(EntryKind::Assay),
            Just(EntryKind::Kernel),
            Just(EntryKind::Guard),
            Just(EntryKind::Answer),
            Just(EntryKind::Anneal),
            Just(EntryKind::Migrate),
            Just(EntryKind::Admin),
            Just(EntryKind::Erase),
        ]
    }

    fn subject_strategy() -> impl Strategy<Value = SubjectId> {
        prop_oneof![
            any::<[u8; 16]>().prop_map(|bytes| SubjectId::Cx(CxId::from_bytes(bytes))),
            any::<[u8; 16]>().prop_map(|bytes| SubjectId::Lens(LensId::from_bytes(bytes))),
            prop::collection::vec(any::<u8>(), 0..64).prop_map(SubjectId::Kernel),
            prop::collection::vec(any::<u8>(), 0..64).prop_map(SubjectId::Guard),
            prop::collection::vec(any::<u8>(), 0..64).prop_map(SubjectId::Query),
        ]
    }

    fn actor_strategy() -> impl Strategy<Value = ActorId> {
        prop_oneof![
            Just(ActorId::System),
            "[a-z0-9_-]{0,32}".prop_map(ActorId::Agent),
            "[a-z0-9_-]{0,32}".prop_map(ActorId::Service),
        ]
    }

    #[test]
    fn entry_hash_golden() {
        let entry = golden_entry();
        println!("ENTRY_HASH_GOLDEN {}", hex32(&entry.entry_hash));
        assert!(entry.verify());
        assert_eq!(entry.entry_hash, GOLDEN_HASH);
    }

    #[test]
    fn entry_hash_edges_verify() {
        let entries = [
            LedgerEntry::new(
                0,
                [0; HASH_BYTES],
                EntryKind::Admin,
                SubjectId::Query(vec![0; HASH_BYTES]),
                Vec::new(),
                ActorId::Service("svc".to_string()),
                0,
            ),
            LedgerEntry::new(
                u64::MAX,
                [9; HASH_BYTES],
                EntryKind::Erase,
                SubjectId::Guard(vec![7; HASH_BYTES]),
                "snowman-utf8".as_bytes().to_vec(),
                ActorId::Agent("agent_max".to_string()),
                u64::MAX,
            ),
            LedgerEntry::new(
                42,
                [3; HASH_BYTES],
                EntryKind::Kernel,
                SubjectId::Kernel(vec![4; HASH_BYTES]),
                vec![0, 255, 10, 13],
                ActorId::Service(String::new()),
                1,
            ),
        ];
        for entry in entries {
            assert!(entry.verify());
        }
    }

    #[test]
    fn corrupted_entry_hash_fails_verify() {
        let mut entry = golden_entry();
        entry.entry_hash[0] ^= 0xff;
        assert!(!entry.verify());
    }

    proptest! {
        #[test]
        fn entry_hash_is_deterministic(
            seq in any::<u64>(),
            prev_hash in any::<[u8; HASH_BYTES]>(),
            kind in kind_strategy(),
            subject in subject_strategy(),
            payload in prop::collection::vec(any::<u8>(), 0..128),
            actor in actor_strategy(),
            ts in any::<u64>(),
        ) {
            let first = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &actor, ts);
            let second = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &actor, ts);
            prop_assert_eq!(first, second);
        }

        #[test]
        fn changing_each_field_changes_hash(
            seq in any::<u64>(),
            prev_hash in any::<[u8; HASH_BYTES]>(),
            payload in prop::collection::vec(any::<u8>(), 0..128),
            ts in any::<u64>(),
        ) {
            let kind = EntryKind::Ingest;
            let subject = SubjectId::Cx(CxId::from_bytes([1; 16]));
            let actor = ActorId::Service("svc".to_string());
            let base = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &actor, ts);

            let changed_seq = compute_entry_hash(seq.wrapping_add(1), &prev_hash, kind, &subject, &payload, &actor, ts);
            let mut changed_prev = prev_hash;
            changed_prev[0] ^= 1;
            let changed_prev_hash = compute_entry_hash(seq, &changed_prev, kind, &subject, &payload, &actor, ts);
            let changed_kind = compute_entry_hash(seq, &prev_hash, EntryKind::Measure, &subject, &payload, &actor, ts);
            let changed_subject = compute_entry_hash(seq, &prev_hash, kind, &SubjectId::Cx(CxId::from_bytes([2; 16])), &payload, &actor, ts);
            let mut changed_payload = payload.clone();
            changed_payload.push(0xff);
            let changed_payload_hash = compute_entry_hash(seq, &prev_hash, kind, &subject, &changed_payload, &actor, ts);
            let changed_actor = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &ActorId::Service("svc2".to_string()), ts);
            let changed_ts = compute_entry_hash(seq, &prev_hash, kind, &subject, &payload, &actor, ts.wrapping_add(1));

            prop_assert_ne!(base, changed_seq);
            prop_assert_ne!(base, changed_prev_hash);
            prop_assert_ne!(base, changed_kind);
            prop_assert_ne!(base, changed_subject);
            prop_assert_ne!(base, changed_payload_hash);
            prop_assert_ne!(base, changed_actor);
            prop_assert_ne!(base, changed_ts);
        }
    }

    fn hex32(bytes: &[u8; HASH_BYTES]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
